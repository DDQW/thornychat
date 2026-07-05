//! Room list, backed by `matrix_sdk::Client::rooms_stream()` rather than
//! `matrix-sdk-ui`'s `RoomListService` — that service's dynamic
//! filter/sorter API is built for sliding-sync-scale room lists with
//! pagination; for a first read-only room list, the plain client-level
//! snapshot + diff-as-wakeup is simpler and correct. Revisit if/when
//! filtering, sorting, or very large account sizes need it.

use matrix_sdk::{Client, Room, RoomMemberships};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_stream::StreamExt;

use crate::events::{ClientEvent, RoomMember, RoomSummary};

pub fn to_summary(room: &Room) -> RoomSummary {
    let room_id = room.room_id().to_string();
    RoomSummary {
        name: display_name(room).unwrap_or_else(|| room_id.clone()),
        room_id,
        topic: room.topic(),
        avatar_url: room.avatar_url().map(|url| url.to_string()),
        unread_count: room.num_unread_messages(),
        is_encrypted: room.encryption_state().is_encrypted(),
        is_space: room.is_space(),
        // direct_targets() clones the whole target set; only the count matters.
        is_dm: room.direct_targets_length() != 0,
        // Populated once space hierarchy tracking lands.
        parent_space_id: None,
        last_message_preview: None,
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
        Some(matrix_sdk::RoomDisplayName::Empty) | None => room.name(),
        Some(name) => Some(name.to_string()),
    }
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
/// the client's room vector changes; detaches on send failure (i.e. once
/// the UI side of the channel is gone).
pub fn spawn_forwarder(
    client: Client,
    event_tx: mpsc::UnboundedSender<ClientEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        // Joined rooms only: `Client::rooms()` also returns left/banned and
        // invited rooms, which would sit in the sidebar looking like normal
        // rooms (left rooms persist in the sqlite store across restarts).
        // Invited rooms can join the list once an invite accept/reject UI
        // exists (phase 6).
        let (initial, mut diffs) = client.rooms_stream();
        let summaries: Vec<RoomSummary> = initial
            .iter()
            .filter(|room| room.state() == matrix_sdk::RoomState::Joined)
            .map(to_summary)
            .collect();
        if event_tx.send(ClientEvent::RoomListUpdated(summaries)).is_err() {
            return;
        }

        while diffs.next().await.is_some() {
            // The diffs are used purely as wakeups; coalesce an already-queued
            // burst (startup discovers rooms one insert at a time) so N queued
            // diffs cost one rebuild instead of N full O(rooms) rebuilds.
            while let Ok(Some(_)) =
                tokio::time::timeout(std::time::Duration::ZERO, diffs.next()).await
            {}
            let summaries: Vec<RoomSummary> = client
                .rooms()
                .iter()
                .filter(|room| room.state() == matrix_sdk::RoomState::Joined)
                .map(to_summary)
                .collect();
            if event_tx.send(ClientEvent::RoomListUpdated(summaries)).is_err() {
                break;
            }
        }
    })
}
