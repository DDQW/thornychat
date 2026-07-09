//! Space hierarchy ("explore space"): lists a space's children through the
//! client-server `/hierarchy` API (MSC2946), the standard Matrix space
//! directory — it returns rooms the account hasn't joined, which is the
//! point. Fetched one level deep per request; the UI drills into subspaces
//! with follow-up fetches instead of walking the whole tree up front.
//!
//! Also owns joined-space *discovery* ([`spawn_joined_spaces_subscriber`]):
//! sliding sync filters `m.space` rooms out entirely, so the spaces the
//! account belongs to have to be found over REST and reported to the sync
//! worker, which subscribes to them (force-feeding them into the store) so the
//! sidebar can list them.

use std::collections::HashMap;
use std::sync::Arc;

use matrix_sdk::ruma::api::client::membership::joined_rooms;
use matrix_sdk::ruma::api::client::space::get_hierarchy;
use matrix_sdk::ruma::api::client::state::get_state_event_for_key;
use matrix_sdk::ruma::events::room::create::RoomCreateEventContent;
use matrix_sdk::ruma::events::StateEventType;
use matrix_sdk::ruma::room::RoomType;
use matrix_sdk::ruma::{OwnedRoomId, RoomId, UInt};
use matrix_sdk::{Client, RoomState};
use matrix_sdk_ui::room_list_service::{RoomListLoadingState, RoomListService};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::events::{ClientEvent, SpaceChildSummary, SpaceJoinRule};

/// Children per page — small enough for a snappy first paint, paged via the
/// returned `next_batch` token.
const PAGE_LIMIT: u32 = 50;

/// Page cap for the sidebar-grouping child sweep ([`fetch_all_child_ids`]) —
/// 10 pages = 500 children, far beyond any space whose rooms should nest in
/// a sidebar. Truncation just means the overflow rooms stay in the flat
/// "Rooms" section.
const CHILD_ID_PAGE_CAP: u32 = 10;

/// Fetches one page of `space_id`'s direct children. Returns the children
/// plus the pagination token for the next page (`None` = last page).
pub async fn fetch_children(
    client: &Client,
    space_id: &RoomId,
    from: Option<String>,
) -> Result<(Vec<SpaceChildSummary>, Option<String>), matrix_sdk::HttpError> {
    let mut request = get_hierarchy::v1::Request::new(space_id.to_owned());
    request.from = from;
    request.limit = Some(UInt::from(PAGE_LIMIT));
    // Direct children only; subspaces are explored by drilling in.
    request.max_depth = Some(UInt::from(1u32));
    let response = client.send(request).await?;

    // Candidate join servers per child, harvested from the space's stripped
    // `m.space.child` events — a child's own chunk doesn't carry them.
    let mut via: HashMap<OwnedRoomId, Vec<String>> = HashMap::new();
    for chunk in &response.rooms {
        for raw in &chunk.children_state {
            if let Ok(event) = raw.deserialize() {
                via.insert(
                    event.state_key,
                    event.content.via.iter().map(ToString::to_string).collect(),
                );
            }
        }
    }

    let children = response
        .rooms
        .iter()
        // The queried space itself is returned as a chunk of page one; only
        // its children belong in the listing.
        // 0.18/ruma-0.16 flattened the per-room fields into a `summary`.
        .filter(|chunk| chunk.summary.room_id != space_id)
        .map(|chunk| SpaceChildSummary {
            room_id: chunk.summary.room_id.to_string(),
            name: chunk.summary.name.clone(),
            topic: chunk.summary.topic.clone(),
            canonical_alias: chunk.summary.canonical_alias.as_ref().map(ToString::to_string),
            avatar_url: chunk.summary.avatar_url.as_ref().map(ToString::to_string),
            num_joined_members: chunk.summary.num_joined_members.into(),
            is_space: chunk.summary.room_type == Some(matrix_sdk::ruma::room::RoomType::Space),
            joined: client
                .get_room(&chunk.summary.room_id)
                .is_some_and(|room| room.state() == matrix_sdk::RoomState::Joined),
            join_rule: map_join_rule(&chunk.summary.join_rule),
            via: via.get(&chunk.summary.room_id).cloned().unwrap_or_default(),
        })
        .collect();

    Ok((children, response.next_batch))
}

/// Every child room id of `space_id` (rooms and subspaces, joined or not),
/// walking the hierarchy one level deep across pages. Powers the sidebar's
/// space grouping, so it only needs ids — the explorer keeps using the
/// richer per-page [`fetch_children`].
pub async fn fetch_all_child_ids(
    client: &Client,
    space_id: &RoomId,
) -> Result<Vec<String>, matrix_sdk::HttpError> {
    let mut ids = Vec::new();
    let mut from = None;
    for page in 0.. {
        if page == CHILD_ID_PAGE_CAP {
            tracing::warn!(%space_id, "space child sweep truncated at {CHILD_ID_PAGE_CAP} pages");
            break;
        }
        let (children, next_batch) = fetch_children(client, space_id, from).await?;
        ids.extend(children.into_iter().map(|child| child.room_id));
        match next_batch {
            Some(token) => from = Some(token),
            None => break,
        }
    }
    Ok(ids)
}

/// Fetches `space_id`'s child ids and announces them to the UI; failures are
/// logged and swallowed (the sidebar just keeps that space's rooms in the
/// flat list). Shared by the startup sweep and the post-join hook.
pub async fn emit_space_children(
    client: &Client,
    space_id: &RoomId,
    event_tx: &mpsc::UnboundedSender<ClientEvent>,
) {
    match fetch_all_child_ids(client, space_id).await {
        Ok(children) => {
            let _ = event_tx.send(ClientEvent::SpaceChildrenFetched {
                space_id: space_id.to_string(),
                children,
            });
        }
        Err(error) => {
            tracing::warn!(%space_id, %error, "could not fetch space children for sidebar grouping");
        }
    }
}

/// Gets joined spaces flowing into the client store — without this the
/// sidebar's "Spaces" section (the only way into the explorer) stays empty
/// forever. matrix-sdk-ui's `RoomListService` hard-codes
/// `not_room_types: ["m.space"]` into its sliding sync list, so a joined
/// space never arrives through sync alone; room *subscriptions* bypass list
/// filters, so this task discovers the account's joined spaces server-side
/// once at startup and reports them over `pin_tx` to the sync worker, which
/// owns the session subscription set and subscribes to them. From there the
/// normal machinery takes over (store insert → room-list forwarder → sidebar).
///
/// The worker (not this task) issues the subscription because since
/// matrix-sdk-ui 0.17 `subscribe_to_rooms` REPLACES its set — a subscription
/// issued here would clobber the open rooms' subscriptions and vice-versa, so
/// the set has to be owned in one place.
///
/// Spaces joined mid-session from the explorer are covered by the `JoinRoom`
/// handler pinning whatever it joins; a space joined from *another* client
/// mid-session only shows up on the next start.
pub fn spawn_joined_spaces_subscriber(
    client: Client,
    room_list_service: Arc<RoomListService>,
    pin_tx: mpsc::UnboundedSender<Vec<OwnedRoomId>>,
    event_tx: mpsc::UnboundedSender<ClientEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        // Wait for the first full room-list load: before it, "joined
        // server-side but missing from the store" describes most of the
        // account's ordinary rooms, not just its spaces, and the create-state
        // probe below would fire once per room instead of once per space.
        match room_list_service.all_rooms().await {
            Ok(all_rooms) => {
                let mut loading = all_rooms.loading_state();
                loop {
                    match loading.next().await {
                        Some(RoomListLoadingState::Loaded { .. }) => break,
                        Some(RoomListLoadingState::NotLoaded) => continue,
                        // Observable dropped — the sync service is shutting
                        // down, nothing left to do.
                        None => return,
                    }
                }
            }
            Err(error) => {
                tracing::warn!(%error, "space discovery: could not watch the room list loading state; probing immediately");
            }
        }

        let joined = match client.send(joined_rooms::v3::Request::new()).await {
            Ok(response) => response.joined_rooms,
            Err(error) => {
                tracing::warn!(%error, "space discovery: /joined_rooms failed; joined spaces stay hidden until next start");
                return;
            }
        };

        let mut space_ids: Vec<OwnedRoomId> = Vec::new();
        for room_id in joined {
            match client.get_room(&room_id) {
                // Already in the store (persisted by a previous session's
                // subscription): re-subscribe spaces so their name/avatar
                // keep updating — subscriptions only last a session.
                Some(room) => {
                    if room.state() == RoomState::Joined && room.is_space() {
                        space_ids.push(room_id);
                    }
                }
                // Joined server-side yet unknown to a fully-loaded store —
                // almost certainly a space; confirm via its m.room.create
                // rather than subscribing blindly.
                None => match fetch_room_type(&client, &room_id).await {
                    Ok(Some(RoomType::Space)) => space_ids.push(room_id),
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!(%room_id, %error, "space discovery: create-state fetch failed; skipping room");
                    }
                },
            }
        }

        if space_ids.is_empty() {
            return;
        }
        // Hand the discovered spaces to the sync worker, which owns the
        // session subscription set and subscribes them alongside the open
        // rooms. (Subscribing here would clobber that set — 0.17+ replace
        // semantics.)
        tracing::info!(count = space_ids.len(), "discovered joined spaces");
        let _ = pin_tx.send(space_ids.clone());

        // With the spaces subscribed, fetch each one's child ids so the
        // sidebar can nest joined rooms under their space instead of
        // listing the container next to its contents.
        for space_id in &space_ids {
            emit_space_children(&client, space_id, &event_tx).await;
        }
    })
}

/// The room's `m.room.create` `type` field, fetched over REST — for rooms
/// the local store doesn't know (sliding sync never delivers spaces).
async fn fetch_room_type(
    client: &Client,
    room_id: &RoomId,
) -> anyhow::Result<Option<RoomType>> {
    let request = get_state_event_for_key::v3::Request::new(
        room_id.to_owned(),
        StateEventType::RoomCreate,
        String::new(),
    );
    let response = client.send(request).await?;
    // 0.18: the response carries raw JSON (`event_or_content`); parse it directly.
    let content = serde_json::from_str::<RoomCreateEventContent>(response.event_or_content.get())?;
    Ok(content.room_type)
}

fn map_join_rule(rule: &matrix_sdk::ruma::room::JoinRuleSummary) -> SpaceJoinRule {
    use matrix_sdk::ruma::room::JoinRuleSummary as Sdk;
    match rule {
        Sdk::Public => SpaceJoinRule::Public,
        Sdk::Restricted(_) | Sdk::KnockRestricted(_) => SpaceJoinRule::Restricted,
        Sdk::Knock => SpaceJoinRule::Knock,
        // Invite, Private, and custom rules all mean "you can't just join".
        _ => SpaceJoinRule::InviteOnly,
    }
}
