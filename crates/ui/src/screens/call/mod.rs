//! Call banner for the open room (Phase 5, signaling only): shows the
//! room's active MatrixRTC call, who's in it, and Join/Leave. Media
//! doesn't flow yet — the banner says so while joined — but memberships
//! are real: Element/Element Call users see this device in the call.
//!
//! No `Message`/`update` of its own: the banner renders inside the
//! timeline column (below the room header), so it emits the timeline's
//! messages, and the root dispatcher owns this `State`.

use std::collections::HashMap;

use client_core::commands::RequestId;
use client_core::events::{CallState, RoomMember};
use iced::widget::{button, container, row, text, tooltip};
use iced::{Element, Length};

/// Per-room call signaling state plus in-flight join/leave bookkeeping,
/// owned by the root `App` and fed by `ClientEvent::CallStateUpdated`.
#[derive(Debug, Clone, Default)]
pub struct State {
    /// Latest state per room. An entry with no participants means "no
    /// active call" (kept, so a call ending clears the banner).
    pub calls: HashMap<String, CallState>,
    /// In-flight JoinCall/LeaveCall: (request id, room id, joining). The
    /// bool records the direction so the banner label doesn't have to infer
    /// it from live flags (which the optimistic CallStateUpdated flips one
    /// message before CommandSucceeded clears this). Disables the banner's
    /// button and routes `CommandFailed` back to it.
    pub pending: Option<(RequestId, String, bool)>,
    /// (room id, message) — room-scoped so a failed join in room A doesn't
    /// paint an error strip over every other room's banner.
    pub error: Option<(String, String)>,
}

impl State {
    /// Whether the room currently has an active call (sidebar indicator,
    /// header start-call button visibility).
    pub fn has_active_call(&self, room_id: &str) -> bool {
        self.calls.get(room_id).is_some_and(|c| !c.participants.is_empty())
    }

    pub fn pending_for(&self, room_id: &str) -> bool {
        self.pending.as_ref().is_some_and(|(_, id, _)| id == room_id)
    }
}

/// The banner for a room's active call (or a join/leave in flight, or a
/// failed one) — `None` when there's nothing to show. Callers wrap it in
/// `theme::slot` so the widget tree keeps its shape either way.
#[allow(clippy::too_many_arguments)]
pub fn banner<'a, M: Clone + 'a>(
    state: &'a State,
    room_id: &str,
    members: &'a [RoomMember],
    member_index: &'a HashMap<String, usize>,
    media: &'a crate::media_cache::State,
    on_join: M,
    on_leave: M,
    on_dismiss_error: M,
) -> Option<Element<'a, M>> {
    let call = state.calls.get(room_id);
    let pending_join: Option<bool> = state
        .pending
        .as_ref()
        .filter(|(_, id, _)| id == room_id)
        .map(|(_, _, joining)| *joining);
    let pending = pending_join.is_some();
    let joined = call.is_some_and(|c| c.joined);
    let active = call.is_some_and(|c| !c.participants.is_empty());
    // Only this room's error — a failed join in room A must not replace
    // room B's banner (or render outside A at all).
    let error = state.error.as_ref().filter(|(id, _)| id == room_id).map(|(_, e)| e);
    if !active && !joined && !pending && error.is_none() {
        return None;
    }

    let mut bar = row![].spacing(10).align_y(iced::Center).width(Length::Fill);
    bar = bar.push(text(crate::theme::icon::CALL).font(crate::theme::ICON_FONT).size(14));

    if let Some(error) = error {
        bar = bar.push(text(error.clone()).size(12).style(text::danger).width(Length::Fill));
        bar = bar.push(
            button(text(crate::theme::icon::CLOSE).font(crate::theme::ICON_FONT).size(11))
                .on_press(on_dismiss_error)
                .style(crate::theme::ghost_button)
                .padding([2, 6]),
        );
        return Some(styled(bar));
    }

    // Direction comes from the pending record, not from `joined`/`active` —
    // the optimistic CallStateUpdated flips those one message before
    // CommandSucceeded clears `pending`, which briefly showed the opposite
    // label ("Leaving call…" right after a successful join).
    let label = match pending_join {
        Some(false) => "Leaving call…",
        Some(true) => {
            if active {
                "Joining call…"
            } else {
                "Starting call…"
            }
        }
        None if joined => "In call — signaling only, no audio yet",
        None => "Call in progress",
    };
    bar = bar.push(text(label).size(13).font(crate::theme::SEMIBOLD_FONT));

    // One face per user (a user on two devices still shows once).
    let mut user_ids: Vec<&str> = Vec::new();
    for participant in call.map(|c| c.participants.as_slice()).unwrap_or_default() {
        if !user_ids.contains(&participant.user_id.as_str()) {
            user_ids.push(&participant.user_id);
        }
    }
    let mut faces = row![].spacing(4).align_y(iced::Center);
    const MAX_FACES: usize = 8;
    for user_id in user_ids.iter().take(MAX_FACES) {
        let (name, avatar_url) = match member_index.get(*user_id).and_then(|&i| members.get(i)) {
            Some(member) => (member.display_name.as_str(), member.avatar_url.as_deref()),
            None => (*user_id, None),
        };
        faces = faces.push(tooltip(
            crate::media_cache::avatar(media, avatar_url, name, 20),
            container(text(name.to_owned()).size(12)).padding(6).style(crate::theme::panel),
            tooltip::Position::Bottom,
        ));
    }
    if user_ids.len() > MAX_FACES {
        faces = faces.push(text(format!("+{}", user_ids.len() - MAX_FACES)).size(12));
    }
    bar = bar.push(faces);
    if !user_ids.is_empty() {
        bar = bar.push(
            text(format!("{} in call", user_ids.len())).size(12).style(text::secondary),
        );
    }
    bar = bar.push(iced::widget::horizontal_space());

    let action: Element<'a, M> = if pending {
        // In flight — the label above already says which way.
        text("…").size(13).into()
    } else if joined {
        button(text("Leave").size(12))
            .on_press(on_leave)
            .style(button::danger)
            .padding([4, 10])
            .into()
    } else {
        button(text("Join call").size(12))
            .on_press(on_join)
            .style(button::primary)
            .padding([4, 10])
            .into()
    };
    bar = bar.push(action);

    Some(styled(bar))
}

/// Accent-tinted strip under the room header, visually distinct from both
/// the header and the message list.
fn styled<'a, M: 'a>(bar: iced::widget::Row<'a, M>) -> Element<'a, M> {
    container(bar)
        .padding([6, 12])
        .width(Length::Fill)
        .style(|theme: &iced::Theme| {
            let palette = theme.extended_palette();
            iced::widget::container::Style {
                background: Some(palette.primary.weak.color.into()),
                text_color: Some(palette.primary.weak.text),
                ..iced::widget::container::Style::default()
            }
        })
        .into()
}
