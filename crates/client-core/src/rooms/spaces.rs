//! Space hierarchy ("explore space"): lists a space's children through the
//! client-server `/hierarchy` API (MSC2946), the standard Matrix space
//! directory — it returns rooms the account hasn't joined, which is the
//! point. Fetched one level deep per request; the UI drills into subspaces
//! with follow-up fetches instead of walking the whole tree up front.

use std::collections::HashMap;

use matrix_sdk::ruma::api::client::space::get_hierarchy;
use matrix_sdk::ruma::{OwnedRoomId, RoomId, UInt};
use matrix_sdk::Client;

use crate::events::{SpaceChildSummary, SpaceJoinRule};

/// Children per page — small enough for a snappy first paint, paged via the
/// returned `next_batch` token.
const PAGE_LIMIT: u32 = 50;

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
        .filter(|chunk| chunk.room_id != space_id)
        .map(|chunk| SpaceChildSummary {
            room_id: chunk.room_id.to_string(),
            name: chunk.name.clone(),
            topic: chunk.topic.clone(),
            canonical_alias: chunk.canonical_alias.as_ref().map(ToString::to_string),
            avatar_url: chunk.avatar_url.as_ref().map(ToString::to_string),
            num_joined_members: chunk.num_joined_members.into(),
            is_space: chunk.room_type == Some(matrix_sdk::ruma::room::RoomType::Space),
            joined: client
                .get_room(&chunk.room_id)
                .is_some_and(|room| room.state() == matrix_sdk::RoomState::Joined),
            join_rule: map_join_rule(&chunk.join_rule),
            via: via.get(&chunk.room_id).cloned().unwrap_or_default(),
        })
        .collect();

    Ok((children, response.next_batch))
}

fn map_join_rule(rule: &matrix_sdk::ruma::space::SpaceRoomJoinRule) -> SpaceJoinRule {
    use matrix_sdk::ruma::space::SpaceRoomJoinRule as Sdk;
    match rule {
        Sdk::Public => SpaceJoinRule::Public,
        Sdk::Restricted | Sdk::KnockRestricted => SpaceJoinRule::Restricted,
        Sdk::Knock => SpaceJoinRule::Knock,
        // Invite, Private, and custom rules all mean "you can't just join".
        _ => SpaceJoinRule::InviteOnly,
    }
}
