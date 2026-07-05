//! MSC2545 "Image Packs" (custom emoji). `matrix-sdk` has no typed support
//! for this MSC — confirmed: `ruma-events` gates the typed content structs
//! behind an `unstable-msc2545` cargo feature that `matrix-sdk` doesn't
//! enable — so this fetches the raw account data / state events and
//! deserializes them against plain structs mirroring the MSC's JSON shape
//! directly (no dependency on ruma's `EventContent` machinery needed).
//!
//! Resolves three sources, matching how Element-family clients expose
//! custom emoji: the account's personal pack (`im.ponies.user_emotes`),
//! packs explicitly enabled via `im.ponies.emote_rooms`, and the
//! currently-open room's own default pack (`im.ponies.room_emotes` with an
//! empty state key — usable by anyone in the room without opting in).
//!
//! Every step logs at debug/warn so a run's log file shows exactly where
//! a homeserver's pack setup diverges from what this expects.

use std::collections::BTreeMap;

use matrix_sdk::ruma::api::client::state::get_state_events_for_key;
use matrix_sdk::ruma::events::{GlobalAccountDataEventType, StateEventType};
use matrix_sdk::{Client, Room};
use serde::Deserialize;

use crate::events::{CustomEmoji, EmojiPack};

#[derive(Debug, Deserialize)]
struct PackImage {
    url: String,
}

#[derive(Debug, Deserialize, Default)]
struct PackContent {
    #[serde(default)]
    images: BTreeMap<String, PackImage>,
    #[serde(default)]
    pack: Option<PackInfo>,
}

#[derive(Debug, Deserialize, Default)]
struct PackInfo {
    display_name: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct EmoteRoomsContent {
    #[serde(default)]
    rooms: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
}

fn pack_content_to_emoji_pack(fallback_name: String, content: PackContent) -> Option<EmojiPack> {
    if content.images.is_empty() {
        tracing::debug!(pack = %fallback_name, "pack content had no images");
        return None;
    }
    let name = content.pack.and_then(|p| p.display_name).unwrap_or(fallback_name);
    let emojis: Vec<CustomEmoji> = content
        .images
        .into_iter()
        .map(|(shortcode, image)| CustomEmoji { shortcode, mxc_url: image.url })
        .collect();
    tracing::info!(pack = %name, count = emojis.len(), "resolved custom emoji pack");
    Some(EmojiPack { name, emojis })
}

/// `Err(())` = transport failure (the data may well exist server-side);
/// `Ok(None)` = genuinely no pack. Deserialize failures are persistent data
/// problems, not transient, so they count as absence.
async fn fetch_user_pack(client: &Client) -> Result<Option<EmojiPack>, ()> {
    let event_type = GlobalAccountDataEventType::from("im.ponies.user_emotes");
    let raw = match client.account().account_data_raw(event_type).await {
        Ok(Some(raw)) => raw,
        Ok(None) => {
            tracing::debug!("no im.ponies.user_emotes account data set");
            return Ok(None);
        }
        Err(error) => {
            tracing::warn!(%error, "failed to fetch im.ponies.user_emotes");
            return Err(());
        }
    };
    let content: PackContent = match raw.deserialize_as() {
        Ok(content) => content,
        Err(error) => {
            tracing::warn!(%error, "failed to deserialize im.ponies.user_emotes content");
            return Ok(None);
        }
    };
    Ok(pack_content_to_emoji_pack("Personal".to_string(), content))
}

/// `Err(())` when the enabled-packs account data or any referenced pack
/// failed to fetch for transport reasons — a partial result would make the
/// caller wipe the missing packs from the UI.
async fn fetch_enabled_room_packs(client: &Client) -> Result<Vec<EmojiPack>, ()> {
    let event_type = GlobalAccountDataEventType::from("im.ponies.emote_rooms");
    let raw = match client.account().account_data_raw(event_type).await {
        Ok(Some(raw)) => raw,
        Ok(None) => {
            tracing::debug!("no im.ponies.emote_rooms account data set");
            return Ok(Vec::new());
        }
        Err(error) => {
            tracing::warn!(%error, "failed to fetch im.ponies.emote_rooms");
            return Err(());
        }
    };
    let content = match raw.deserialize_as::<EmoteRoomsContent>() {
        Ok(content) => content,
        Err(error) => {
            tracing::warn!(%error, "failed to deserialize im.ponies.emote_rooms content");
            return Ok(Vec::new());
        }
    };

    tracing::debug!(rooms = content.rooms.len(), "im.ponies.emote_rooms references this many rooms");

    let mut packs = Vec::new();
    for (room_id_str, state_keys) in content.rooms {
        let Ok(room_id) = matrix_sdk::ruma::RoomId::parse(&room_id_str) else {
            tracing::warn!(room_id = %room_id_str, "invalid room id in im.ponies.emote_rooms");
            continue;
        };
        let Some(room) = client.get_room(&room_id) else {
            tracing::warn!(room_id = %room_id_str, "room from im.ponies.emote_rooms not known locally");
            continue;
        };
        for state_key in state_keys.keys() {
            if let Some(pack) = fetch_room_pack(&room, state_key).await? {
                packs.push(pack);
            }
        }
    }
    Ok(packs)
}

/// A room's own pack at the given state key (empty string = the room's
/// default pack).
///
/// Fetched as a direct, uncached `/state/{eventType}/{stateKey}` request —
/// confirmed (by reading matrix-sdk-ui's `RoomListService` source) that
/// sliding sync's default `required_state` is a fixed list of well-known
/// event types that does not include arbitrary custom state, so
/// `im.ponies.room_emotes` is never synced into the local state store and
/// `Room::get_state_event` (a local-cache-only read) always misses even when
/// the event genuinely exists server-side.
/// `Err(())` = transport failure; `Ok(None)` = no such pack (404, empty, or
/// undeserializable content).
pub async fn fetch_room_pack(room: &Room, state_key: &str) -> Result<Option<EmojiPack>, ()> {
    let request = get_state_events_for_key::v3::Request::new(
        room.room_id().to_owned(),
        StateEventType::from("im.ponies.room_emotes"),
        state_key.to_owned(),
    );
    let raw = match room.client().send(request).await {
        Ok(response) => response.content,
        Err(error) => {
            let not_found =
                error.as_client_api_error().is_some_and(|e| e.status_code.as_u16() == 404);
            if not_found {
                tracing::debug!(
                    room_id = %room.room_id(),
                    state_key,
                    "no im.ponies.room_emotes state event at this state key"
                );
                return Ok(None);
            }
            tracing::warn!(room_id = %room.room_id(), %error, "failed to fetch im.ponies.room_emotes state event");
            return Err(());
        }
    };

    let content: PackContent = match raw.deserialize_as() {
        Ok(content) => content,
        Err(error) => {
            tracing::warn!(room_id = %room.room_id(), %error, "failed to deserialize im.ponies.room_emotes content");
            return Ok(None);
        }
    };

    let fallback_name = room.name().unwrap_or_else(|| room.room_id().to_string());
    Ok(pack_content_to_emoji_pack(fallback_name, content))
}

/// Every pack this account can currently use, or `None` if any source
/// failed for transport reasons — the caller should then keep whatever pack
/// set it already has rather than wiping custom emoji over a network blip.
pub async fn fetch_all(client: &Client, current_room: Option<&Room>) -> Option<Vec<EmojiPack>> {
    let mut packs = Vec::new();

    if let Some(pack) = fetch_user_pack(client).await.ok()? {
        packs.push(pack);
    }
    packs.extend(fetch_enabled_room_packs(client).await.ok()?);

    if let Some(room) = current_room {
        tracing::debug!(room_id = %room.room_id(), "checking room's own default emoji pack");
        if let Some(pack) = fetch_room_pack(room, "").await.ok()? {
            if !packs.iter().any(|p| p.name == pack.name) {
                packs.push(pack);
            }
        }
    }

    tracing::info!(total_packs = packs.len(), "custom emoji pack resolution complete");
    Some(packs)
}
