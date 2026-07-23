//! MSC2545 "Image Packs" (custom emoji). `matrix-sdk` has no typed support
//! for this MSC — confirmed: `ruma-events` gates the typed content structs
//! behind an `unstable-msc2545` cargo feature that `matrix-sdk` doesn't
//! enable — so this fetches the raw account data / state events and
//! deserializes them against plain structs mirroring the MSC's JSON shape
//! directly (no dependency on ruma's `EventContent` machinery needed).
//!
//! Resolves all four MSC2545 pack sources, matching how Element-family
//! clients expose custom emoji and stickers:
//!   1. the account's personal pack (`im.ponies.user_emotes`, account data);
//!   2. packs the account has enabled everywhere (`im.ponies.emote_rooms`,
//!      account data — a map of room → the state keys whose packs to pull);
//!   3. every one of the currently-open room's own packs
//!      (`im.ponies.room_emotes` under ANY state key — usable by anyone in
//!      the room without opting in; Cinny files packs under named state
//!      keys, so probing only the `""` default misses rooms that plainly
//!      have packs);
//!   4. the packs of that room's parent space(s), walked recursively up the
//!      space chain, for every space the account has joined — the MSC's
//!      "suggest image packs of a room's canonical space… recursively on
//!      canonical spaces" rule. This is the one source that isn't a single
//!      flat fetch, and the one most easily missed: a community's shared
//!      pack usually lives on the space, not on each of its rooms.
//!
//! The stable event names (`m.room.image_pack`, `m.image_pack.rooms`) aren't
//! deployed anywhere yet — every server writes the `im.ponies.*` unstable
//! names — but room-pack scanning also accepts `m.room.image_pack`, so a
//! forward-looking client's packs won't be invisible either.
//!
//! Every step logs at debug/warn so a run's log file shows exactly where
//! a homeserver's pack setup diverges from what this expects.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use matrix_sdk::ruma::api::client::state::{get_state_event_for_key, get_state_events};
use matrix_sdk::ruma::events::{GlobalAccountDataEventType, StateEventType};
use matrix_sdk::ruma::{OwnedRoomId, RoomId};
use matrix_sdk::{Client, Room, RoomState};
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

/// Just enough of an `m.space.parent` event to walk the space chain: the
/// servers that can be used to reach the parent (a link with none is not a
/// valid parent, per MSC1772, and is ignored) and whether it's the room's
/// canonical parent (canonical parents are suggested first).
#[derive(Debug, Deserialize, Default)]
struct SpaceParentContent {
    #[serde(default)]
    via: Vec<String>,
    #[serde(default)]
    canonical: bool,
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
    tracing::info!(raw = %raw.json(), "DIAGNOSTIC: im.ponies.emote_rooms raw account data");
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

/// One full-state fetch of `room`, mined for everything MSC2545 resolution
/// needs from a room in a single request: its own image packs (every
/// `im.ponies.room_emotes` — and stable `m.room.image_pack` — state event,
/// under ANY state key) and the ids of its parent spaces (`m.space.parent`
/// links), canonical parent first.
///
/// MSC2545 allows a room any number of packs, one per state key, all usable
/// by any member without opting in — and Cinny files them under named state
/// keys, so the `""` default usually doesn't exist even when the room plainly
/// has packs (observed live: both HQ packs sit under named keys while
/// `/state/im.ponies.room_emotes/` 404s). The C-S spec has no "all state keys
/// of one type" endpoint, so this pulls the full room state (member events
/// included — the price of completeness, paid once per room the walk touches)
/// and filters by type. Fetched as a direct, uncached `/state` request for the
/// reason spelled out on [`fetch_room_pack`]: sliding sync's `required_state`
/// never includes these custom/relationship events, so a local-cache read
/// misses them even when they exist server-side.
///
/// `Err(())` = transport failure; per-event deserialize failures are
/// persistent data problems and just skip that one event.
async fn scan_room_state(room: &Room) -> Result<(Vec<EmojiPack>, Vec<OwnedRoomId>), ()> {
    let request = get_state_events::v3::Request::new(room.room_id().to_owned());
    let response = match room.client().send(request).await {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(room_id = %room.room_id(), %error, "failed to fetch room state for emoji packs");
            return Err(());
        }
    };

    let mut packs = Vec::new();
    // (parent room id, is_canonical); canonical parents float to the front
    // afterwards so the walk suggests the primary space's packs first.
    let mut parents: Vec<(OwnedRoomId, bool)> = Vec::new();
    for raw in &response.room_state {
        let event_type = match raw.get_field::<String>("type") {
            Ok(Some(event_type)) => event_type,
            _ => continue,
        };
        match event_type.as_str() {
            // The stable name is accepted alongside the unstable one so a pack
            // written by a forward-looking client isn't skipped.
            "im.ponies.room_emotes" | "m.room.image_pack" => {
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
                            "failed to deserialize room image pack content"
                        );
                        continue;
                    }
                };
                // Nameless packs: the room name reads better than an empty
                // string for the default pack; a named state key is itself
                // descriptive.
                let fallback_name = if state_key.is_empty() {
                    room.name().unwrap_or_else(|| room.room_id().to_string())
                } else {
                    state_key.clone()
                };
                if let Some(pack) = pack_content_to_emoji_pack(fallback_name, content) {
                    packs.push(pack);
                }
            }
            "m.space.parent" => {
                let Some(state_key) = raw.get_field::<String>("state_key").ok().flatten() else {
                    continue;
                };
                let Ok(parent_id) = RoomId::parse(&state_key) else {
                    tracing::warn!(room_id = %room.room_id(), state_key, "m.space.parent with an unparseable parent id");
                    continue;
                };
                let content = raw
                    .get_field::<SpaceParentContent>("content")
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                // MSC1772: a parent link with no `via` servers is not a link.
                if content.via.is_empty() {
                    continue;
                }
                parents.push((parent_id, content.canonical));
            }
            _ => continue,
        }
    }

    // Server-defined state order isn't stable; sorted packs keep the picker
    // sections and first-wins shortcode collisions deterministic.
    packs.sort_by(|a, b| a.name.cmp(&b.name));
    // Stable partition: canonical parents first, insertion order kept otherwise.
    parents.sort_by_key(|(_, canonical)| !canonical);
    let parents: Vec<OwnedRoomId> = parents.into_iter().map(|(id, _)| id).collect();

    tracing::debug!(
        room_id = %room.room_id(),
        state_events = response.room_state.len(),
        packs = packs.len(),
        parent_spaces = parents.len(),
        "scanned full room state for emoji packs and parent spaces"
    );
    Ok((packs, parents))
}

/// Every image pack in the room, across ALL state keys — the packs half of
/// [`scan_room_state`] (see it for why the whole room state has to be pulled).
/// `Err(())` = transport failure.
pub async fn fetch_room_packs(room: &Room) -> Result<Vec<EmojiPack>, ()> {
    Ok(scan_room_state(room).await?.0)
}

/// A room's parent-space chain can loop or fan out; cap how many space rooms
/// one resolution will pull full state for. Real nesting is 1–3 deep, so this
/// only ever bites a pathological or cyclic space graph.
const MAX_SPACE_PACK_ROOMS: usize = 16;

/// MSC2545 space packs: the image packs of the open room's parent space(s),
/// walked recursively up the space chain, for every space the account has
/// actually joined. A room names its parents with `m.space.parent` state
/// events (canonical parent first, per the MSC's "canonical space" wording —
/// but a room can belong to several spaces and all of them are suggested);
/// each parent is itself a room whose `im.ponies.room_emotes` packs its
/// members may use, and that space can have its own parent, so the walk
/// recurses. This is where a community's shared pack almost always lives.
///
/// `seed_parents` are the origin room's parents, already mined from its state
/// by [`scan_room_state`], so the open room isn't fetched a second time.
/// `origin` seeds the visited set so a space that lists the open room as a
/// child can't send the walk straight back into it (the parent/child links
/// form a 2-cycle by design).
///
/// Best-effort by design: this is bonus discovery layered on top of the three
/// flat sources, so a single space whose state won't load is logged and
/// skipped rather than failing the whole resolution and wiping the packs
/// already gathered. Only joined spaces contribute — the MSC gates on "if the
/// user is also in that space", and unjoined space state may not be readable
/// at all.
async fn fetch_space_packs(
    client: &Client,
    seed_parents: Vec<OwnedRoomId>,
    origin: &RoomId,
) -> Vec<EmojiPack> {
    let mut packs = Vec::new();
    let mut visited: BTreeSet<OwnedRoomId> = BTreeSet::new();
    visited.insert(origin.to_owned());
    let mut queue: VecDeque<OwnedRoomId> = seed_parents.into_iter().collect();
    let mut scanned = 0usize;

    while let Some(space_id) = queue.pop_front() {
        // Diamond/cycle guard: never scan the same space twice.
        if !visited.insert(space_id.clone()) {
            continue;
        }
        if scanned >= MAX_SPACE_PACK_ROOMS {
            tracing::warn!(
                origin = %origin,
                cap = MAX_SPACE_PACK_ROOMS,
                "parent-space pack walk hit its room cap; stopping (cyclic or unusually deep space graph)"
            );
            break;
        }
        let Some(space) = client.get_room(&space_id) else {
            tracing::debug!(space_id = %space_id, "parent space not in the local store; skipping its packs");
            continue;
        };
        // "if the user is also in that space" — and unjoined state isn't
        // reliably readable anyway.
        if space.state() != RoomState::Joined {
            tracing::debug!(space_id = %space_id, "not joined to parent space; skipping its packs");
            continue;
        }
        let (space_packs, grandparents) = match scan_room_state(&space).await {
            Ok(result) => result,
            Err(()) => {
                tracing::warn!(space_id = %space_id, "failed to fetch parent-space state; skipping its packs");
                continue;
            }
        };
        scanned += 1;
        tracing::debug!(space_id = %space_id, packs = space_packs.len(), "resolved parent-space emoji packs");
        packs.extend(space_packs);
        // Climb to the next level: this space's own parents.
        for parent in grandparents {
            if !visited.contains(&parent) {
                queue.push_back(parent);
            }
        }
    }
    packs
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
        tracing::debug!(room_id = %room.room_id(), "checking the open room's own packs and its parent-space packs");
        // The open room's own state is primary: a transport failure here keeps
        // the existing pack set (`.ok()?`) rather than wiping custom emoji over
        // a blip. Its parent-space links ride along in the same fetch, so the
        // space walk below never re-fetches this room.
        let (room_packs, parent_spaces) = scan_room_state(room).await.ok()?;
        for pack in room_packs {
            // Rooms already enabled via `im.ponies.emote_rooms` show up a
            // second time here when they're the open room — same dedup as
            // before, first (enabled) copy wins.
            if !packs.iter().any(|p| p.name == pack.name) {
                packs.push(pack);
            }
        }
        // MSC2545 source #4: the open room's canonical space chain. Best-effort
        // (never fails the whole resolution), and the packs are deduped by name
        // against everything above so a space already reached via
        // `im.ponies.emote_rooms` doesn't double up.
        for pack in fetch_space_packs(client, parent_spaces, room.room_id()).await {
            if !packs.iter().any(|p| p.name == pack.name) {
                packs.push(pack);
            }
        }
    }

    tracing::info!(total_packs = packs.len(), "custom emoji pack resolution complete");
    Some(packs)
}
