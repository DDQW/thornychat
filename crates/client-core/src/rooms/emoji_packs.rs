//! MSC2545 "Image Packs" (custom emoji). `matrix-sdk` has no typed support
//! for this MSC — confirmed: `ruma-events` gates the typed content structs
//! behind an `unstable-msc2545` cargo feature that `matrix-sdk` doesn't
//! enable — so this fetches the raw account data / state events and
//! deserializes them against plain structs mirroring the MSC's JSON shape
//! directly (no dependency on ruma's `EventContent` machinery needed).
//!
//! Resolves three sources, matching how Element-family clients expose
//! custom emoji: the account's personal pack (`im.ponies.user_emotes`),
//! packs explicitly enabled via `im.ponies.emote_rooms`, and every one of
//! the currently-open room's own packs (`im.ponies.room_emotes` under ANY
//! state key — usable by anyone in the room without opting in; Cinny
//! creates packs under named state keys, so probing only the `""` default
//! misses rooms that plainly have packs).
//!
//! Every step logs at debug/warn so a run's log file shows exactly where
//! a homeserver's pack setup diverges from what this expects.

use std::collections::BTreeMap;

use matrix_sdk::ruma::api::client::state::{get_state_event_for_key, get_state_events};
use matrix_sdk::ruma::events::{GlobalAccountDataEventType, StateEventType};
use matrix_sdk::{Client, Room};
use serde::Deserialize;

use crate::events::{CustomEmoji, EmojiPack};

#[derive(Debug, Deserialize)]
struct PackImage {
    url: String,
    /// MSC2545 per-image usage (`["emoticon"]`, `["sticker"]`, or both).
    /// Overrides the pack-level usage; absent (or empty) means "inherit".
    usage: Option<Vec<String>>,
    /// Optional per-image `info` block (dimensions etc.).
    info: Option<PackImageInfo>,
}

#[derive(Debug, Deserialize)]
struct PackImageInfo {
    #[serde(rename = "w")]
    width: Option<u32>,
    #[serde(rename = "h")]
    height: Option<u32>,
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
    /// MSC2545 pack-level usage, the default for every image that doesn't set
    /// its own. Absent (or empty) means images are usable as both.
    usage: Option<Vec<String>>,
}

/// Resolves an MSC2545 usage list to `(is_emoticon, is_sticker)`. Absent or
/// empty ⇒ both (the spec's default), otherwise exactly what's listed.
fn resolve_usage(usage: Option<&Vec<String>>) -> (bool, bool) {
    match usage {
        Some(list) if !list.is_empty() => (
            list.iter().any(|u| u == "emoticon"),
            list.iter().any(|u| u == "sticker"),
        ),
        _ => (true, true),
    }
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
    let PackContent { images, pack } = content;
    // Pack-level usage is the per-image default; read it before `pack` is
    // consumed for the display name.
    let pack_usage = pack.as_ref().and_then(|p| p.usage.clone());
    let name = pack.and_then(|p| p.display_name).unwrap_or(fallback_name);
    let emojis: Vec<CustomEmoji> = images
        .into_iter()
        .map(|(shortcode, image)| {
            // A non-empty image-level usage overrides the pack default; an
            // absent/empty one inherits it.
            let usage = match image.usage.as_ref() {
                Some(list) if !list.is_empty() => Some(list),
                _ => pack_usage.as_ref(),
            };
            let (is_emoticon, is_sticker) = resolve_usage(usage);
            let (width, height) =
                image.info.map(|i| (i.width, i.height)).unwrap_or((None, None));
            CustomEmoji { shortcode, mxc_url: image.url, is_emoticon, is_sticker, width, height }
        })
        .collect();
    // Full per-emoji manifest at info (not debug — the default log filter is
    // "info", so a debug! here would never reach the file the user copies
    // over when reporting a "wrong image shown" bug). Ground truth for what
    // *should* render, cross-referenced against the media-fetch/decode/
    // widget-mismatch logs below when diagnosing a report made on a machine
    // we can't inspect directly.
    for emoji in &emojis {
        tracing::info!(
            pack = %name,
            shortcode = %emoji.shortcode,
            url = %emoji.mxc_url,
            emoticon = emoji.is_emoticon,
            sticker = emoji.is_sticker,
            width = ?emoji.width,
            height = ?emoji.height,
            "emoji pack entry"
        );
    }
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
    let content: PackContent = match raw.deserialize_as_unchecked() {
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
    let content = match raw.deserialize_as_unchecked::<EmoteRoomsContent>() {
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
    let request = get_state_event_for_key::v3::Request::new(
        room.room_id().to_owned(),
        StateEventType::from("im.ponies.room_emotes"),
        state_key.to_owned(),
    );
    // 0.18/ruma-0.16: the get_state_event_for_key response now carries raw
    // JSON (`event_or_content`) rather than a typed `Raw<content>`; wrap it back
    // into a `Raw` of the content we're about to read.
    let raw: matrix_sdk::ruma::serde::Raw<PackContent> = match room.client().send(request).await {
        Ok(response) => matrix_sdk::ruma::serde::Raw::from_json(response.event_or_content),
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

    let content: PackContent = match raw.deserialize_as_unchecked() {
        Ok(content) => content,
        Err(error) => {
            tracing::warn!(room_id = %room.room_id(), %error, "failed to deserialize im.ponies.room_emotes content");
            return Ok(None);
        }
    };

    let fallback_name = room.name().unwrap_or_else(|| room.room_id().to_string());
    Ok(pack_content_to_emoji_pack(fallback_name, content))
}

/// Every `im.ponies.room_emotes` pack in the room, across ALL state keys.
/// MSC2545 allows a room any number of packs, one per state key, all usable
/// by any member without opting in — and Cinny files them under named state
/// keys, so the `""` default probed previously usually doesn't exist even
/// when the room plainly has packs (observed live: both HQ packs sit under
/// named keys while `/state/im.ponies.room_emotes/` 404s).
///
/// The C-S spec has no "all state keys of one type" endpoint, so this pulls
/// the full room state (member events included — the price of completeness,
/// paid once per room open like the rest of `fetch_all`) and filters by
/// type. `Err(())` = transport failure; per-event deserialize failures are
/// persistent data problems and just skip that pack.
pub async fn fetch_room_packs(room: &Room) -> Result<Vec<EmojiPack>, ()> {
    let request = get_state_events::v3::Request::new(room.room_id().to_owned());
    let response = match room.client().send(request).await {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(room_id = %room.room_id(), %error, "failed to fetch room state for emoji packs");
            return Err(());
        }
    };

    let mut packs = Vec::new();
    for raw in &response.room_state {
        match raw.get_field::<String>("type") {
            Ok(Some(event_type)) if event_type == "im.ponies.room_emotes" => {}
            _ => continue,
        }
        let state_key =
            raw.get_field::<String>("state_key").ok().flatten().unwrap_or_default();
        let content = match raw.get_field::<PackContent>("content") {
            Ok(Some(content)) => content,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!(
                    room_id = %room.room_id(),
                    state_key,
                    %error,
                    "failed to deserialize im.ponies.room_emotes content"
                );
                continue;
            }
        };
        // Nameless packs: the room name reads better than an empty string
        // for the default pack; a named state key is itself descriptive.
        let fallback_name = if state_key.is_empty() {
            room.name().unwrap_or_else(|| room.room_id().to_string())
        } else {
            state_key.clone()
        };
        if let Some(pack) = pack_content_to_emoji_pack(fallback_name, content) {
            packs.push(pack);
        }
    }
    // Server-defined state order isn't stable; sorted packs keep the picker
    // sections and first-wins shortcode collisions deterministic.
    packs.sort_by(|a, b| a.name.cmp(&b.name));
    tracing::debug!(
        room_id = %room.room_id(),
        state_events = response.room_state.len(),
        packs = packs.len(),
        "scanned full room state for emoji packs"
    );
    Ok(packs)
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
        tracing::debug!(room_id = %room.room_id(), "checking room's own emoji packs");
        for pack in fetch_room_packs(room).await.ok()? {
            // Rooms already enabled via `im.ponies.emote_rooms` show up a
            // second time here when they're the open room — same dedup as
            // before, first (enabled) copy wins.
            if !packs.iter().any(|p| p.name == pack.name) {
                packs.push(pack);
            }
        }
    }

    tracing::info!(total_packs = packs.len(), "custom emoji pack resolution complete");
    Some(packs)
}
