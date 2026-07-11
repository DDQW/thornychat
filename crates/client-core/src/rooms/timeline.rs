//! Per-room `matrix_sdk_ui::timeline::Timeline` wrapper. Consumes the SDK's
//! *windowed* timeline subscription and forwards its `VectorDiff` batches to
//! the UI as [`TimelineDiff`]s (see [`open`]) — only the items that actually
//! changed cross the boundary each sync tick, and a room opens with just the
//! newest ~20 items, revealing older ones as the user paginates. The one
//! wrinkle the translator handles: the SDK's content-less `TimelineStart`
//! marker converts to nothing, so diff indices are renumbered into the UI
//! list's space (see `kept` / `translate_diff`).

use std::sync::Arc;

use matrix_sdk::ruma::events::room::message::MessageType;
use matrix_sdk::ruma::events::room::MediaSource;
use matrix_sdk::ruma::events::sticker::{StickerEventContent, StickerMediaSource};
use matrix_sdk::ruma::events::StateEventContentChange;
use matrix_sdk::ruma::{MilliSecondsSinceUnixEpoch, RoomId, UserId};
use matrix_sdk::{Client, Room};
use matrix_sdk_ui::timeline::{
    MembershipChange, ReactionsByKeyBySender, RoomExt, RoomMembershipChange, Timeline,
    TimelineItem as SdkTimelineItem, TimelineItemContent as SdkTimelineItemContent, TimelineItemKind,
    VirtualTimelineItem,
};
use matrix_sdk_ui::timeline::{
    EventSendState, InReplyToDetails, MsgLikeKind, TimelineDetails, TimelineEventShieldState,
};
use matrix_sdk_ui::eyeball_im::VectorDiff;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_stream::StreamExt;

use crate::events::{
    friendly_user_id, ClientEvent, ReactionGroup, ReplyPreview, SendFailure, TimelineDiff,
    TimelineItem, TimelineItemContent, TrustShield,
};

/// Opens (or re-opens) the timeline for `room_id_str` and spawns a task that
/// forwards the SDK's windowed `VectorDiff` stream as [`TimelineDiff`]
/// batches. Returns the shared `Timeline` handle too, so the caller can also
/// call `.send()`/`.edit()`/
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

    // Consume the SDK's *windowed* timeline subscription: `subscribe()` yields
    // an initial vector (the newest ~20 cached items) plus a stream of
    // `VectorDiff` batches that keep that window — and its growth under
    // back-pagination — in sync. Forward those as `TimelineDiff`s rather than
    // re-reading and re-converting the whole list each tick.
    //
    // `convert_item` drops the SDK's content-less `TimelineStart` marker, which
    // would misalign every later indexed diff; `kept` records which SDK
    // positions survive conversion so `translate_diff` can renumber indices
    // into the UI list's space.
    let (initial, mut diff_stream) = timeline.subscribe().await;
    let mut reply_details_requested = std::collections::HashSet::new();
    request_missing_reply_details(&timeline, initial.iter(), &mut reply_details_requested);

    let mut kept: Vec<bool> = Vec::with_capacity(initial.len());
    let mut initial_items: Vec<TimelineItem> = Vec::new();
    for item in initial.iter() {
        match convert_item(item, own_user_id.as_deref()) {
            Some(converted) => {
                kept.push(true);
                initial_items.push(converted);
            }
            None => kept.push(false),
        }
    }
    let _ = event_tx.send(ClientEvent::TimelineDiffs {
        room_id: room_id_str.clone(),
        diffs: vec![TimelineDiff::Reset(initial_items)],
    });

    let forward_timeline = Arc::clone(&timeline);
    let handle = tokio::spawn(async move {
        while let Some(batch) = diff_stream.next().await {
            // Reply previews load lazily; kick off a fetch for any quoted event
            // an item in this batch still hasn't resolved (over the raw SDK
            // items, before conversion drops anything).
            for vdiff in &batch {
                match vdiff {
                    VectorDiff::Append { values } | VectorDiff::Reset { values } => {
                        request_missing_reply_details(
                            &forward_timeline,
                            values.iter(),
                            &mut reply_details_requested,
                        );
                    }
                    VectorDiff::PushFront { value }
                    | VectorDiff::PushBack { value }
                    | VectorDiff::Insert { value, .. }
                    | VectorDiff::Set { value, .. } => {
                        request_missing_reply_details(
                            &forward_timeline,
                            std::iter::once(value),
                            &mut reply_details_requested,
                        );
                    }
                    _ => {}
                }
            }

            let diffs: Vec<TimelineDiff> = batch
                .into_iter()
                .filter_map(|vdiff| translate_diff(vdiff, own_user_id.as_deref(), &mut kept))
                .collect();
            if diffs.is_empty() {
                continue;
            }
            if event_tx
                .send(ClientEvent::TimelineDiffs { room_id: room_id_str.clone(), diffs })
                .is_err()
            {
                break;
            }
        }
    });

    Ok((timeline, handle))
}

/// Translates one SDK `VectorDiff` — over the *windowed* timeline, whose
/// indices include the content-less `TimelineStart` marker — into a UI
/// [`TimelineDiff`] whose indices are in the converted list's space. `kept`
/// mirrors, per SDK position, whether that item survives conversion, and is
/// updated in place. Returns `None` when the diff only touched a dropped item
/// (so it has no UI effect).
fn translate_diff(
    diff: VectorDiff<Arc<SdkTimelineItem>>,
    own_user_id: Option<&UserId>,
    kept: &mut Vec<bool>,
) -> Option<TimelineDiff> {
    // UI index for SDK index `i`: the count of kept positions strictly before it.
    fn ui_index(kept: &[bool], i: usize) -> usize {
        kept[..i.min(kept.len())].iter().filter(|k| **k).count()
    }
    match diff {
        VectorDiff::Append { values } => {
            let mut items = Vec::new();
            for value in &values {
                match convert_item(value, own_user_id) {
                    Some(item) => {
                        kept.push(true);
                        items.push(item);
                    }
                    None => kept.push(false),
                }
            }
            (!items.is_empty()).then_some(TimelineDiff::Append(items))
        }
        VectorDiff::Clear => {
            kept.clear();
            Some(TimelineDiff::Clear)
        }
        VectorDiff::PushFront { value } => {
            let converted = convert_item(&value, own_user_id);
            kept.insert(0, converted.is_some());
            converted.map(TimelineDiff::PushFront)
        }
        VectorDiff::PushBack { value } => {
            let converted = convert_item(&value, own_user_id);
            kept.push(converted.is_some());
            converted.map(TimelineDiff::PushBack)
        }
        VectorDiff::PopFront => {
            let was_kept = if kept.is_empty() { false } else { kept.remove(0) };
            was_kept.then_some(TimelineDiff::PopFront)
        }
        VectorDiff::PopBack => {
            let was_kept = kept.pop().unwrap_or(false);
            was_kept.then_some(TimelineDiff::PopBack)
        }
        VectorDiff::Insert { index, value } => {
            let index = index.min(kept.len());
            let ui = ui_index(kept, index);
            let converted = convert_item(&value, own_user_id);
            kept.insert(index, converted.is_some());
            converted.map(|item| TimelineDiff::Insert { index: ui, item })
        }
        VectorDiff::Set { index, value } => {
            if index >= kept.len() {
                return None;
            }
            let ui = ui_index(kept, index);
            match (kept[index], convert_item(&value, own_user_id)) {
                (true, Some(item)) => Some(TimelineDiff::Set { index: ui, item }),
                (true, None) => {
                    kept[index] = false;
                    Some(TimelineDiff::Remove { index: ui })
                }
                (false, Some(item)) => {
                    kept[index] = true;
                    Some(TimelineDiff::Insert { index: ui, item })
                }
                (false, None) => None,
            }
        }
        VectorDiff::Remove { index } => {
            if index >= kept.len() {
                return None;
            }
            let ui = ui_index(kept, index);
            let was_kept = kept.remove(index);
            was_kept.then_some(TimelineDiff::Remove { index: ui })
        }
        VectorDiff::Truncate { length } => {
            let ui_len = ui_index(kept, length);
            kept.truncate(length);
            Some(TimelineDiff::Truncate { length: ui_len })
        }
        VectorDiff::Reset { values } => {
            kept.clear();
            let mut items = Vec::new();
            for value in &values {
                match convert_item(value, own_user_id) {
                    Some(item) => {
                        kept.push(true);
                        items.push(item);
                    }
                    None => kept.push(false),
                }
            }
            Some(TimelineDiff::Reset(items))
        }
    }
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
            // Built before the struct literal: it borrows `sender_display_name`,
            // which the struct then moves into its own field.
            let content = convert_content(
                event.content(),
                event.sender().as_str(),
                sender_display_name.as_deref(),
            );

            Some(TimelineItem {
                event_id: event.event_id().map(|id| id.to_string()),
                sender: event.sender().to_string(),
                sender_display_name,
                sender_avatar_url,
                timestamp_ms: u64::from(event.timestamp().get()),
                content,
                // Lax mode matches what Element shows by default (doesn't
                // nag about every never-verified sender, still flags real
                // problems like a verification violation or sent-in-clear).
                shield: convert_shield(event.get_shield(false)),
                reactions: convert_reactions(event.content().reactions(), own_user_id),
                thread_root,
                thread_reply_count,
                read_by: event.read_receipts().keys().map(|id| id.to_string()).collect(),
                in_reply_to,
                edited: content_is_edited(event.content()),
                send_failed: event.send_state().and_then(|state| match state {
                    EventSendState::SendingFailed { error, is_recoverable } => {
                        Some(SendFailure { is_recoverable: *is_recoverable, error: error.to_string() })
                    }
                    _ => None,
                }),
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
        edited: false,
        send_failed: None,
    }
}

/// Whether an `m.replace` aggregation has been applied to this content.
/// Only real messages can carry the flag — stickers, polls, and state
/// events never show an "(edited)" tag here.
fn content_is_edited(content: &SdkTimelineItemContent) -> bool {
    match content {
        SdkTimelineItemContent::MsgLike(msg_like) => match &msg_like.kind {
            MsgLikeKind::Message(message) => message.is_edited(),
            _ => false,
        },
        _ => false,
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
                    .unwrap_or_else(|| friendly_user_id(embedded.sender.as_str()).to_string()),
                _ => friendly_user_id(embedded.sender.as_str()).to_string(),
            };
            let image_url = match convert_content(&embedded.content, embedded.sender.as_str(), None) {
                TimelineItemContent::Image { url, .. }
                | TimelineItemContent::Sticker { url, .. } => Some(url),
                _ => None,
            };
            (sender, summarize_content(&embedded.content, embedded.sender.as_str()), image_url)
        }
        _ => (String::new(), "…".to_string(), None),
    };
    ReplyPreview { event_id: details.event_id.to_string(), sender, snippet, image_url }
}

fn summarize_content(content: &SdkTimelineItemContent, sender: &str) -> String {
    const LIMIT: usize = 90;
    match convert_content(content, sender, None) {
        TimelineItemContent::Text(body) | TimelineItemContent::Emote(body) => {
            let flat = body.replace('\n', " ");
            if flat.chars().count() > LIMIT {
                let cut: String = flat.chars().take(LIMIT).collect();
                format!("{cut}…")
            } else {
                flat
            }
        }
        TimelineItemContent::Image { caption: Some(caption), .. } => {
            format!("[image: {caption}]")
        }
        TimelineItemContent::Image { caption: None, .. } => "[image]".to_string(),
        TimelineItemContent::Sticker { .. } => "[sticker]".to_string(),
        TimelineItemContent::File { filename, .. } => format!("[file: {filename}]"),
        TimelineItemContent::Redacted => "(message removed)".to_string(),
        TimelineItemContent::MembershipChange(desc) => desc,
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

fn convert_content(
    content: &SdkTimelineItemContent,
    sender: &str,
    sender_display_name: Option<&str>,
) -> TimelineItemContent {
    match content {
        SdkTimelineItemContent::MsgLike(msg_like) => match &msg_like.kind {
            MsgLikeKind::Message(message) => convert_message(message),
            MsgLikeKind::Redacted => TimelineItemContent::Redacted,
            MsgLikeKind::Sticker(sticker) => convert_sticker(sticker.content()),
            MsgLikeKind::Poll(_) => TimelineItemContent::Text("[poll]".to_string()),
            MsgLikeKind::UnableToDecrypt(_) => {
                TimelineItemContent::Text("[unable to decrypt]".to_string())
            }
            MsgLikeKind::Other(_) => TimelineItemContent::Text("(unsupported event)".to_string()),
            MsgLikeKind::LiveLocation(_) => {
                TimelineItemContent::Text("(live location)".to_string())
            }
        },
        SdkTimelineItemContent::MembershipChange(change) => TimelineItemContent::MembershipChange(
            describe_membership_change(change, sender, sender_display_name),
        ),
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
        // 0.18 renamed CallNotify → RtcNotification.
        SdkTimelineItemContent::RtcNotification { .. } => {
            TimelineItemContent::Text("(call notification)".to_string())
        }
    }
}

/// Renders a membership change into a human sentence for the timeline. The
/// *actor* (who performed the action) is the event sender; the *target*
/// (whose membership changed) is `change.user_id()`. For self-performed
/// changes (join/leave/knock…) the two are the same person, so only the
/// target is named. Kick/ban reasons are appended when present; invite
/// reasons are deliberately *not* shown — ruma's own guidance is that an
/// invite `reason` is an unsolicited-message/abuse vector.
fn describe_membership_change(
    change: &RoomMembershipChange,
    actor: &str,
    actor_display_name: Option<&str>,
) -> String {
    let target = change
        .display_name()
        .unwrap_or_else(|| friendly_user_id(change.user_id().as_str()).to_string());
    let actor_name = actor_display_name
        .map(str::to_string)
        .unwrap_or_else(|| friendly_user_id(actor).to_string());
    let reason = || match change.content() {
        StateEventContentChange::Original { content, .. } => content.reason.clone(),
        StateEventContentChange::Redacted(_) => None,
    };
    match change.change() {
        Some(MembershipChange::Joined) => format!("{target} joined the room"),
        Some(MembershipChange::Left) => format!("{target} left the room"),
        Some(MembershipChange::Banned) => {
            with_reason(format!("{actor_name} banned {target}"), reason())
        }
        Some(MembershipChange::Unbanned) => format!("{actor_name} unbanned {target}"),
        Some(MembershipChange::Kicked) => {
            with_reason(format!("{actor_name} kicked {target}"), reason())
        }
        Some(MembershipChange::KickedAndBanned) => {
            with_reason(format!("{actor_name} kicked and banned {target}"), reason())
        }
        Some(MembershipChange::Invited) => format!("{actor_name} invited {target}"),
        Some(MembershipChange::InvitationAccepted) => format!("{target} accepted the invite"),
        Some(MembershipChange::InvitationRejected) => format!("{target} rejected the invite"),
        Some(MembershipChange::InvitationRevoked) => {
            format!("{actor_name} revoked {target}'s invite")
        }
        Some(MembershipChange::Knocked) => format!("{target} requested to join"),
        Some(MembershipChange::KnockAccepted) => format!("{actor_name} let {target} in"),
        Some(MembershipChange::KnockRetracted) => {
            format!("{target} withdrew their request to join")
        }
        Some(MembershipChange::KnockDenied) => {
            format!("{actor_name} rejected {target}'s request to join")
        }
        // None / Error / NotImplemented (and any future variant): no reliable
        // actor/verb, so just note that the target's membership changed.
        _ => format!("{target}'s membership changed"),
    }
}

/// Appends a `: reason` tail to a membership sentence when the moderation
/// event carried a non-empty reason.
fn with_reason(sentence: String, reason: Option<String>) -> String {
    match reason {
        Some(reason) if !reason.trim().is_empty() => format!("{sentence}: {reason}"),
        _ => sentence,
    }
}

/// Stickers (`m.sticker`) carry their image directly on the event instead
/// of nested in a `msgtype`. They get their own `Sticker` content (rather
/// than being flattened into `Image`) so the UI can render them at sticker
/// size and harvest them into the grow-with-use collection, keeping the
/// `mxc://` URL + body needed to resend one.
fn convert_sticker(content: &StickerEventContent) -> TimelineItemContent {
    match &content.source {
        StickerMediaSource::Plain(uri) => {
            let width = content.info.width.and_then(|w| u32::try_from(w).ok());
            let height = content.info.height.and_then(|h| u32::try_from(h).ok());
            TimelineItemContent::Sticker {
                url: uri.to_string(),
                body: content.body.clone(),
                width,
                height,
            }
        }
        StickerMediaSource::Encrypted(_) => {
            TimelineItemContent::Text(format!("[encrypted sticker: {}]", content.body))
        }
        // `StickerMediaSource` is `#[non_exhaustive]`; no other variant
        // exists today, so this can't currently be reached.
        _ => TimelineItemContent::Text(format!("[sticker: {}]", content.body)),
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
                // MSC2530: with a `filename` field present, a differing
                // `body` is a caption (ruma's `caption()` encodes that rule).
                TimelineItemContent::Image {
                    url: uri.to_string(),
                    caption: image.caption().map(str::to_owned),
                    width,
                    height,
                }
            }
            MediaSource::Encrypted(_) => {
                TimelineItemContent::Text(format!("[encrypted image: {}]", message.body()))
            }
        },
        MessageType::File(file) => match &file.source {
            MediaSource::Plain(uri) => {
                // `filename()` (not `body()`): for captioned files the body
                // IS the caption and the real name sits in `filename`.
                TimelineItemContent::File {
                    url: uri.to_string(),
                    filename: file.filename().to_string(),
                    caption: file.caption().map(str::to_owned),
                }
            }
            MediaSource::Encrypted(_) => {
                TimelineItemContent::Text(format!("[encrypted file: {}]", message.body()))
            }
        },
        // `/me` actions: the body carries only the action text; the UI
        // prepends the sender's name.
        MessageType::Emote(emote) => TimelineItemContent::Emote(emote.body.clone()),
        _ => TimelineItemContent::Text(message.body().to_string()),
    }
}

fn convert_shield(shield: TimelineEventShieldState) -> Option<TrustShield> {
    // 0.18 exposes a machine-readable `code` instead of a human message; surface
    // the code name so the shield tooltip still says why it's shown.
    match shield {
        TimelineEventShieldState::Red { code } => Some(TrustShield::Red(format!("{code:?}"))),
        TimelineEventShieldState::Grey { code } => Some(TrustShield::Grey(format!("{code:?}"))),
        TimelineEventShieldState::None => None,
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
