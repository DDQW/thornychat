//! Room list, backed by `matrix_sdk::Client::rooms_stream()` rather than
//! `matrix-sdk-ui`'s `RoomListService` — that service's dynamic
//! filter/sorter API is built for sliding-sync-scale room lists with
//! pagination; for a first read-only room list, the plain client-level
//! snapshot + diff-as-wakeup is simpler and correct. Revisit if/when
//! filtering, sorting, or very large account sizes need it.

use std::collections::HashSet;

use matrix_sdk::ruma::OwnedRoomId;
use matrix_sdk::{Client, Room, RoomMemberships};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_stream::StreamExt;

use crate::events::{ClientEvent, RoomMember, RoomSummary};

pub fn to_summary(room: &Room, dm_rooms: &HashSet<OwnedRoomId>) -> RoomSummary {
    let room_id = room.room_id().to_string();
    // Classify DMs from the account's global `m.direct` set: on sliding sync
    // the per-room `direct_targets()` is frequently empty even for real DMs
    // (the account-data → per-room propagation lags), so a DM would otherwise
    // show up as a plain room. Keep `direct_targets()` as a secondary signal.
    let is_dm = dm_rooms.contains(room.room_id()) || room.direct_targets_length() != 0;
    RoomSummary {
        name: display_name(room).unwrap_or_else(|| room_id.clone()),
        room_id,
        topic: room.topic(),
        avatar_url: avatar_url(room, is_dm),
        unread_count: room.num_unread_messages(),
        is_encrypted: room.encryption_state().is_encrypted(),
        is_space: room.is_space(),
        is_dm,
        last_message_preview: None,
    }
}

/// Every room id the account marks as a direct message, from the global
/// `m.direct` account data. More reliable than per-room `direct_targets()`,
/// which sliding sync doesn't consistently populate. Empty if the account
/// data hasn't synced yet.
pub async fn direct_room_ids(client: &Client) -> HashSet<OwnedRoomId> {
    use matrix_sdk::ruma::events::direct::DirectEventContent;
    match client.account().account_data::<DirectEventContent>().await {
        Ok(Some(raw)) => match raw.deserialize() {
            Ok(content) => content.0.into_values().flatten().collect(),
            Err(_) => HashSet::new(),
        },
        _ => HashSet::new(),
    }
}

/// The SDK's *computed* display name (heroes-based, Element-style — DM
/// rooms with no explicit `m.room.name` get the other member's name
/// instead of a raw room id). `cached_display_name` is a plain lock read,
/// recomputed by the SDK after every sync, so this stays cheap and fresh
/// without an async call here. Early in the very first sync the cache can
/// be empty or legitimately `Empty` — fall back to `room.name()`/room id
/// and let the next room-list emission correct it.
fn display_name(room: &Room) -> Option<String> {
    match room.cached_display_name() {
        Some(matrix_sdk::RoomDisplayName::Empty) | None => {
            // No SDK-computed name yet (e.g. a just-created DM before its
            // first sync): prefer an explicit room name, then the Matrix
            // "heroes" — the other participants — so a DM shows the person's
            // name and initials instead of flashing the raw room id.
            room.name().or_else(|| hero_names(room))
        }
        Some(name) => Some(name.to_string()),
    }
}

/// Comma-joined hero display names (falling back to a hero's user id), the
/// standard Matrix mechanism for naming DMs and unnamed rooms after their
/// members. `None` when the room has no heroes cached yet.
fn hero_names(room: &Room) -> Option<String> {
    let names: Vec<String> = room
        .heroes()
        .into_iter()
        .map(|hero| hero.display_name.unwrap_or_else(|| hero.user_id.to_string()))
        .collect();
    (!names.is_empty()).then(|| names.join(", "))
}

/// The room's own `m.room.avatar` if set. For DMs *only*, fall back to a
/// hero's avatar — a 1:1 (or small direct) chat with no room avatar should
/// show the other participant's picture, the way Element does. Group rooms
/// deliberately get no fallback: an unset room avatar means initials, not a
/// borrowed member face (a group with no logo showing some random member's
/// photo as its icon is wrong, not a missing-avatar bug).
fn avatar_url(room: &Room, is_dm: bool) -> Option<String> {
    if let Some(url) = room.avatar_url() {
        return Some(url.to_string());
    }
    if is_dm {
        return room.heroes().into_iter().find_map(|hero| hero.avatar_url.map(|url| url.to_string()));
    }
    None
}

/// Lists a room's joined members (member list panel, @mention
/// autocomplete, name colors). Uses the syncing variant: lazy member
/// loading means the local store only knows recently-active senders (a
/// fraction of the real roster), so this fetches the full `/members` list
/// from the server when needed. Runs in a spawned task on room open — the
/// UI shows what it has until the full list lands.
pub async fn fetch_members(room: &Room) -> Vec<RoomMember> {
    match room.members(RoomMemberships::JOIN).await {
        Ok(members) => members
            .into_iter()
            .map(|m| RoomMember {
                user_id: m.user_id().to_string(),
                display_name: m.name().to_string(),
                avatar_url: m.avatar_url().map(|url| url.to_string()),
                power_level: m.power_level(),
            })
            .collect(),
        Err(error) => {
            tracing::warn!(%error, "failed to list room members");
            Vec::new()
        }
    }
}

/// Spawns a task that emits the room list on startup and again every time
/// the client's room vector changes or any room's info notably updates;
/// detaches on send failure (i.e. once the UI side of the channel is gone).
pub fn spawn_forwarder(
    client: Client,
    event_tx: mpsc::UnboundedSender<ClientEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        // Second wakeup source, subscribed before the initial snapshot so
        // nothing lands between the two unobserved: `rooms_stream` diffs only
        // fire when a room is created or forgotten, while a room's *info*
        // (display name, unread counts, the m.room.create that makes
        // `is_space` true) mutates in place afterwards with no diff. Most
        // visibly, a space delivered via sliding sync room subscription is
        // inserted bare and gets its state applied moments later — rebuilding
        // only on diffs left it rendered as a nameless regular room forever.
        let mut info_updates = client.room_info_notable_update_receiver();

        // Joined rooms only: `Client::rooms()` also returns left/banned and
        // invited rooms, which would sit in the sidebar looking like normal
        // rooms (left rooms persist in the sqlite store across restarts).
        // Invited rooms can join the list once an invite accept/reject UI
        // exists (phase 6).
        let (initial, mut diffs) = client.rooms_stream();
        let dm_rooms = direct_room_ids(&client).await;
        let summaries: Vec<RoomSummary> = initial
            .iter()
            .filter(|room| room.state() == matrix_sdk::RoomState::Joined)
            .map(|room| to_summary(room, &dm_rooms))
            .collect();
        if event_tx.send(ClientEvent::RoomListUpdated(summaries)).is_err() {
            return;
        }

        loop {
            // Both streams are pure wakeups — the rebuild below re-reads
            // everything from the client regardless of what was received.
            tokio::select! {
                diff = diffs.next() => {
                    if diff.is_none() {
                        break;
                    }
                }
                update = info_updates.recv() => {
                    match update {
                        // Lagged only means wakeups were missed, and the
                        // rebuild reads current state anyway.
                        Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                        // The sender lives inside the client — closed means
                        // shutdown, same as the diff stream ending.
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }

            // Debounce, then drain both sources: sync responses touch many
            // rooms at once and receipts arrive in bursts, so N queued
            // wakeups cost one O(rooms) rebuild instead of N.
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            while let Ok(Some(_)) =
                tokio::time::timeout(std::time::Duration::ZERO, diffs.next()).await
            {}
            loop {
                match info_updates.try_recv() {
                    Ok(_) | Err(broadcast::error::TryRecvError::Lagged(_)) => {}
                    // Empty (or Closed, which the next recv() handles).
                    Err(_) => break,
                }
            }

            // Re-read `m.direct` each rebuild so a DM created/marked after
            // startup starts classifying correctly.
            let dm_rooms = direct_room_ids(&client).await;
            let summaries: Vec<RoomSummary> = client
                .rooms()
                .iter()
                .filter(|room| room.state() == matrix_sdk::RoomState::Joined)
                .map(|room| to_summary(room, &dm_rooms))
                .collect();
            if event_tx.send(ClientEvent::RoomListUpdated(summaries)).is_err() {
                break;
            }
        }
    })
}
