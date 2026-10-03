//! Native notifications: the push-rule hits each sync produces become
//! [`ClientEvent::Notification`], which the shell shows as a toast (and
//! suppresses while its window has focus).
//!
//! The deciding is matrix-sdk's: it evaluates the account's push rules against
//! every timeline event a sync delivers — sliding sync included, and after
//! decrypting what it can — and hands the ones whose actions say "notify" to
//! registered notification handlers. So per-room modes (muted, mentions only),
//! keywords and the account-wide defaults are honoured with no logic here;
//! this module only filters out what a desktop toast shouldn't show and turns
//! the event into a title and a line of text.

use matrix_sdk::deserialized_responses::RawAnySyncOrStrippedTimelineEvent;
use matrix_sdk::ruma::events::room::message::{MessageType, Relation};
use matrix_sdk::ruma::events::{AnySyncMessageLikeEvent, AnySyncTimelineEvent, SyncMessageLikeEvent};
use matrix_sdk::ruma::push::Action;
use matrix_sdk::ruma::MilliSecondsSinceUnixEpoch;
use matrix_sdk::sync::Notification;
use matrix_sdk::{Client, Room};
use tokio::sync::mpsc;

use crate::events::{friendly_user_id, ClientEvent, NotificationEvent};

/// Longest message excerpt a toast carries, in characters.
const MAX_BODY_CHARS: usize = 200;

/// How old an event may be and still toast, in milliseconds. Sync also
/// delivers history (the latest messages of every room on startup, backfill
/// when a room is first subscribed), and a burst of toasts for things that
/// happened while the app was closed helps nobody.
const MAX_AGE_MS: u64 = 2 * 60 * 1000;

/// Slack for clock skew between this machine and the homeserver.
const CLOCK_SLACK_MS: u64 = 10 * 1000;

/// Starts turning push-rule hits into [`ClientEvent::Notification`]s. Events
/// from before this call never toast.
pub(crate) async fn register(client: &Client, event_tx: mpsc::UnboundedSender<ClientEvent>) {
    let started = now_ms();
    client
        .register_notification_handler(move |notification: Notification, room: Room, client: Client| {
            let event_tx = event_tx.clone();
            async move {
                if let Some(event) = describe(&notification, &room, &client, started).await {
                    let _ = event_tx.send(ClientEvent::Notification(event));
                }
            }
        })
        .await;
}

fn now_ms() -> u64 {
    MilliSecondsSinceUnixEpoch::now().get().into()
}

async fn describe(notification: &Notification, room: &Room, client: &Client, started: u64) -> Option<NotificationEvent> {
    if !notification.actions.iter().any(Action::should_notify) {
        return None;
    }
    // Invites arrive as stripped state; there is no room to open for them yet.
    let RawAnySyncOrStrippedTimelineEvent::Sync(raw) = &notification.event else { return None };
    let event = raw.deserialize().ok()?;
    if client.user_id() == Some(event.sender()) {
        return None;
    }
    let sent_at: u64 = event.origin_server_ts().get().into();
    if !is_fresh(sent_at, started, now_ms()) {
        return None;
    }

    let sender = event.sender().to_string();
    let sender_name = match room.get_member_no_sync(event.sender()).await {
        Ok(Some(member)) => friendly_user_id(member.name()).to_string(),
        _ => friendly_user_id(&sender).to_string(),
    };
    let text = match &event {
        AnySyncTimelineEvent::MessageLike(message) => message_text(message, &sender_name)?,
        AnySyncTimelineEvent::State(_) => return None,
    };
    let is_dm = room.direct_targets_length() != 0
        || crate::rooms::room_list::direct_room_ids(client).await.contains(room.room_id());
    let room_name = crate::rooms::room_list::display_name(room).unwrap_or_else(|| sender_name.clone());
    let (title, body) = compose(&room_name, &sender_name, &text, is_dm);
    Some(NotificationEvent { room_id: room.room_id().to_string(), sender, title, body })
}

/// Whether an event sent at `sent_at` is news: not from before this session
/// started, and not stale (both with some clock slack).
fn is_fresh(sent_at: u64, started: u64, now: u64) -> bool {
    sent_at + CLOCK_SLACK_MS >= started && sent_at + MAX_AGE_MS + CLOCK_SLACK_MS >= now
}

/// The line a toast shows for an event, or `None` for one that shouldn't toast
/// at all (an edit — the original already did — or a redaction).
fn message_text(event: &AnySyncMessageLikeEvent, sender_name: &str) -> Option<String> {
    let text = match event {
        AnySyncMessageLikeEvent::RoomMessage(SyncMessageLikeEvent::Original(message)) => {
            if matches!(message.content.relates_to, Some(Relation::Replacement(_))) {
                return None;
            }
            match &message.content.msgtype {
                MessageType::Text(text) => strip_reply_fallback(&text.body).to_string(),
                MessageType::Notice(notice) => strip_reply_fallback(&notice.body).to_string(),
                MessageType::Emote(emote) => format!("* {sender_name} {}", strip_reply_fallback(&emote.body)),
                MessageType::Image(_) => "sent an image".to_string(),
                MessageType::Video(_) => "sent a video".to_string(),
                MessageType::Audio(_) => "sent a voice or audio message".to_string(),
                MessageType::File(_) => "sent a file".to_string(),
                MessageType::Location(_) => "shared a location".to_string(),
                MessageType::VerificationRequest(_) => "wants to verify with you".to_string(),
                _ => "sent a message".to_string(),
            }
        }
        AnySyncMessageLikeEvent::Sticker(SyncMessageLikeEvent::Original(_)) => "sent a sticker".to_string(),
        AnySyncMessageLikeEvent::RoomEncrypted(SyncMessageLikeEvent::Original(_)) => {
            // Couldn't be decrypted at sync time (keys not here yet).
            "sent an encrypted message".to_string()
        }
        AnySyncMessageLikeEvent::CallInvite(_) => "is calling".to_string(),
        AnySyncMessageLikeEvent::UnstablePollStart(_) | AnySyncMessageLikeEvent::PollStart(_) => {
            "started a poll".to_string()
        }
        _ if event.is_redacted() => return None,
        _ => "sent a message".to_string(),
    };
    Some(shorten(text.trim(), MAX_BODY_CHARS))
}

/// The toast's two lines. In a DM the person is the title; in a room it's the
/// room, and the line says who wrote. Emotes already carry the name.
fn compose(room_name: &str, sender_name: &str, text: &str, is_dm: bool) -> (String, String) {
    if is_dm {
        (sender_name.to_string(), text.to_string())
    } else if text.starts_with("* ") {
        (room_name.to_string(), text.to_string())
    } else {
        (room_name.to_string(), format!("{sender_name}: {text}"))
    }
}

/// A body without the quoted parent a rich reply (or an older client) puts in
/// front of it: leading `> ` lines up to the first blank line.
fn strip_reply_fallback(body: &str) -> &str {
    if !body.starts_with("> ") {
        return body;
    }
    let mut rest = body;
    while let Some(line_end) = rest.find('\n') {
        let line = &rest[..line_end];
        rest = &rest[line_end + 1..];
        if line.trim().is_empty() {
            return rest;
        }
        if !line.starts_with('>') {
            return body;
        }
    }
    // Nothing but quote: show it rather than nothing.
    body
}

/// At most `max` characters, with an ellipsis when cut.
fn shorten(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", text[..cut].trim_end()),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_news_toasts() {
        let started = 1_000_000_000;
        let now = started + 60 * 60 * 1000;
        // Sent just now.
        assert!(is_fresh(now - 1000, started, now));
        // Before this session started: something that happened while the app was closed.
        assert!(!is_fresh(started - 60_000, started, started + 1000));
        // During the session but long ago: history delivered by a backfill.
        assert!(!is_fresh(now - 10 * 60 * 1000, started, now));
        // A server clock a few seconds behind ours still counts.
        assert!(is_fresh(started - 5000, started, started + 1000));
    }

    #[test]
    fn a_rich_reply_toasts_the_reply_not_the_quote() {
        let body = "> <@alice:example.org> are you coming?\n> it starts at 8\n\nyes, on my way";
        assert_eq!(strip_reply_fallback(body), "yes, on my way");
        // A message that merely starts with a quote and never ends it is kept whole.
        assert_eq!(strip_reply_fallback("> quoted\nnot a fallback"), "> quoted\nnot a fallback");
        assert_eq!(strip_reply_fallback("> only a quote"), "> only a quote");
        assert_eq!(strip_reply_fallback("plain"), "plain");
    }

    #[test]
    fn long_messages_are_cut_on_a_character_boundary() {
        assert_eq!(shorten("short", 10), "short");
        assert_eq!(shorten("abcdefghij", 10), "abcdefghij");
        assert_eq!(shorten("abcde fghijk", 6), "abcde…");
        // Multi-byte characters count once and are never split.
        assert_eq!(shorten("ééééé", 3), "ééé…");
    }

    #[test]
    fn a_dm_is_titled_by_the_person_and_a_room_by_its_name() {
        assert_eq!(compose("Alice", "Alice", "hi", true), ("Alice".to_string(), "hi".to_string()));
        assert_eq!(compose("dev", "Bob", "ship it", false), ("dev".to_string(), "Bob: ship it".to_string()));
        // An emote names its sender already.
        assert_eq!(compose("dev", "Bob", "* Bob waves", false), ("dev".to_string(), "* Bob waves".to_string()));
    }

    fn message(json: serde_json::Value) -> AnySyncMessageLikeEvent {
        serde_json::from_value(json).unwrap()
    }

    fn text_event(content: serde_json::Value) -> AnySyncMessageLikeEvent {
        message(serde_json::json!({
            "type": "m.room.message",
            "event_id": "$e:example.org",
            "sender": "@bob:example.org",
            "origin_server_ts": 1,
            "content": content,
        }))
    }

    #[test]
    fn messages_become_one_readable_line() {
        let text = text_event(serde_json::json!({ "msgtype": "m.text", "body": "hello there" }));
        assert_eq!(message_text(&text, "Bob").as_deref(), Some("hello there"));
        let emote = text_event(serde_json::json!({ "msgtype": "m.emote", "body": "waves" }));
        assert_eq!(message_text(&emote, "Bob").as_deref(), Some("* Bob waves"));
        let image = text_event(serde_json::json!({ "msgtype": "m.image", "body": "cat.jpg", "url": "mxc://example.org/abc" }));
        assert_eq!(message_text(&image, "Bob").as_deref(), Some("sent an image"));
    }

    #[test]
    fn an_edit_does_not_toast_again() {
        let edit = text_event(serde_json::json!({
            "msgtype": "m.text",
            "body": "* fixed typo",
            "m.new_content": { "msgtype": "m.text", "body": "fixed typo" },
            "m.relates_to": { "rel_type": "m.replace", "event_id": "$original:example.org" },
        }));
        assert_eq!(message_text(&edit, "Bob"), None);
    }

    #[test]
    fn an_undecryptable_message_says_so_without_content() {
        let encrypted = message(serde_json::json!({
            "type": "m.room.encrypted",
            "event_id": "$e:example.org",
            "sender": "@bob:example.org",
            "origin_server_ts": 1,
            "content": {
                "algorithm": "m.megolm.v1.aes-sha2",
                "ciphertext": "AwgAEnAC",
                "device_id": "DEVICE",
                "sender_key": "key",
                "session_id": "session",
            },
        }));
        assert_eq!(message_text(&encrypted, "Bob").as_deref(), Some("sent an encrypted message"));
    }
}
