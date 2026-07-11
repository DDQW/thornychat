//! The sync worker: a single long-lived tokio task that owns the
//! `matrix_sdk::Client` and a `matrix_sdk_ui::sync_service::SyncService`,
//! bridging them to the UI over two `mpsc` channels.
//!
//! This is the architectural linchpin described in the project plan: iced's
//! `Task<Message>` is one-shot and the wrong shape for a persistent service,
//! so the worker lives entirely outside iced's executor and communicates
//! purely through `ClientCommand` (in) / `ClientEvent` (out).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use matrix_sdk::encryption::verification::VerificationRequest;
use matrix_sdk::notification_settings::{
    IsEncrypted, IsOneToOne, RoomNotificationMode as SdkNotificationMode,
};
use matrix_sdk::room::edit::EditedContent;
use matrix_sdk::ruma::api::client::receipt::create_receipt::v3::ReceiptType;
use matrix_sdk::ruma::events::key::verification::request::ToDeviceKeyVerificationRequestEventContent;
use matrix_sdk::ruma::events::room::message::{
    MessageType, OriginalSyncRoomMessageEvent, RoomMessageEventContent,
    RoomMessageEventContentWithoutRelation,
};
use matrix_sdk::ruma::events::room::ImageInfo;
use matrix_sdk::ruma::events::sticker::StickerEventContent;
use matrix_sdk::ruma::events::{AnyMessageLikeEventContent, Mentions, ToDeviceEvent};
use matrix_sdk::ruma::{
    EventId, OwnedMxcUri, OwnedRoomId, OwnedServerName, OwnedUserId, RoomId, RoomOrAliasId,
    ServerName, UInt, UserId,
};
use matrix_sdk::send_queue::SendQueueRoomError;
use matrix_sdk::Client;
use matrix_sdk_ui::sync_service::{self, SyncService};
use matrix_sdk_ui::timeline::{AttachmentSource, Timeline, TimelineEventItemId};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

/// How long the error-triggered send-queue recovery (see `spawn_send_queue_recovery`)
/// waits before re-enabling a room whose queue just disabled itself. The SDK
/// already retries a request a few times with its own backoff before giving
/// up and disabling the queue, so this only needs to guard against hammering
/// a link that's still down — not implement backoff itself. The
/// reconnect-triggered recovery (the `state_stream` arm below) covers the
/// "whole connection dropped" case with the sync service's own backoff for
/// free; this constant only matters for the narrower "one request timed out
/// while sync stayed healthy" case.
const SEND_QUEUE_RETRY_DELAY: Duration = Duration::from_secs(5);

use crate::commands::{ClientCommand, RequestId};
use crate::events::{ClientEvent, SyncState, UserSearchResult};
use crate::rooms::{room_list, timeline};
use crate::verification::{self, VerificationAction, VerificationSession};

/// Everything kept alive for a room the UI currently has open: the shared
/// `Timeline` instance (used both to forward updates and to send/edit/
/// redact/attach against) plus the two background tasks tied to its
/// lifetime.
struct RoomHandles {
    timeline: Arc<Timeline>,
    timeline_task: JoinHandle<()>,
    typing_task: JoinHandle<()>,
}

impl Drop for RoomHandles {
    fn drop(&mut self) {
        self.timeline_task.abort();
        self.typing_task.abort();
    }
}

/// State the command loop needs across iterations, beyond the plain
/// request/response of a single command.
struct WorkerState {
    open_rooms: HashMap<String, RoomHandles>,
    /// Room ids kept subscribed for the whole session no matter what is open:
    /// joined spaces (which sliding sync's list filter keeps out of the store
    /// unless subscribed) and rooms joined this session. Unioned with the open
    /// rooms on every [`resubscribe`], which explains why the whole set has to
    /// be re-sent each time.
    pinned_subscriptions: HashSet<OwnedRoomId>,
    /// The single active SAS verification flow, if any (self-verification
    /// or verifying another user/incoming request) — see the project plan
    /// for why only one flow at a time is supported.
    verification: Option<VerificationSession>,
    /// UIAA session id awaiting completion via the fallback web page,
    /// captured from the original `bootstrap_cross_signing` failure.
    pending_cross_signing_session: Option<String>,
}

/// Spawns the sync worker and returns the command sender the UI layer holds
/// onto, plus the join handle for lifecycle/shutdown management from `app`.
pub fn spawn(
    client: Client,
    event_tx: mpsc::UnboundedSender<ClientEvent>,
    media_cache_dir: PathBuf,
) -> (mpsc::UnboundedSender<ClientCommand>, JoinHandle<()>) {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();

    let handle = tokio::spawn(async move {
        if let Err(err) = run(client, cmd_rx, event_tx.clone(), media_cache_dir).await {
            tracing::error!(?err, "sync worker exited with error");
            let _ = event_tx.send(ClientEvent::SyncStateChanged(SyncState::Error(
                err.to_string(),
            )));
        }
    });

    (cmd_tx, handle)
}

async fn run(
    client: Client,
    mut cmd_rx: mpsc::UnboundedReceiver<ClientCommand>,
    event_tx: mpsc::UnboundedSender<ClientEvent>,
    media_cache_dir: PathBuf,
) -> anyhow::Result<()> {
    // `with_offline_mode`: sync errors park the service in an auto-retrying
    // Offline state instead of the terminal Error state. Without it, any
    // sustained run of failed requests (a saturated uplink during a large
    // upload, Wi-Fi blip, server restart) killed sync permanently — the only
    // `.start()` call is the one below, so nothing ever restarted the loops
    // and the app stopped receiving messages until relaunch.
    let sync_service =
        SyncService::builder(client.clone()).with_offline_mode().build().await?;
    let mut state_stream = sync_service.state();
    let _ = event_tx.send(ClientEvent::SyncStateChanged(SyncState::Connecting));
    sync_service.start().await;

    // Detached: run for the process lifetime, clean themselves up once
    // `event_tx` is gone (send fails, their loops break).
    let _room_list_handle = room_list::spawn_forwarder(client.clone(), event_tx.clone());
    let _notification_watcher = crate::push::spawn_watcher(client.clone(), event_tx.clone());
    let call_manager = crate::calls::CallManager::spawn(client.clone(), event_tx.clone());
    // Detached for the process lifetime, like the watchers above — but this
    // one doesn't touch `event_tx` at all, so it has no natural shutdown
    // signal of its own; that's fine, it just rides out with the process.
    let _send_queue_recovery = spawn_send_queue_recovery(client.clone());
    // Rooms the worker keeps subscribed for the session flow in here: the
    // joined-space discovery below reports the spaces it finds, and `JoinRoom`
    // reports what it joins. The worker owns the single active subscription
    // set (see `resubscribe`); components no longer subscribe on their own,
    // because since matrix-sdk-ui 0.17 each `subscribe_to_rooms` REPLACES the
    // set and would clobber the others.
    let (pin_tx, mut pin_rx) = mpsc::unbounded_channel::<Vec<OwnedRoomId>>();

    // One-shot rather than lifetime, but detached for the same reason:
    // sliding sync never delivers `m.space` rooms (list filter), so joined
    // spaces have to be discovered over REST and subscribed to explicitly or
    // the sidebar's Spaces section stays empty.
    let _space_discovery = crate::rooms::spaces::spawn_joined_spaces_subscriber(
        client.clone(),
        sync_service.room_list_service(),
        pin_tx.clone(),
        event_tx.clone(),
    );

    // One-time startup checks. Blocking the command loop briefly here is
    // fine — there's nothing meaningful for the UI to command yet (no room
    // is open) while these run.
    let pending_cross_signing_session =
        crate::key_backup::bootstrap_cross_signing(&client, &event_tx).await;
    crate::key_backup::check_recovery_state(&client, &event_tx).await;

    let (incoming_verification_tx, mut incoming_verification_rx) = mpsc::unbounded_channel();
    register_verification_handlers(&client, incoming_verification_tx);

    let mut worker_state = WorkerState {
        open_rooms: HashMap::new(),
        pinned_subscriptions: HashSet::new(),
        verification: None,
        pending_cross_signing_session,
    };
    // Tracks whether the *previous* mapped state was Offline/Error, so the
    // edge back into Syncing (not every observation of it) is what triggers
    // send-queue recovery below — see the state_stream arm.
    let mut was_disconnected = false;

    loop {
        tokio::select! {
            Some(state) = state_stream.next() => {
                let mapped = match state {
                    sync_service::State::Idle => SyncState::Connecting,
                    sync_service::State::Running => SyncState::Syncing,
                    sync_service::State::Terminated => SyncState::Offline,
                    sync_service::State::Error(_) => {
                        SyncState::Error("sync service reported an error state".into())
                    }
                    sync_service::State::Offline => SyncState::Offline,
                };
                // Reconnect-triggered send-queue recovery: catches a room
                // whose queue disabled itself because the *whole* connection
                // dropped (as opposed to a single request timing out while
                // sync stayed healthy — `spawn_send_queue_recovery` handles
                // that case instead). Gated on the edge, not every `Syncing`
                // observation, because `SendQueue::set_enabled` does an
                // unconditional local store read on every call.
                if matches!(mapped, SyncState::Syncing) && was_disconnected {
                    let client = client.clone();
                    tokio::spawn(async move {
                        client.send_queue().set_enabled(true).await;
                    });
                }
                was_disconnected = matches!(mapped, SyncState::Offline | SyncState::Error(_));
                let _ = event_tx.send(ClientEvent::SyncStateChanged(mapped));
            }

            Some(request) = incoming_verification_rx.recv() => {
                let already_active = worker_state.verification.as_ref().is_some_and(VerificationSession::is_active);
                if already_active {
                    // Only one flow at a time; politely decline the new one.
                    tokio::spawn(async move { let _ = request.cancel().await; });
                } else {
                    worker_state.verification = Some(verification::spawn(request, event_tx.clone()));
                }
            }

            Some(room_ids) = pin_rx.recv() => {
                // A component (space discovery or a join) asked to keep these
                // rooms subscribed for the session; record them and re-send
                // the whole active set.
                for id in room_ids {
                    worker_state.pinned_subscriptions.insert(id);
                }
                resubscribe(
                    &sync_service,
                    &worker_state.pinned_subscriptions,
                    &worker_state.open_rooms,
                    None,
                )
                .await;
            }

            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else {
                    // UI dropped its sender; shut down cleanly.
                    break;
                };
                handle_command(&client, &sync_service, &call_manager, &mut worker_state, cmd, &event_tx, &media_cache_dir, &pin_tx).await;
            }
        }
    }

    worker_state.open_rooms.clear();
    call_manager.leave_all().await;
    sync_service.stop().await;

    Ok(())
}

/// Error-triggered send-queue recovery: re-enables a room's send queue
/// shortly after matrix-sdk disables it for a recoverable error (a plain
/// request timeout, most commonly — see `ClientEvent`'s doc comment on
/// `TimelineItem::send_failed` for the full story). Deliberately a
/// standalone spawned loop rather than a `tokio::select!` arm on
/// `error_rx.recv()` directly: a `Lagged` result has to `continue`, and
/// matching only `Ok(...)` in a `select!` arm would silently stop polling
/// this branch forever the first time that happens (the same hazard
/// `rooms::timeline::spawn_typing_forwarder` already routes around the same
/// way). Targets `client.get_room` rather than `worker_state.open_rooms` so
/// it recovers a room even if the UI isn't currently looking at it.
fn spawn_send_queue_recovery(client: Client) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut error_rx = client.send_queue().subscribe_errors();
        loop {
            match error_rx.recv().await {
                Ok(SendQueueRoomError { room_id, is_recoverable: true, .. }) => {
                    tokio::time::sleep(SEND_QUEUE_RETRY_DELAY).await;
                    if let Some(room) = client.get_room(&room_id) {
                        room.send_queue().set_enabled(true);
                    }
                }
                // Unrecoverable ("wedged") errors need `SendHandle::unwedge()`
                // on the specific stuck request, not a blanket re-enable —
                // out of scope for now, see `TimelineItem::send_failed`'s doc
                // comment for why the UI doesn't offer a Retry button here.
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

/// Registers handlers for both transports a verification request can
/// arrive through (direct to-device, or as a message in a shared room),
/// forwarding the discovered `VerificationRequest` to the main loop via
/// `tx` — matrix-sdk has no first-class stream for this, so an event
/// handler plus a lookup by flow id is the documented approach.
fn register_verification_handlers(client: &Client, tx: mpsc::UnboundedSender<VerificationRequest>) {
    let to_device_tx = tx.clone();
    client.add_event_handler(
        move |ev: ToDeviceEvent<ToDeviceKeyVerificationRequestEventContent>, client: Client| {
            let tx = to_device_tx.clone();
            async move {
                let flow_id = ev.content.transaction_id.to_string();
                if let Some(request) = client.encryption().get_verification_request(&ev.sender, &flow_id).await {
                    let _ = tx.send(request);
                }
            }
        },
    );

    client.add_event_handler(move |ev: OriginalSyncRoomMessageEvent, client: Client| {
        let tx = tx.clone();
        async move {
            if matches!(ev.content.msgtype, MessageType::VerificationRequest(_)) {
                let flow_id = ev.event_id.to_string();
                if let Some(request) = client.encryption().get_verification_request(&ev.sender, &flow_id).await {
                    let _ = tx.send(request);
                }
            }
        }
    });
}

/// Re-sends the full active room-subscription set. Since matrix-sdk-ui 0.17
/// `subscribe_to_rooms` REPLACES the previous set ("All previous room
/// subscriptions will be forgotten"), so every call must carry the whole set
/// we want live: the session-pinned rooms (joined spaces + rooms joined this
/// session, which sliding sync's list filter would otherwise keep out of the
/// store) plus every currently-open room (each needs a real timeline window so
/// live events append instead of resetting), plus an optional room being
/// opened right now that isn't recorded in `open_rooms` yet.
async fn resubscribe(
    sync_service: &SyncService,
    pinned: &HashSet<OwnedRoomId>,
    open_rooms: &HashMap<String, RoomHandles>,
    opening: Option<&RoomId>,
) {
    let mut set: HashSet<OwnedRoomId> = pinned.clone();
    for key in open_rooms.keys() {
        if let Ok(id) = RoomId::parse(key) {
            set.insert(id);
        }
    }
    if let Some(id) = opening {
        set.insert(id.to_owned());
    }
    if set.is_empty() {
        return;
    }
    let refs: Vec<&RoomId> = set.iter().map(std::ops::Deref::deref).collect();
    sync_service.room_list_service().subscribe_to_rooms(&refs).await;
}

// One parameter per worker-owned resource a command can touch; bundling them
// into a context struct would just move the same list one level down.
#[allow(clippy::too_many_arguments)]
async fn handle_command(
    client: &Client,
    sync_service: &SyncService,
    call_manager: &std::sync::Arc<crate::calls::CallManager>,
    worker_state: &mut WorkerState,
    cmd: ClientCommand,
    event_tx: &mpsc::UnboundedSender<ClientEvent>,
    media_cache_dir: &std::path::Path,
    pin_tx: &mpsc::UnboundedSender<Vec<OwnedRoomId>>,
) {
    match cmd {
        ClientCommand::Logout => {
            call_manager.leave_all().await;
            sync_service.stop().await;
            let _ = event_tx.send(ClientEvent::LoggedOut);
        }

        ClientCommand::JoinCall { room_id, request_id } => {
            // Several HTTP round trips (delayed event, state event, possibly
            // .well-known) — detached like every other network-bound command.
            let manager = call_manager.clone();
            tokio::spawn(async move { manager.join(room_id, request_id).await });
        }
        ClientCommand::LeaveCall { room_id, request_id } => {
            let manager = call_manager.clone();
            tokio::spawn(async move { manager.leave(room_id, request_id).await });
        }

        ClientCommand::OpenRoom { room_id } => {
            // Sliding sync delivers unsubscribed rooms through the all-rooms
            // list, which requests `timeline_limit: 1` — so in an active
            // room nearly every sync response is `limited` (a gap), and the
            // SDK reacts to gaps by clearing and re-seeding the room's
            // timeline. The UI sees its whole item list replaced by a
            // shorter one and the scroll position collapses to the gap
            // boundary — the "timeline randomly jumps back up to some old
            // post" bug. Subscribing upgrades the open room to a real
            // timeline window (limit 20 + full required state), so live
            // updates append instead of resetting. Since matrix-sdk-ui 0.17
            // `subscribe_to_rooms` REPLACES its set ("All previous room
            // subscriptions will be forgotten"), so re-send the whole active
            // set (pinned spaces + every open room + this one) — subscribing
            // to just this room would silently drop the joined-space
            // subscriptions and empty the sidebar.
            if let Ok(parsed_room_id) = RoomId::parse(&room_id) {
                resubscribe(
                    sync_service,
                    &worker_state.pinned_subscriptions,
                    &worker_state.open_rooms,
                    Some(&parsed_room_id),
                )
                .await;
            }
            match timeline::open(client, room_id.clone(), event_tx.clone()).await {
                Ok((timeline, timeline_task)) => {
                    let room = timeline.room().clone();

                    // Full roster fetch can hit the network (lazy member
                    // loading) — detached so opening the room stays instant.
                    let members_room = room.clone();
                    let members_tx = event_tx.clone();
                    let members_room_id = room_id.clone();
                    tokio::spawn(async move {
                        let members = room_list::fetch_members(&members_room).await;
                        let _ = members_tx.send(ClientEvent::RoomMembersUpdated {
                            room_id: members_room_id,
                            members,
                        });
                    });

                    let typing_task = timeline::spawn_typing_forwarder(
                        room.clone(),
                        room_id.clone(),
                        event_tx.clone(),
                    );

                    let emoji_client = client.clone();
                    let emoji_event_tx = event_tx.clone();
                    let emoji_room = room.clone();
                    tokio::spawn(async move {
                        // None = transport failure somewhere; keep the UI's
                        // current pack set instead of wiping custom emoji
                        // over a network blip.
                        if let Some(packs) =
                            crate::rooms::emoji_packs::fetch_all(&emoji_client, Some(&emoji_room)).await
                        {
                            let _ = emoji_event_tx.send(ClientEvent::CustomEmojiPacksUpdated(packs));
                        }
                    });

                    let tags_event_tx = event_tx.clone();
                    let tags_room = room.clone();
                    let tags_room_id = room_id.clone();
                    tokio::spawn(async move {
                        let tags = crate::rooms::power_tags::fetch(&tags_room).await;
                        let _ = tags_event_tx.send(ClientEvent::PowerLevelTagsUpdated {
                            room_id: tags_room_id,
                            tags,
                        });
                    });

                    // Current call state (usually "none") so the banner is
                    // right the moment the room opens; the calls watcher
                    // keeps it live from here. snapshot_for (not the raw
                    // snapshot): the store lags our own membership until the
                    // sync echo, and opening the room in that window would
                    // show "Join" mid-call.
                    let call_event_tx = event_tx.clone();
                    let call_room = room;
                    let call_manager = call_manager.clone();
                    tokio::spawn(async move {
                        let state = call_manager.snapshot_for(&call_room).await;
                        let _ = call_event_tx.send(ClientEvent::CallStateUpdated(state));
                    });

                    // Pull a real chunk of history right away — sync alone
                    // only seeds the timeline with the last ~20 events.
                    // (Restored original behavior: this was removed while
                    // hunting the scroll bug, but prepended history never
                    // moves a *bottom-anchored* view — the jumps it seemed
                    // to cause were the top-anchoring root bug.) Detached:
                    // items arrive via the timeline's own diff stream.
                    let paginate_timeline = timeline.clone();
                    let paginate_event_tx = event_tx.clone();
                    let paginate_room_id = room_id.clone();
                    tokio::spawn(async move {
                        match paginate_timeline.paginate_backwards(60).await {
                            Ok(true) => {
                                let _ = paginate_event_tx.send(ClientEvent::TimelineStartReached {
                                    room_id: paginate_room_id,
                                });
                            }
                            Ok(false) => {}
                            Err(error) => {
                                tracing::warn!(room_id = %paginate_room_id, %error, "initial back-pagination failed");
                            }
                        }
                    });

                    worker_state
                        .open_rooms
                        .insert(room_id, RoomHandles { timeline, timeline_task, typing_task });
                }
                Err(error) => {
                    tracing::warn!(%room_id, %error, "failed to open room timeline");
                }
            }
        }

        ClientCommand::CloseRoom { room_id } => {
            worker_state.open_rooms.remove(&room_id);
            // Relinquish the closed room's subscription slot (kept only if it
            // is pinned) by re-sending the now-smaller active set.
            resubscribe(
                sync_service,
                &worker_state.pinned_subscriptions,
                &worker_state.open_rooms,
                None,
            )
            .await;
        }

        ClientCommand::OpenDirectMessage { user_id, encrypted, request_id } => {
            let Ok(parsed_user_id) = UserId::parse(&user_id) else {
                fail(event_tx, request_id, "invalid user id");
                return;
            };
            let client = client.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                // Reuse any existing DM with this user (whatever its
                // encryption) so we don't spawn a duplicate room every time.
                if let Some(room_id) = find_dm_room(&client, &parsed_user_id).await {
                    let _ = event_tx
                        .send(ClientEvent::DirectMessageReady { room_id: room_id.to_string() });
                    succeed(&event_tx, request_id);
                    return;
                }
                // Build the DM room ourselves (rather than `create_dm`, which
                // always encrypts when the crate's e2e feature is on) so the
                // caller's `encrypted` flag decides: same DM shape either way
                // (`is_direct`, TrustedPrivateChat), with the encryption state
                // event added only when requested.
                use matrix_sdk::ruma::api::client::room::create_room;
                let mut request = create_room::v3::Request::new();
                request.invite = vec![parsed_user_id];
                request.is_direct = true;
                request.preset = Some(create_room::v3::RoomPreset::TrustedPrivateChat);
                if encrypted {
                    request.initial_state = vec![encryption_initial_state()];
                }
                match client.create_room(request).await {
                    Ok(room) => {
                        let _ = event_tx.send(ClientEvent::DirectMessageReady {
                            room_id: room.room_id().to_string(),
                        });
                        succeed(&event_tx, request_id);
                    }
                    Err(error) => fail(&event_tx, request_id, &error.to_string()),
                }
            });
        }

        ClientCommand::PaginateBackwards { room_id, request_id } => {
            let Some(handles) = worker_state.open_rooms.get(&room_id) else {
                fail(event_tx, request_id, "room is not open");
                return;
            };
            let timeline = handles.timeline.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                match timeline.paginate_backwards(50).await {
                    Ok(reached_start) => {
                        if reached_start {
                            let _ = event_tx.send(ClientEvent::TimelineStartReached { room_id });
                        }
                        succeed(&event_tx, request_id);
                    }
                    Err(error) => fail(&event_tx, request_id, &error.to_string()),
                }
            });
        }

        ClientCommand::SendMessage { room_id, body, mentioned_user_ids, reply_to_event_id, emote, markdown, request_id } => {
            let Some(handles) = worker_state.open_rooms.get(&room_id) else {
                fail(event_tx, request_id, "room is not open");
                return;
            };

            // `/me` actions ride as `m.emote`; a `/plain` send skips Markdown
            // and posts the body verbatim; everything else is Markdown `m.text`.
            // All take the same mentions/reply handling below.
            let mut content = if emote {
                RoomMessageEventContent::emote_markdown(body)
            } else if markdown {
                RoomMessageEventContent::text_markdown(body)
            } else {
                RoomMessageEventContent::text_plain(body)
            };
            if !mentioned_user_ids.is_empty() {
                match parse_user_ids(&mentioned_user_ids) {
                    Ok(ids) => content = content.add_mentions(Mentions::with_user_ids(ids)),
                    Err(error) => {
                        tracing::warn!(%error, "invalid mentioned user id, sending without mentions");
                    }
                }
            }

            if let Some(reply_to) = reply_to_event_id {
                let Ok(reply_event_id) = EventId::parse(&reply_to) else {
                    fail(event_tx, request_id, "invalid reply event id");
                    return;
                };
                // 0.18: send_reply takes the reply-target event id directly and
                // decides threading itself (a reply in the main timeline stays
                // in the main timeline, matching the old MaybeThreaded). It
                // resolves the quoted event via make_reply_event, which falls
                // back to a blocking /event fetch when it isn't in the event
                // cache — spawn it so a slow request doesn't stall the command
                // loop (same hazard as RedactEvent / SendAttachment).
                let timeline = handles.timeline.clone();
                let event_tx = event_tx.clone();
                tokio::spawn(async move {
                    match timeline.send_reply(content.into(), reply_event_id).await {
                        Ok(()) => succeed(&event_tx, request_id),
                        Err(error) => fail(&event_tx, request_id, &error.to_string()),
                    }
                });
                return;
            }

            match handles.timeline.send(content.into()).await {
                Ok(_) => succeed(event_tx, request_id),
                Err(error) => fail(event_tx, request_id, &error.to_string()),
            }
        }

        ClientCommand::EditMessage { room_id, event_id, new_body, request_id } => {
            let Some(handles) = worker_state.open_rooms.get(&room_id) else {
                fail(event_tx, request_id, "room is not open");
                return;
            };
            let Ok(event_id) = EventId::parse(&event_id) else {
                fail(event_tx, request_id, "invalid event id");
                return;
            };

            let item_id = TimelineEventItemId::EventId(event_id);
            let content =
                EditedContent::RoomMessage(RoomMessageEventContentWithoutRelation::text_markdown(new_body));

            match handles.timeline.edit(&item_id, content).await {
                Ok(()) => succeed(event_tx, request_id),
                Err(error) => fail(event_tx, request_id, &error.to_string()),
            }
        }

        ClientCommand::RedactEvent { room_id, event_id, reason, request_id } => {
            let Some(handles) = worker_state.open_rooms.get(&room_id) else {
                fail(event_tx, request_id, "room is not open");
                return;
            };
            let Ok(event_id) = EventId::parse(&event_id) else {
                fail(event_tx, request_id, "invalid event id");
                return;
            };

            // Redaction is a direct HTTP round trip (not send-queue backed) —
            // spawn it so a slow request doesn't stall the command loop.
            let item_id = TimelineEventItemId::EventId(event_id);
            let timeline = handles.timeline.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                match timeline.redact(&item_id, reason.as_deref()).await {
                    Ok(()) => succeed(&event_tx, request_id),
                    Err(error) => fail(&event_tx, request_id, &error.to_string()),
                }
            });
        }

        ClientCommand::SendAttachment {
            room_id,
            filename,
            bytes,
            mime,
            caption,
            mentioned_user_ids,
            reply_to_event_id,
            request_id,
        } => {
            let Some(handles) = worker_state.open_rooms.get(&room_id) else {
                fail(event_tx, request_id, "room is not open");
                return;
            };

            let mime_type: mime::Mime =
                mime.parse().unwrap_or(mime::APPLICATION_OCTET_STREAM);
            // 0.18: matrix-sdk-ui's Timeline::send_attachment takes ITS OWN
            // AttachmentConfig — a plain struct (not matrix_sdk's builder).
            // Caption is a TextMessageEventContent (markdown → body + HTML
            // formatted_body, MSC2530); the reply is just the target event id
            // (the SDK derives threading/mentions itself).
            let caption = caption.map(
                matrix_sdk::ruma::events::room::message::TextMessageEventContent::markdown,
            );
            let mentions = if mentioned_user_ids.is_empty() {
                None
            } else {
                match parse_user_ids(&mentioned_user_ids) {
                    Ok(ids) => Some(Mentions::with_user_ids(ids)),
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            "invalid mentioned user id, sending attachment without mentions"
                        );
                        None
                    }
                }
            };
            let in_reply_to = match reply_to_event_id {
                Some(reply_to) => match EventId::parse(&reply_to) {
                    Ok(id) => Some(id),
                    Err(_) => {
                        fail(event_tx, request_id, "invalid reply event id");
                        return;
                    }
                },
                None => None,
            };
            let config = matrix_sdk_ui::timeline::AttachmentConfig {
                txn_id: None,
                info: None,
                thumbnail: None,
                caption,
                mentions,
                in_reply_to,
            };

            // send_attachment performs the full media upload before returning
            // (no send queue for attachments in SDK 0.13) — awaiting it inline
            // would freeze every other command (incl. room switches) for the
            // whole upload on a slow uplink.
            let timeline = handles.timeline.clone();
            let event_tx = event_tx.clone();
            let client = client.clone();
            tokio::spawn(async move {
                // Pre-flight against the server's advertised upload cap
                // (cached client-side after the first fetch). Without this,
                // an over-limit file grinds through a doomed multi-minute
                // upload that monopolizes the HTTP connection — starving
                // sliding sync and every other request — before the server
                // rejects it anyway. Fail-open: if the cap can't be fetched,
                // let the upload attempt proceed.
                if let Ok(limit) = client.load_or_fetch_max_upload_size().await {
                    if bytes.len() as u64 > u64::from(limit) {
                        let file_mb = bytes.len() as f64 / (1024.0 * 1024.0);
                        let cap_mb = u64::from(limit) as f64 / (1024.0 * 1024.0);
                        fail(
                            &event_tx,
                            request_id,
                            &format!(
                                "{filename} is {file_mb:.1} MB — over the server's \
                                 {cap_mb:.1} MB upload limit"
                            ),
                        );
                        return;
                    }
                }
                let source = AttachmentSource::Data { bytes, filename };
                match timeline.send_attachment(source, mime_type, config).await {
                    Ok(()) => succeed(&event_tx, request_id),
                    Err(error) => fail(&event_tx, request_id, &error.to_string()),
                }
            });
        }

        ClientCommand::SendSticker { room_id, url, body, width, height, mimetype, request_id } => {
            let Some(handles) = worker_state.open_rooms.get(&room_id) else {
                fail(event_tx, request_id, "room is not open");
                return;
            };
            // Picked stickers already point at a hosted `mxc://` image, so
            // there's nothing to upload — this is a plain event send.
            if !url.starts_with("mxc://") {
                fail(event_tx, request_id, "sticker url is not an mxc uri");
                return;
            }
            let mut info = ImageInfo::new();
            info.width = width.map(UInt::from);
            info.height = height.map(UInt::from);
            info.mimetype = mimetype;
            let content = StickerEventContent::new(body, info, OwnedMxcUri::from(url));

            match handles.timeline.send(AnyMessageLikeEventContent::Sticker(content)).await {
                Ok(_) => succeed(event_tx, request_id),
                Err(error) => fail(event_tx, request_id, &error.to_string()),
            }
        }

        ClientCommand::SetTyping { room_id, typing } => {
            if let Some(handles) = worker_state.open_rooms.get(&room_id) {
                let room = handles.timeline.room().clone();
                tokio::spawn(async move {
                    let _ = room.typing_notice(typing).await;
                });
            }
        }

        ClientCommand::RetrySend { room_id } => {
            // Room-scoped and synchronous/infallible — no need to spawn.
            if let Some(handles) = worker_state.open_rooms.get(&room_id) {
                handles.timeline.room().send_queue().set_enabled(true);
            }
        }

        ClientCommand::RetryCrossSigningBootstrap => {
            if let Some(session) = worker_state.pending_cross_signing_session.take() {
                // Inline on purpose (rare, user-initiated, one HTTP round
                // trip): the retry can itself come back needing another UIAA
                // fallback round, and the new session id must land back in
                // worker_state — a detached task couldn't store it.
                worker_state.pending_cross_signing_session =
                    crate::key_backup::retry_with_fallback(client, session, event_tx).await;
            }
        }

        ClientCommand::StartVerification { user_id } => {
            let Ok(user_id) = UserId::parse(&user_id) else {
                tracing::warn!(%user_id, "invalid user id for verification");
                return;
            };
            let identity = match client.encryption().get_user_identity(&user_id).await {
                Ok(Some(identity)) => identity,
                Ok(None) => {
                    tracing::warn!(%user_id, "no cross-signing identity found for user");
                    return;
                }
                Err(error) => {
                    tracing::warn!(%error, "failed to fetch user identity");
                    return;
                }
            };
            match identity.request_verification().await {
                Ok(request) => {
                    worker_state.verification = Some(verification::spawn(request, event_tx.clone()));
                }
                Err(error) => tracing::warn!(%error, "failed to start verification"),
            }
        }

        ClientCommand::AcceptVerificationRequest => {
            if let Some(session) = &worker_state.verification {
                session.send(VerificationAction::Accept);
            }
        }
        ClientCommand::ConfirmSasMatch => {
            if let Some(session) = &worker_state.verification {
                session.send(VerificationAction::ConfirmMatch);
            }
        }
        ClientCommand::RejectSasMatch => {
            if let Some(session) = &worker_state.verification {
                session.send(VerificationAction::RejectMatch);
            }
        }
        ClientCommand::VerificationCancel => {
            if let Some(session) = &worker_state.verification {
                session.send(VerificationAction::Cancel);
            }
        }

        ClientCommand::EnableRecovery { passphrase, request_id } => {
            let client = client.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                crate::key_backup::enable_recovery(&client, passphrase, &event_tx, request_id).await;
            });
        }
        ClientCommand::RestoreFromBackup { recovery_key, request_id } => {
            let client = client.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                crate::key_backup::restore_from_backup(&client, recovery_key, &event_tx, request_id).await;
            });
        }

        ClientCommand::ToggleReaction { room_id, event_id, key, request_id } => {
            let Some(handles) = worker_state.open_rooms.get(&room_id) else {
                fail(event_tx, request_id, "room is not open");
                return;
            };
            let Ok(event_id) = EventId::parse(&event_id) else {
                fail(event_tx, request_id, "invalid event id");
                return;
            };
            let item_id = TimelineEventItemId::EventId(event_id);
            match handles.timeline.toggle_reaction(&item_id, &key).await {
                // 0.18 returns whether the reaction is now set; we only need success.
                Ok(_) => succeed(event_tx, request_id),
                Err(error) => fail(event_tx, request_id, &error.to_string()),
            }
        }

        ClientCommand::MarkRoomRead { room_id, public_receipt } => {
            if let Some(handles) = worker_state.open_rooms.get(&room_id) {
                let timeline = Arc::clone(&handles.timeline);
                tokio::spawn(async move {
                    // Two receipts, like Element: a read receipt (clears the
                    // unread badge) plus the private fully-read marker (what
                    // positions the "new messages" divider next time the room
                    // opens). The read receipt is public (`m.read`, federated,
                    // others see it) normally, but the *private* variant
                    // (`m.read.private`) when the user has read receipts off —
                    // it still advances the read position server-side (so
                    // unread badges clear on their own devices) without ever
                    // being shared with anyone else. The fully-read marker is
                    // room account data, never federated, so it's sent
                    // regardless. `mark_as_read` returns whether it actually
                    // sent — `false` means the SDK decided the receipt wasn't
                    // needed (e.g. it believes the marker is already at or
                    // past the latest event), which is invisible without
                    // logging and exactly what a stuck "new messages" divider
                    // would look like.
                    let read_receipt =
                        if public_receipt { ReceiptType::Read } else { ReceiptType::ReadPrivate };
                    match timeline.mark_as_read(read_receipt).await {
                        Ok(sent) => tracing::debug!(sent, public = public_receipt, "read receipt"),
                        Err(error) => tracing::warn!(%error, "failed to send read receipt"),
                    }
                    match timeline.mark_as_read(ReceiptType::FullyRead).await {
                        Ok(sent) => tracing::debug!(sent, "fully-read marker"),
                        Err(error) => tracing::warn!(%error, "failed to send fully-read marker"),
                    }
                });
            }
        }

        ClientCommand::FetchMedia { mxc_url, request_id } => {
            let client = client.clone();
            let event_tx = event_tx.clone();
            let cache_dir = media_cache_dir.to_path_buf();
            tokio::spawn(async move {
                match crate::media::fetch(&client, &cache_dir, &mxc_url).await {
                    Ok(bytes) => {
                        let _ = event_tx.send(ClientEvent::MediaFetched { request_id, bytes });
                    }
                    Err(error) => {
                        let _ = event_tx
                            .send(ClientEvent::MediaFetchFailed { request_id, reason: error.to_string() });
                    }
                }
            });
        }

        ClientCommand::SetRoomNotificationMode { room_id, mode, request_id } => {
            let Ok(parsed_room_id) = RoomId::parse(&room_id) else {
                fail(event_tx, request_id, "invalid room id");
                return;
            };
            let sdk_mode = match mode {
                crate::events::NotificationMode::AllMessages => SdkNotificationMode::AllMessages,
                crate::events::NotificationMode::MentionsAndKeywordsOnly => {
                    SdkNotificationMode::MentionsAndKeywordsOnly
                }
                crate::events::NotificationMode::Mute => SdkNotificationMode::Mute,
            };
            let client = client.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                let settings = client.notification_settings().await;
                match settings.set_room_notification_mode(&parsed_room_id, sdk_mode).await {
                    Ok(()) => {
                        let _ = event_tx.send(ClientEvent::RoomNotificationModeChanged {
                            room_id: parsed_room_id.to_string(),
                            mode,
                        });
                        succeed(&event_tx, request_id);
                    }
                    Err(error) => fail(&event_tx, request_id, &error.to_string()),
                }
            });
        }

        ClientCommand::FetchUrlPreview { url } => {
            let client = client.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                use matrix_sdk::ruma::api::client::authenticated_media::get_media_preview;

                let request = get_media_preview::v1::Request::new(url.clone());
                let event = match client.send(request).await {
                    Ok(response) => match response.data {
                        Some(raw) => match serde_json::from_str::<serde_json::Value>(raw.get()) {
                            Ok(og) => {
                                let field = |key: &str| {
                                    og.get(key)
                                        .and_then(|v| v.as_str())
                                        .map(str::trim)
                                        .filter(|s| !s.is_empty())
                                        .map(ToOwned::to_owned)
                                };
                                // Dimensions arrive as numbers or strings
                                // depending on the scraped page.
                                let dimension = |key: &str| {
                                    og.get(key).and_then(|v| {
                                        v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                                    })
                                };
                                ClientEvent::UrlPreviewFetched(crate::events::UrlPreview {
                                    url,
                                    title: field("og:title"),
                                    description: field("og:description"),
                                    site_name: field("og:site_name"),
                                    image_mxc: field("og:image"),
                                    image_width: dimension("og:image:width"),
                                    image_height: dimension("og:image:height"),
                                })
                            }
                            Err(error) => {
                                tracing::debug!(%error, "url preview data wasn't valid JSON");
                                ClientEvent::UrlPreviewFailed { url }
                            }
                        },
                        None => ClientEvent::UrlPreviewFailed { url },
                    },
                    Err(error) => {
                        tracing::debug!(%error, "url preview request failed");
                        ClientEvent::UrlPreviewFailed { url }
                    }
                };
                let _ = event_tx.send(event);
            });
        }

        ClientCommand::SetDefaultNotificationMode { scope, mode, request_id } => {
            let is_one_to_one = match scope {
                crate::events::NotificationScope::DirectMessages => IsOneToOne::Yes,
                crate::events::NotificationScope::GroupChats => IsOneToOne::No,
            };
            let sdk_mode = match mode {
                crate::events::NotificationMode::AllMessages => SdkNotificationMode::AllMessages,
                crate::events::NotificationMode::MentionsAndKeywordsOnly => {
                    SdkNotificationMode::MentionsAndKeywordsOnly
                }
                crate::events::NotificationMode::Mute => SdkNotificationMode::Mute,
            };
            let client = client.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                let settings = client.notification_settings().await;
                // The UI only exposes one control per scope, not the SDK's
                // full encrypted x one-to-one matrix, so both encrypted
                // variants are kept in sync together (same simplification
                // Element makes). The push-rules watcher in `push.rs`
                // re-snapshots both defaults once this account-data change
                // lands, so no event is sent from here directly.
                let first = settings
                    .set_default_room_notification_mode(IsEncrypted::Yes, is_one_to_one, sdk_mode)
                    .await;
                let second = settings
                    .set_default_room_notification_mode(IsEncrypted::No, is_one_to_one, sdk_mode)
                    .await;
                match first.and(second) {
                    Ok(()) => succeed(&event_tx, request_id),
                    Err(error) => fail(&event_tx, request_id, &error.to_string()),
                }
            });
        }

        ClientCommand::ClearRoomNotificationMode { room_id, request_id } => {
            let Ok(parsed_room_id) = RoomId::parse(&room_id) else {
                fail(event_tx, request_id, "invalid room id");
                return;
            };
            let client = client.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                let settings = client.notification_settings().await;
                match settings.delete_user_defined_room_rules(&parsed_room_id).await {
                    Ok(()) => {
                        let _ = event_tx.send(ClientEvent::RoomNotificationModeCleared {
                            room_id: parsed_room_id.to_string(),
                        });
                        succeed(&event_tx, request_id);
                    }
                    Err(error) => fail(&event_tx, request_id, &error.to_string()),
                }
            });
        }

        ClientCommand::CreateRoomWith { user_id, encrypted, request_id } => {
            let Ok(parsed_user_id) = UserId::parse(&user_id) else {
                fail(event_tx, request_id, "invalid user id");
                return;
            };
            let client = client.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                use matrix_sdk::ruma::api::client::room::create_room;
                let mut request = create_room::v3::Request::new();
                // A group room (not a DM): invite-only, creator is admin.
                request.invite = vec![parsed_user_id];
                request.is_direct = false;
                request.preset = Some(create_room::v3::RoomPreset::PrivateChat);
                if encrypted {
                    request.initial_state = vec![encryption_initial_state()];
                }
                match client.create_room(request).await {
                    Ok(room) => {
                        let _ = event_tx.send(ClientEvent::RoomCreated {
                            room_id: room.room_id().to_string(),
                        });
                        succeed(&event_tx, request_id);
                    }
                    Err(error) => fail(&event_tx, request_id, &error.to_string()),
                }
            });
        }

        ClientCommand::SearchUsers { query, request_id } => {
            let client = client.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                // 20 is plenty for a pick-a-person list; the server also caps
                // and flags `limited` when it truncates.
                match client.search_users(&query, 20).await {
                    Ok(response) => {
                        let results = response
                            .results
                            .into_iter()
                            .map(|user| UserSearchResult {
                                user_id: user.user_id.to_string(),
                                display_name: user.display_name,
                                avatar_url: user.avatar_url.map(|url| url.to_string()),
                            })
                            .collect();
                        let _ = event_tx.send(ClientEvent::UserSearchResults {
                            request_id,
                            results,
                            limited: response.limited,
                        });
                        succeed(&event_tx, request_id);
                    }
                    Err(error) => fail(&event_tx, request_id, &error.to_string()),
                }
            });
        }

        ClientCommand::CreateRoom { encrypted, request_id } => {
            let client = client.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                use matrix_sdk::ruma::api::client::room::create_room;
                let mut request = create_room::v3::Request::new();
                // A solo room: no invites, creator is the sole (admin) member.
                request.is_direct = false;
                request.preset = Some(create_room::v3::RoomPreset::PrivateChat);
                if encrypted {
                    request.initial_state = vec![encryption_initial_state()];
                }
                match client.create_room(request).await {
                    Ok(room) => {
                        let _ = event_tx.send(ClientEvent::RoomCreated {
                            room_id: room.room_id().to_string(),
                        });
                        succeed(&event_tx, request_id);
                    }
                    Err(error) => fail(&event_tx, request_id, &error.to_string()),
                }
            });
        }

        ClientCommand::SetRoomName { room_id, name, request_id } => {
            let Ok(parsed_room_id) = RoomId::parse(&room_id) else {
                fail(event_tx, request_id, "invalid room id");
                return;
            };
            let Some(room) = client.get_room(&parsed_room_id) else {
                fail(event_tx, request_id, "unknown room");
                return;
            };
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                match room.set_name(name).await {
                    Ok(_) => succeed(&event_tx, request_id),
                    Err(error) => fail(&event_tx, request_id, &error.to_string()),
                }
            });
        }

        ClientCommand::InviteUser { room_id, user_id, request_id } => {
            let Ok(parsed_room_id) = RoomId::parse(&room_id) else {
                fail(event_tx, request_id, "invalid room id");
                return;
            };
            let Ok(parsed_user_id) = UserId::parse(&user_id) else {
                fail(event_tx, request_id, "invalid user id");
                return;
            };
            let Some(room) = client.get_room(&parsed_room_id) else {
                fail(event_tx, request_id, "unknown room");
                return;
            };
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                match room.invite_user_by_id(&parsed_user_id).await {
                    Ok(()) => succeed(&event_tx, request_id),
                    Err(error) => fail(&event_tx, request_id, &error.to_string()),
                }
            });
        }

        ClientCommand::KickUser { room_id, user_id, reason, request_id } => {
            let Ok(parsed_room_id) = RoomId::parse(&room_id) else {
                fail(event_tx, request_id, "invalid room id");
                return;
            };
            let Ok(parsed_user_id) = UserId::parse(&user_id) else {
                fail(event_tx, request_id, "invalid user id");
                return;
            };
            let Some(room) = client.get_room(&parsed_room_id) else {
                fail(event_tx, request_id, "unknown room");
                return;
            };
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                match room.kick_user(&parsed_user_id, reason.as_deref()).await {
                    Ok(()) => succeed(&event_tx, request_id),
                    Err(error) => fail(&event_tx, request_id, &error.to_string()),
                }
            });
        }

        ClientCommand::BanUser { room_id, user_id, reason, request_id } => {
            let Ok(parsed_room_id) = RoomId::parse(&room_id) else {
                fail(event_tx, request_id, "invalid room id");
                return;
            };
            let Ok(parsed_user_id) = UserId::parse(&user_id) else {
                fail(event_tx, request_id, "invalid user id");
                return;
            };
            let Some(room) = client.get_room(&parsed_room_id) else {
                fail(event_tx, request_id, "unknown room");
                return;
            };
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                match room.ban_user(&parsed_user_id, reason.as_deref()).await {
                    Ok(()) => succeed(&event_tx, request_id),
                    Err(error) => fail(&event_tx, request_id, &error.to_string()),
                }
            });
        }

        ClientCommand::UnbanUser { room_id, user_id, request_id } => {
            let Ok(parsed_room_id) = RoomId::parse(&room_id) else {
                fail(event_tx, request_id, "invalid room id");
                return;
            };
            let Ok(parsed_user_id) = UserId::parse(&user_id) else {
                fail(event_tx, request_id, "invalid user id");
                return;
            };
            let Some(room) = client.get_room(&parsed_room_id) else {
                fail(event_tx, request_id, "unknown room");
                return;
            };
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                match room.unban_user(&parsed_user_id, None).await {
                    Ok(()) => succeed(&event_tx, request_id),
                    Err(error) => fail(&event_tx, request_id, &error.to_string()),
                }
            });
        }

        ClientCommand::SetRoomTopic { room_id, topic, request_id } => {
            let Ok(parsed_room_id) = RoomId::parse(&room_id) else {
                fail(event_tx, request_id, "invalid room id");
                return;
            };
            let Some(room) = client.get_room(&parsed_room_id) else {
                fail(event_tx, request_id, "unknown room");
                return;
            };
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                match room.set_room_topic(&topic).await {
                    Ok(_) => succeed(&event_tx, request_id),
                    Err(error) => fail(&event_tx, request_id, &error.to_string()),
                }
            });
        }

        ClientCommand::SetDisplayName { name, request_id } => {
            let client = client.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                match client.account().set_display_name(Some(name.as_str())).await {
                    Ok(()) => succeed(&event_tx, request_id),
                    Err(error) => fail(&event_tx, request_id, &error.to_string()),
                }
            });
        }

        ClientCommand::LeaveRoom { room_id, request_id } => {
            let Ok(parsed_room_id) = RoomId::parse(&room_id) else {
                fail(event_tx, request_id, "invalid room id");
                return;
            };
            let Some(room) = client.get_room(&parsed_room_id) else {
                fail(event_tx, request_id, "unknown room");
                return;
            };
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                match room.leave().await {
                    Ok(()) => succeed(&event_tx, request_id),
                    Err(error) => fail(&event_tx, request_id, &error.to_string()),
                }
            });
        }

        ClientCommand::ForgetRoom { room_id, request_id } => {
            let Ok(parsed_room_id) = RoomId::parse(&room_id) else {
                fail(event_tx, request_id, "invalid room id");
                return;
            };
            let Some(room) = client.get_room(&parsed_room_id) else {
                fail(event_tx, request_id, "unknown room");
                return;
            };
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                // `forget()` only accepts a Left/Banned room, so leave first
                // if we're still in it.
                if matches!(
                    room.state(),
                    matrix_sdk::RoomState::Joined
                        | matrix_sdk::RoomState::Invited
                        | matrix_sdk::RoomState::Knocked
                ) {
                    if let Err(error) = room.leave().await {
                        fail(&event_tx, request_id, &error.to_string());
                        return;
                    }
                }
                match room.forget().await {
                    Ok(()) => succeed(&event_tx, request_id),
                    Err(error) => fail(&event_tx, request_id, &error.to_string()),
                }
            });
        }

        ClientCommand::FetchSpaceHierarchy { space_id, from, request_id } => {
            let Ok(parsed_space_id) = RoomId::parse(&space_id) else {
                fail(event_tx, request_id, "invalid space id");
                return;
            };
            let client = client.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                match crate::rooms::spaces::fetch_children(&client, &parsed_space_id, from).await {
                    Ok((children, next_batch)) => {
                        let _ = event_tx.send(ClientEvent::SpaceHierarchyFetched {
                            request_id,
                            space_id,
                            children,
                            next_batch,
                        });
                    }
                    Err(error) => fail(&event_tx, request_id, &error.to_string()),
                }
            });
        }

        ClientCommand::JoinRoom { room_id_or_alias, via, request_id } => {
            let Ok(alias) = RoomOrAliasId::parse(&room_id_or_alias) else {
                fail(event_tx, request_id, "invalid room id or alias");
                return;
            };
            // Unparseable via entries are dropped rather than failing the
            // join — they're only routing hints.
            let via: Vec<OwnedServerName> =
                via.iter().filter_map(|s| ServerName::parse(s).ok()).collect();
            let client = client.clone();
            let event_tx = event_tx.clone();
            let pin_tx = pin_tx.clone();
            tokio::spawn(async move {
                match client.join_room_by_id_or_alias(&alias, &via).await {
                    Ok(room) => {
                        // Pin what we just joined for the session: if it's a
                        // space, sliding sync will never deliver it (list
                        // filter), and its type isn't knowable locally yet
                        // (the join response is just a room id; the create
                        // event only arrives via the subscription itself), so
                        // we can't tell a space from a plain room here — pin
                        // either way (harmless for a plain room). The worker
                        // folds it into the active set; subscribing directly
                        // here would clobber the spaces/open rooms (0.17+
                        // replace semantics).
                        let _ = pin_tx.send(vec![room.room_id().to_owned()]);
                        succeed(&event_tx, request_id);
                        // Same uncertainty about the type: sweep children so
                        // a just-joined space groups its rooms in the
                        // sidebar right away. On a plain room the hierarchy
                        // returns only the room itself, i.e. no children.
                        crate::rooms::spaces::emit_space_children(
                            &client,
                            room.room_id(),
                            &event_tx,
                        )
                        .await;
                    }
                    Err(error) => fail(&event_tx, request_id, &error.to_string()),
                }
            });
        }

        ClientCommand::KnockRoom { room_id_or_alias, via, request_id } => {
            let Ok(alias) = RoomOrAliasId::parse(&room_id_or_alias) else {
                fail(event_tx, request_id, "invalid room id or alias");
                return;
            };
            // Unparseable via entries are dropped rather than failing the
            // knock — they're only routing hints (same as JoinRoom).
            let via: Vec<OwnedServerName> =
                via.iter().filter_map(|s| ServerName::parse(s).ok()).collect();
            let client = client.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                // No reason attached: moderators see who's asking either
                // way, and prompting for prose is friction the flow doesn't
                // need. Nothing to pin or sweep on success either — unlike a
                // join, a knock doesn't put us in the room; if a moderator
                // accepts, the join arrives through sync like any other.
                match client.knock(alias, None, via).await {
                    Ok(_) => succeed(&event_tx, request_id),
                    Err(error) => fail(&event_tx, request_id, &error.to_string()),
                }
            });
        }

        other => {
            tracing::debug!(?other, "command not yet implemented");
        }
    }
}

fn parse_user_ids(ids: &[String]) -> Result<Vec<OwnedUserId>, matrix_sdk::ruma::IdParseError> {
    ids.iter().map(UserId::parse).collect()
}

fn succeed(event_tx: &mpsc::UnboundedSender<ClientEvent>, request_id: RequestId) {
    let _ = event_tx.send(ClientEvent::CommandSucceeded { request_id });
}

fn fail(event_tx: &mpsc::UnboundedSender<ClientEvent>, request_id: RequestId, error: &str) {
    // The UI surfaces `error` in whichever screen issued the command; log it
    // too so failures are diagnosable from the file after the fact.
    tracing::warn!(%request_id, error, "client command failed");
    let _ = event_tx.send(ClientEvent::CommandFailed { request_id, error: error.to_string() });
}

/// Resolve an existing direct-message room with `user_id`, preferring the
/// SDK's own view (`get_dm_room`) and falling back to the global `m.direct`
/// account data — local first, then an authoritative server fetch. On sliding
/// sync the per-room `direct_targets` is frequently empty even for real DMs,
/// so without this "open DM" spawns a fresh room every time instead of
/// reusing the existing conversation.
async fn find_dm_room(client: &Client, user_id: &UserId) -> Option<matrix_sdk::ruma::OwnedRoomId> {
    if let Some(room) = client.get_dm_room(user_id) {
        return Some(room.room_id().to_owned());
    }
    use matrix_sdk::ruma::events::{
        direct::{DirectEventContent, DirectUserIdentifier},
        GlobalAccountDataEventType,
    };
    let content = match client.account().account_data::<DirectEventContent>().await {
        Ok(Some(raw)) => raw.deserialize().ok(),
        _ => None,
    };
    let content = match content {
        Some(content) => Some(content),
        None => client
            .account()
            .fetch_account_data(GlobalAccountDataEventType::Direct)
            .await
            .ok()
            .flatten()
            .and_then(|raw| raw.deserialize_as_unchecked::<DirectEventContent>().ok()),
    };
    let content = content?;
    let room_ids = content.get(<&DirectUserIdentifier>::from(user_id))?;
    // Only reuse a room we're actually joined to. Leaving/forgetting a DM
    // does not reliably remove it from m.direct, so falling back to the
    // first listed id would reopen a dead (left/forgotten) room instead of
    // letting the caller create a fresh DM — which create_room then marks
    // in m.direct for future opens.
    room_ids
        .iter()
        .find(|id| client.get_room(id).is_some_and(|r| r.state() == matrix_sdk::RoomState::Joined))
        .cloned()
}

/// The `m.room.encryption` state event to seed a room's `initial_state` with
/// when creating it encrypted — the same recommended defaults the SDK's
/// `create_dm` uses.
fn encryption_initial_state(
) -> matrix_sdk::ruma::serde::Raw<matrix_sdk::ruma::events::AnyInitialStateEvent> {
    use matrix_sdk::ruma::events::{room::encryption::RoomEncryptionEventContent, InitialStateEvent};
    InitialStateEvent::with_empty_state_key(RoomEncryptionEventContent::with_recommended_defaults())
        .to_raw_any()
}
