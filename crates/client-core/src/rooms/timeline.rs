//! Per-room `matrix_sdk_ui::timeline::Timeline` wrapper. Like the room list,
//! forwards a full snapshot on every update rather than translating
//! `matrix-sdk-ui`'s `VectorDiff` batches into an incremental UI-side patch
//! — correct and simple for the timeline sizes a chat client renders at
//! once; can be optimized into real diffing later without changing the
//! `ClientEvent` shape (`TimelineUpdated` already just carries the full
//! item list).

use std::sync::Arc;

use matrix_sdk::deserialized_responses::ShieldState as SdkShieldState;
use matrix_sdk::ruma::events::room::message::MessageType;
use matrix_sdk::ruma::events::room::MediaSource;
use matrix_sdk::ruma::{MilliSecondsSinceUnixEpoch, RoomId, UserId};
use matrix_sdk::{Client, Room};
use matrix_sdk_ui::timeline::{
    ReactionsByKeyBySender, RoomExt, Timeline, TimelineItem as SdkTimelineItem,
    TimelineItemContent as SdkTimelineItemContent, TimelineItemKind, VirtualTimelineItem,
};
use matrix_sdk_ui::timeline::{InReplyToDetails, MsgLikeKind, TimelineDetails};
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_stream::StreamExt;

use crate::events::{
    ClientEvent, ReactionGroup, ReplyPreview, TimelineItem, TimelineItemContent, TrustShield,
};

/// Opens (or re-opens) the timeline for `room_id_str` and spawns a task that
/// forwards a full snapshot whenever it changes. Returns the shared
/// `Timeline` handle too, so the caller can also call `.send()`/`.edit()`/
/// `.redact()`/`.send_attachment()`/`.toggle_reaction()` on the exact same
/// instance whose changes are already being forwarded — `Timeline` isn't
/// `Clone`, so it's wrapped in an `Arc` for both sides to hold onto.
/// Returns an error if the room isn't known locally or the timeline fails
/// to build.
pub async fn open(
    client: &Client,
    room_id_str: String,
    event_tx: mpsc::UnboundedSender<ClientEvent>,
) -> anyhow::Result<(Arc<Timeline>, JoinHandle<()>)> {
    let room_id = RoomId::parse(&room_id_str)?;
    let room = client
        .get_room(&room_id)
        .ok_or_else(|| anyhow::anyhow!("room not found: {room_id_str}"))?;
    let timeline = Arc::new(room.timeline().await?);
    let own_user_id = client.user_id().map(ToOwned::to_owned);

    // subscribe()'s initial vector is a skip-windowed view (at most the last
    // 20 cached items in SDK 0.13), but every later snapshot below reads
    // `items()` — the FULL unskipped list. Mixing the two made the item
    // count jump on the first diff after re-opening a room with cached
    // history ("content changed by itself"). Use items() for the initial
    // snapshot too; the subscription is only the wakeup stream.
    let (_, mut diff_stream) = timeline.subscribe().await;
    let initial_items = timeline.items().await;
    let mut reply_details_requested = std::collections::HashSet::new();
    request_missing_reply_details(&timeline, initial_items.iter(), &mut reply_details_requested);
    let _ = event_tx.send(ClientEvent::TimelineUpdated {
        room_id: room_id_str.clone(),
        items: convert_items(initial_items.iter(), own_user_id.as_deref()),
    });

    let forward_timeline = Arc::clone(&timeline);
    let handle = tokio::spawn(async move {
        let mut last_forwarded_newest: Option<String> = None;
        while diff_stream.next().await.is_some() {
            // Coalesce diff bursts (initial back-pagination, decryption
            // catch-up): each snapshot below is a full convert+clone pass
            // that the UI fully reprocesses, and only the last one of a
            // burst is ever visible. Diffs in a burst arrive milliseconds
            // apart — not necessarily pre-queued — so absorb until a short
            // quiet window passes (`items()` reads current state, skipping
            // intermediate wakeups is safe). Hard cap so a room with a
            // steady event drizzle still ships snapshots promptly.
            let absorb_started = std::time::Instant::now();
            while absorb_started.elapsed() < std::time::Duration::from_millis(400) {
                match tokio::time::timeout(
                    std::time::Duration::from_millis(80),
                    diff_stream.next(),
                )
                .await
                {
                    // Another batch landed inside the quiet window — keep
                    // absorbing.
                    Ok(Some(_)) => continue,
                    // Stream closed (room closing) — snapshot what we have;
                    // the outer loop exits on its next poll.
                    Ok(None) => break,
                    // The quiet window passed with no new diffs — the burst
                    // is over.
                    Err(_) => break,
                }
            }
            let items = forward_timeline.items().await;
            request_missing_reply_details(
                &forward_timeline,
                items.iter(),
                &mut reply_details_requested,
            );
            let converted = convert_items(items.iter(), own_user_id.as_deref());
            // Diagnostic: proves whether new events flow through this
            // forwarder at all (vs. the stream silently stalling), and at
            // what rate. Logged only when the newest event changes.
            let newest = converted.iter().rev().find_map(|item| item.event_id.clone());
            if newest != last_forwarded_newest {
                tracing::debug!(
                    len = converted.len(),
                    newest = newest.as_deref().unwrap_or("-"),
                    "timeline snapshot forwarded (new tail event)"
                );
                last_forwarded_newest = newest;
            }
            let event = ClientEvent::TimelineUpdated {
                room_id: room_id_str.clone(),
                items: converted,
            };
            if event_tx.send(event).is_err() {
                break;
            }
        }
    });

    Ok((timeline, handle))
}

/// The SDK only resolves a reply's quoted-event details when that event
/// happens to already be in the local timeline — otherwise they stay
/// `Unavailable` until `fetch_details_for_event` is explicitly called
/// (which fetches over the network and re-emits the item through the diff
/// stream). Kick that off for every unresolved quote, once per reply
/// event.
fn request_missing_reply_details<'a>(
    timeline: &Arc<Timeline>,
    items: impl Iterator<Item = &'a Arc<SdkTimelineItem>>,
    already_requested: &mut std::collections::HashSet<String>,
) {
    for item in items {
        let TimelineItemKind::Event(event) = item.kind() else { continue };
        let SdkTimelineItemContent::MsgLike(msg_like) = event.content() else { continue };
        let Some(details) = &msg_like.in_reply_to else { continue };
        if matches!(details.event, TimelineDetails::Unavailable) {
            let Some(reply_event_id) = event.event_id() else { continue };
            if !already_requested.insert(reply_event_id.to_string()) {
                continue;
            }
            let timeline = Arc::clone(timeline);
            let reply_event_id = reply_event_id.to_owned();
            tokio::spawn(async move {
                if let Err(error) = timeline.fetch_details_for_event(&reply_event_id).await {
                    tracing::debug!(%reply_event_id, %error, "failed to fetch reply details");
                }
            });
        }
    }
}

/// Forwards other users' typing state for a room as `ClientEvent::TypingUpdated`.
/// Owns the `EventHandlerDropGuard` internally so the handler is unregistered
/// automatically when this task is aborted (room closed) or ends.
pub fn spawn_typing_forwarder(
    room: Room,
    room_id: String,
    event_tx: mpsc::UnboundedSender<ClientEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let (_guard, mut rx) = room.subscribe_to_typing_notifications();
        loop {
            match rx.recv().await {
                Ok(user_ids) => {
                    let user_ids = user_ids.into_iter().map(|id| id.to_string()).collect();
                    let event = ClientEvent::TypingUpdated { room_id: room_id.clone(), user_ids };
                    if event_tx.send(event).is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

fn convert_items<'a>(
    items: impl Iterator<Item = &'a Arc<SdkTimelineItem>>,
    own_user_id: Option<&UserId>,
) -> Vec<TimelineItem> {
    items.filter_map(|item| convert_item(item, own_user_id)).collect()
}

fn convert_item(item: &SdkTimelineItem, own_user_id: Option<&UserId>) -> Option<TimelineItem> {
    match item.kind() {
        TimelineItemKind::Event(event) => {
            let (sender_display_name, sender_avatar_url) = match event.sender_profile() {
                TimelineDetails::Ready(profile) => (
                    profile.display_name.clone(),
                    profile.avatar_url.as_ref().map(|url| url.to_string()),
                ),
                _ => (None, None),
            };
            let (thread_root, thread_reply_count, in_reply_to) = match event.content() {
                SdkTimelineItemContent::MsgLike(msg_like) => (
                    msg_like.thread_root.as_ref().map(|id| id.to_string()),
                    msg_like.thread_summary.as_ref().map(|s| s.num_replies),
                    msg_like.in_reply_to.as_ref().map(convert_reply_preview),
                ),
                _ => (None, None, None),
            };

            Some(TimelineItem {
                event_id: event.event_id().map(|id| id.to_string()),
                sender: event.sender().to_string(),
                sender_display_name,
                sender_avatar_url,
                timestamp_ms: u64::from(event.timestamp().get()),
                content: convert_content(event.content()),
                // Lax mode matches what Element shows by default (doesn't
                // nag about every never-verified sender, still flags real
                // problems like a verification violation or sent-in-clear).
                shield: event.get_shield(false).and_then(convert_shield),
                reactions: convert_reactions(event.content().reactions(), own_user_id),
                thread_root,
                thread_reply_count,
                read_by: event.read_receipts().keys().map(|id| id.to_string()).collect(),
                in_reply_to,
            })
        }
        TimelineItemKind::Virtual(VirtualTimelineItem::DateDivider(ts)) => {
            Some(virtual_item(TimelineItemContent::DateDivider(format_date(*ts)), u64::from(ts.get())))
        }
        TimelineItemKind::Virtual(VirtualTimelineItem::ReadMarker) => {
            Some(virtual_item(TimelineItemContent::NewMessagesDivider, 0))
        }
        TimelineItemKind::Virtual(VirtualTimelineItem::TimelineStart) => None,
    }
}

fn virtual_item(content: TimelineItemContent, timestamp_ms: u64) -> TimelineItem {
    TimelineItem {
        event_id: None,
        sender: String::new(),
        sender_display_name: None,
        sender_avatar_url: None,
        timestamp_ms,
        content,
        shield: None,
        reactions: Vec::new(),
        thread_root: None,
        thread_reply_count: None,
        read_by: Vec::new(),
        in_reply_to: None,
    }
}

/// Flattens the SDK's replied-to details into the small preview the UI
/// quotes above a reply. Details may legitimately be unavailable (the SDK
/// lazily loads replied-to events); the UI shows just "↩" + ellipsis then.
fn convert_reply_preview(details: &InReplyToDetails) -> ReplyPreview {
    let (sender, snippet, image_url) = match &details.event {
        TimelineDetails::Ready(embedded) => {
            let sender = match &embedded.sender_profile {
                TimelineDetails::Ready(profile) => profile
                    .display_name
                    .clone()
                    .unwrap_or_else(|| embedded.sender.to_string()),
                _ => embedded.sender.to_string(),
            };
            let image_url = match convert_content(&embedded.content) {
                TimelineItemContent::Image { url, .. } => Some(url),
                _ => None,
            };
            (sender, summarize_content(&embedded.content), image_url)
        }
        _ => (String::new(), "…".to_string(), None),
    };
    ReplyPreview { event_id: details.event_id.to_string(), sender, snippet, image_url }
}

fn summarize_content(content: &SdkTimelineItemContent) -> String {
    const LIMIT: usize = 90;
    match convert_content(content) {
        TimelineItemContent::Text(body) => {
            let flat = body.replace('\n', " ");
            if flat.chars().count() > LIMIT {
                let cut: String = flat.chars().take(LIMIT).collect();
                format!("{cut}…")
            } else {
                flat
            }
        }
        TimelineItemContent::Image { .. } => "[image]".to_string(),
        TimelineItemContent::File { filename, .. } => format!("[file: {filename}]"),
        TimelineItemContent::Redacted => "(message removed)".to_string(),
        TimelineItemContent::DateDivider(_) | TimelineItemContent::NewMessagesDivider => {
            String::new()
        }
    }
}

fn convert_reactions(
    reactions: Option<&ReactionsByKeyBySender>,
    own_user_id: Option<&UserId>,
) -> Vec<ReactionGroup> {
    let Some(reactions) = reactions else { return Vec::new() };
    reactions
        .iter()
        .map(|(key, senders)| ReactionGroup {
            key: key.clone(),
            count: senders.len(),
            reacted_by_me: own_user_id.is_some_and(|uid| senders.contains_key(uid)),
            senders: senders.keys().map(|id| id.to_string()).collect(),
        })
        .collect()
}

fn convert_content(content: &SdkTimelineItemContent) -> TimelineItemContent {
    match content {
        SdkTimelineItemContent::MsgLike(msg_like) => match &msg_like.kind {
            MsgLikeKind::Message(message) => convert_message(message),
            MsgLikeKind::Redacted => TimelineItemContent::Redacted,
            MsgLikeKind::Sticker(_) => TimelineItemContent::Text("[sticker]".to_string()),
            MsgLikeKind::Poll(_) => TimelineItemContent::Text("[poll]".to_string()),
            MsgLikeKind::UnableToDecrypt(_) => {
                TimelineItemContent::Text("[unable to decrypt]".to_string())
            }
        },
        SdkTimelineItemContent::MembershipChange(_) => {
            TimelineItemContent::Text("(membership change)".to_string())
        }
        SdkTimelineItemContent::ProfileChange(_) => {
            TimelineItemContent::Text("(profile change)".to_string())
        }
        SdkTimelineItemContent::OtherState(_) => {
            TimelineItemContent::Text("(state event)".to_string())
        }
        SdkTimelineItemContent::FailedToParseMessageLike { .. }
        | SdkTimelineItemContent::FailedToParseState { .. } => {
            TimelineItemContent::Text("(unsupported event)".to_string())
        }
        SdkTimelineItemContent::CallInvite => TimelineItemContent::Text("(call invite)".to_string()),
        SdkTimelineItemContent::CallNotify => {
            TimelineItemContent::Text("(call notification)".to_string())
        }
    }
}

/// Images/files sent in encrypted rooms use `MediaSource::Encrypted`, whose
/// AES-CTR key material doesn't fit through the plain `url: String`
/// boundary this app uses for media — those degrade to a text placeholder.
/// Unencrypted-room media (and all custom emoji, which are never E2EE since
/// they're state events/account data, not message content) render for real.
fn convert_message(message: &matrix_sdk_ui::timeline::Message) -> TimelineItemContent {
    match message.msgtype() {
        MessageType::Image(image) => match &image.source {
            MediaSource::Plain(uri) => {
                // Sender-declared dimensions ride along so the UI can
                // reserve the display footprint before the bytes arrive.
                let (width, height) = image
                    .info
                    .as_ref()
                    .map(|info| {
                        (
                            info.width.and_then(|w| u32::try_from(w).ok()),
                            info.height.and_then(|h| u32::try_from(h).ok()),
                        )
                    })
                    .unwrap_or((None, None));
                TimelineItemContent::Image { url: uri.to_string(), caption: None, width, height }
            }
            MediaSource::Encrypted(_) => {
                TimelineItemContent::Text(format!("[encrypted image: {}]", message.body()))
            }
        },
        MessageType::File(file) => match &file.source {
            MediaSource::Plain(uri) => {
                TimelineItemContent::File { url: uri.to_string(), filename: message.body().to_string() }
            }
            MediaSource::Encrypted(_) => {
                TimelineItemContent::Text(format!("[encrypted file: {}]", message.body()))
            }
        },
        _ => TimelineItemContent::Text(message.body().to_string()),
    }
}

fn convert_shield(shield: SdkShieldState) -> Option<TrustShield> {
    match shield {
        SdkShieldState::Red { message, .. } => Some(TrustShield::Red(message.to_string())),
        SdkShieldState::Grey { message, .. } => Some(TrustShield::Grey(message.to_string())),
        SdkShieldState::None => None,
    }
}

/// Formats a millisecond Unix timestamp as `YYYY-MM-DD` without pulling in a
/// date/time crate, using Howard Hinnant's public-domain `civil_from_days`
/// algorithm.
fn format_date(ts: MilliSecondsSinceUnixEpoch) -> String {
    let millis = u64::from(ts.get()) as i64;
    let days = millis.div_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
