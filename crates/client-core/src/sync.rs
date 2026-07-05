//! The sync worker: a single long-lived tokio task that owns the
//! `matrix_sdk::Client` and a `matrix_sdk_ui::sync_service::SyncService`,
//! bridging them to the UI over two `mpsc` channels.
//!
//! This is the architectural linchpin described in the project plan: iced's
//! `Task<Message>` is one-shot and the wrong shape for a persistent service,
//! so the worker lives entirely outside iced's executor and communicates
//! purely through `ClientCommand` (in) / `ClientEvent` (out).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use matrix_sdk::attachment::AttachmentConfig;
use matrix_sdk::encryption::verification::VerificationRequest;
use matrix_sdk::notification_settings::RoomNotificationMode as SdkNotificationMode;
use matrix_sdk::room::edit::EditedContent;
use matrix_sdk::ruma::api::client::receipt::create_receipt::v3::ReceiptType;
use matrix_sdk::ruma::events::key::verification::request::ToDeviceKeyVerificationRequestEventContent;
use matrix_sdk::ruma::events::room::message::{
    MessageType, OriginalSyncRoomMessageEvent, RoomMessageEventContent,
    RoomMessageEventContentWithoutRelation,
};
use matrix_sdk::ruma::events::{Mentions, ToDeviceEvent};
use matrix_sdk::ruma::{EventId, OwnedUserId, RoomId, UserId};
use matrix_sdk::Client;
use matrix_sdk_ui::sync_service::{self, SyncService};
use matrix_sdk_ui::timeline::{AttachmentSource, Timeline, TimelineEventItemId};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::commands::{ClientCommand, RequestId};
use crate::events::{ClientEvent, SyncState};
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
    let sync_service = SyncService::builder(client.clone()).build().await?;
    let mut state_stream = sync_service.state();
    let _ = event_tx.send(ClientEvent::SyncStateChanged(SyncState::Connecting));
    sync_service.start().await;

    // Detached: run for the process lifetime, clean themselves up once
    // `event_tx` is gone (send fails, their loops break).
    let _room_list_handle = room_list::spawn_forwarder(client.clone(), event_tx.clone());
    let _notification_watcher = crate::push::spawn_watcher(client.clone(), event_tx.clone());
    let call_manager = crate::calls::CallManager::spawn(client.clone(), event_tx.clone());

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
        verification: None,
        pending_cross_signing_session,
    };

    loop {
        tokio::select! {
            Some(state) = state_stream.next() => {
                let mapped = match state {
                    sync_service::State::Idle => SyncState::Connecting,
                    sync_service::State::Running => SyncState::Syncing,
                    sync_service::State::Terminated => SyncState::Offline,
                    sync_service::State::Error => {
                        SyncState::Error("sync service reported an error state".into())
                    }
                    sync_service::State::Offline => SyncState::Offline,
                };
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

            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else {
                    // UI dropped its sender; shut down cleanly.
                    break;
                };
                handle_command(&client, &sync_service, &call_manager, &mut worker_state, cmd, &event_tx, &media_cache_dir).await;
            }
        }
    }

    worker_state.open_rooms.clear();
    call_manager.leave_all().await;
    sync_service.stop().await;

    Ok(())
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

async fn handle_command(
    client: &Client,
    sync_service: &SyncService,
    call_manager: &std::sync::Arc<crate::calls::CallManager>,
    worker_state: &mut WorkerState,
    cmd: ClientCommand,
    event_tx: &mpsc::UnboundedSender<ClientEvent>,
    media_cache_dir: &std::path::Path,
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
            // updates append instead of resetting. This SDK version has no
            // unsubscribe; subscriptions simply last the session, like
            // Element X.
            if let Ok(parsed_room_id) = RoomId::parse(&room_id) {
                sync_service.room_list_service().subscribe_to_rooms(&[&parsed_room_id]).await;
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
                    let call_room = room.clone();
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
        }

        ClientCommand::OpenDirectMessage { user_id, request_id } => {
            let Ok(parsed_user_id) = UserId::parse(&user_id) else {
                fail(event_tx, request_id, "invalid user id");
                return;
            };
            let client = client.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                if let Some(room) = client.get_dm_room(&parsed_user_id) {
                    let _ = event_tx.send(ClientEvent::DirectMessageReady {
                        room_id: room.room_id().to_string(),
                    });
                    succeed(&event_tx, request_id);
                    return;
                }
                match client.create_dm(&parsed_user_id).await {
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

        ClientCommand::SendMessage { room_id, body, mentioned_user_ids, reply_to_event_id, request_id } => {
            let Some(handles) = worker_state.open_rooms.get(&room_id) else {
                fail(event_tx, request_id, "room is not open");
                return;
            };

            let mut content = RoomMessageEventContent::text_markdown(body);
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
                let reply = matrix_sdk::room::reply::Reply {
                    event_id: reply_event_id,
                    // Follow the quoted message's threading (reply in the
                    // main timeline stays in the main timeline).
                    enforce_thread: matrix_sdk::room::reply::EnforceThread::MaybeThreaded,
                };
                match handles.timeline.send_reply(content.into(), reply).await {
                    Ok(()) => succeed(event_tx, request_id),
                    Err(error) => fail(event_tx, request_id, &error.to_string()),
                }
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

        ClientCommand::SendAttachment { room_id, filename, bytes, mime, request_id } => {
            let Some(handles) = worker_state.open_rooms.get(&room_id) else {
                fail(event_tx, request_id, "room is not open");
                return;
            };

            let mime_type: mime::Mime =
                mime.parse().unwrap_or(mime::APPLICATION_OCTET_STREAM);
            let source = AttachmentSource::Data { bytes, filename };
            let config = AttachmentConfig::new();

            // send_attachment performs the full media upload before returning
            // (no send queue for attachments in SDK 0.13) — awaiting it inline
            // would freeze every other command (incl. room switches) for the
            // whole upload on a slow uplink.
            let timeline = handles.timeline.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                match timeline.send_attachment(source, mime_type, config).await {
                    Ok(()) => succeed(&event_tx, request_id),
                    Err(error) => fail(&event_tx, request_id, &error.to_string()),
                }
            });
        }

        ClientCommand::SetTyping { room_id, typing } => {
            if let Some(handles) = worker_state.open_rooms.get(&room_id) {
                let room = handles.timeline.room().clone();
                tokio::spawn(async move {
                    let _ = room.typing_notice(typing).await;
                });
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
                Ok(()) => succeed(event_tx, request_id),
                Err(error) => fail(event_tx, request_id, &error.to_string()),
            }
        }

        ClientCommand::MarkRoomRead { room_id } => {
            if let Some(handles) = worker_state.open_rooms.get(&room_id) {
                let timeline = Arc::clone(&handles.timeline);
                tokio::spawn(async move {
                    // Both receipts, like Element: the public read receipt
                    // (what others see, clears the unread badge) and the
                    // private fully-read marker (what positions the "new
                    // messages" divider next time the room opens).
                    // `mark_as_read` returns whether it actually sent —
                    // `false` means the SDK decided the receipt wasn't
                    // needed (e.g. it believes the marker is already at or
                    // past the latest event), which is invisible without
                    // logging and exactly what a stuck "new messages"
                    // divider would look like.
                    match timeline.mark_as_read(ReceiptType::Read).await {
                        Ok(sent) => tracing::debug!(sent, "read receipt"),
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

        ClientCommand::AddKeywordHighlight { keyword, request_id } => {
            let client = client.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                let settings = client.notification_settings().await;
                match settings.add_keyword(keyword).await {
                    Ok(()) => {
                        let keywords = settings.enabled_keywords().await.into_iter().collect();
                        let _ = event_tx.send(ClientEvent::KeywordHighlightsUpdated(keywords));
                        succeed(&event_tx, request_id);
                    }
                    Err(error) => fail(&event_tx, request_id, &error.to_string()),
                }
            });
        }

        ClientCommand::RemoveKeywordHighlight { keyword, request_id } => {
            let client = client.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                let settings = client.notification_settings().await;
                match settings.remove_keyword(&keyword).await {
                    Ok(()) => {
                        let keywords = settings.enabled_keywords().await.into_iter().collect();
                        let _ = event_tx.send(ClientEvent::KeywordHighlightsUpdated(keywords));
                        succeed(&event_tx, request_id);
                    }
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
    let _ = event_tx.send(ClientEvent::CommandFailed { request_id, error: error.to_string() });
}
