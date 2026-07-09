//! MSC3949 power-level tags: rooms can name (and color) their power
//! levels — "Red team", "Purple team", "Bot" — which clients like Cinny
//! render as custom member-list groups. Stored as room state of type
//! `m.room.power_level_tags` under the unstable name
//! `in.cinny.room.power_level_tags` (the MSC's mandated prefix).
//!
//! Like the MSC2545 emoji packs, this is a custom state type that sliding
//! sync's fixed `required_state` never delivers, so it's fetched straight
//! from the homeserver rather than the local store.

use std::collections::BTreeMap;

use matrix_sdk::ruma::api::client::state::get_state_event_for_key;
use matrix_sdk::ruma::events::StateEventType;
use matrix_sdk::Room;
use serde::Deserialize;

use crate::events::PowerLevelTag;

#[derive(Debug, Deserialize)]
struct TagContent {
    name: String,
    #[serde(default)]
    color: Option<String>,
}

/// Every tag the room defines, highest power level first. Empty when the
/// room has no tag event (the UI falls back to Admin/Mod/Member).
pub async fn fetch(room: &Room) -> Vec<PowerLevelTag> {
    for event_type in ["in.cinny.room.power_level_tags", "m.room.power_level_tags"] {
        let request = get_state_event_for_key::v3::Request::new(
            room.room_id().to_owned(),
            StateEventType::from(event_type),
            String::new(),
        );
        let raw: matrix_sdk::ruma::serde::Raw<serde_json::Value> =
            match room.client().send(request).await {
                Ok(response) => matrix_sdk::ruma::serde::Raw::from_json(response.event_or_content),
            Err(error) => {
                let not_found =
                    error.as_client_api_error().is_some_and(|e| e.status_code.as_u16() == 404);
                if !not_found {
                    tracing::warn!(room_id = %room.room_id(), %error, "failed to fetch power level tags");
                }
                continue;
            }
        };

        // Keys are power levels as strings ("100", "38", ...) mapping to
        // tag objects; unparseable levels or malformed tags are skipped
        // rather than failing the whole set.
        let content: BTreeMap<String, serde_json::Value> = match raw.deserialize_as_unchecked() {
            Ok(content) => content,
            Err(error) => {
                tracing::warn!(room_id = %room.room_id(), %error, "failed to deserialize power level tags");
                continue;
            }
        };

        let mut tags: Vec<PowerLevelTag> = content
            .into_iter()
            .filter_map(|(level, value)| {
                let level: i64 = level.trim().parse().ok()?;
                let tag: TagContent = serde_json::from_value(value).ok()?;
                Some(PowerLevelTag { level, name: tag.name, color: tag.color })
            })
            .collect();
        if tags.is_empty() {
            continue;
        }
        tags.sort_by_key(|t| std::cmp::Reverse(t.level));
        tracing::info!(room_id = %room.room_id(), count = tags.len(), %event_type, "resolved power level tags");
        return tags;
    }
    Vec::new()
}
