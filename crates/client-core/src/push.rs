//! Read-side of notification settings: pushes the account's per-room
//! notification overrides to the UI, both at startup and whenever the
//! account's push rules change (the SDK's `subscribe_to_changes` fires on
//! every `m.push_rules` account-data event from sync, which covers changes
//! made locally *and* from other devices).
//!
//! The write-side (set/clear mode) lives in `sync.rs`'s command handlers.
//! Push-rule *evaluation* (feeding `ClientEvent::Notification` for native
//! toasts) is still Phase 7.

use matrix_sdk::notification_settings::{
    IsEncrypted, IsOneToOne, NotificationSettings, RoomNotificationMode,
};
use matrix_sdk::ruma::RoomId;
use matrix_sdk::Client;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::events::{ClientEvent, NotificationMode};

pub(crate) fn from_sdk_mode(mode: RoomNotificationMode) -> NotificationMode {
    match mode {
        RoomNotificationMode::AllMessages => NotificationMode::AllMessages,
        RoomNotificationMode::MentionsAndKeywordsOnly => NotificationMode::MentionsAndKeywordsOnly,
        RoomNotificationMode::Mute => NotificationMode::Mute,
    }
}

async fn snapshot_modes(settings: &NotificationSettings) -> Vec<(String, NotificationMode)> {
    let mut modes = Vec::new();
    for room_id_str in settings.get_rooms_with_user_defined_rules(Some(true)).await {
        let Ok(room_id) = RoomId::parse(&room_id_str) else { continue };
        if let Some(mode) = settings.get_user_defined_room_notification_mode(&room_id).await {
            modes.push((room_id_str, from_sdk_mode(mode)));
        }
    }
    modes
}

/// Reads the account-wide defaults for direct messages and group rooms.
/// The SDK tracks these per encrypted/unencrypted variant too, but this app
/// only ever writes both together (see `sync.rs`'s `SetDefaultNotificationMode`
/// handler), so the unencrypted variant is representative of what's shown.
async fn snapshot_defaults(settings: &NotificationSettings) -> (NotificationMode, NotificationMode) {
    let direct_messages = settings
        .get_default_room_notification_mode(IsEncrypted::No, IsOneToOne::Yes)
        .await;
    let group_chats = settings
        .get_default_room_notification_mode(IsEncrypted::No, IsOneToOne::No)
        .await;
    (from_sdk_mode(direct_messages), from_sdk_mode(group_chats))
}

async fn emit_snapshot(
    settings: &NotificationSettings,
    event_tx: &mpsc::UnboundedSender<ClientEvent>,
) -> bool {
    let modes = snapshot_modes(settings).await;
    let (direct_messages, group_chats) = snapshot_defaults(settings).await;
    event_tx.send(ClientEvent::RoomNotificationModesUpdated(modes)).is_ok()
        && event_tx
            .send(ClientEvent::DefaultNotificationModesUpdated { direct_messages, group_chats })
            .is_ok()
}

/// Spawns the settings watcher. The `NotificationSettings` instance must be
/// the one held here for its whole life: its change subscription is backed
/// by a sync event handler that is deregistered when the instance drops.
pub fn spawn_watcher(
    client: Client,
    event_tx: mpsc::UnboundedSender<ClientEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let settings = client.notification_settings().await;
        // Subscribe BEFORE the first snapshot: broadcast messages published
        // while there is no receiver are dropped, and the first sync's
        // m.push_rules account data can land exactly during that window —
        // subscribing first queues it and triggers a fresh snapshot after.
        let mut changes = settings.subscribe_to_changes();
        if !emit_snapshot(&settings, &event_tx).await {
            return;
        }
        // Lagged just means we missed intermediate ticks — the next
        // snapshot read is always of current state anyway. Closed ends the
        // loop.
        while let Ok(()) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) =
            changes.recv().await
        {
            if !emit_snapshot(&settings, &event_tx).await {
                break;
            }
        }
    })
}
