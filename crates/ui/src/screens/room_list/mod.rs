//! Room list sidebar: one group per joined space — the space renders as a
//! header (clicking it opens the explorer overlay; a space has no timeline
//! of its own) with its joined rooms nested beneath, Cinny-style — then
//! flat DM/room sections for everything spaceless. Child sets come from
//! `ClientEvent::SpaceChildrenFetched` (hierarchy sweep at startup/join); a
//! joined subspace groups as its own top-level header rather than nesting
//! deeper.

use std::collections::{HashMap, HashSet};

use client_core::events::{NotificationMode, RoomSummary};
use iced::widget::{button, column, container, row, scrollable, space, text};
use iced::{Element, Length};

#[derive(Debug, Clone, Default)]
pub struct State {
    pub rooms: Vec<RoomSummary>,
    /// space id → all of its child room ids (joined or not, straight from
    /// the space hierarchy) — what nests a joined room under its space.
    pub space_children: HashMap<String, Vec<String>>,
    pub selected_room_id: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    RoomClicked(String),
    /// Click on a space row — opens the space explorer overlay (handled at
    /// the app level, which issues the hierarchy fetch).
    SpaceClicked(String),
    /// Right-click on a room/DM/space row — opens the leave/forget confirm
    /// prompt (handled at the app level, which knows the room's display
    /// name).
    RoomRightClicked(String),
    /// The "+" on the Rooms header — create a fresh solo room and open it
    /// (handled at the app level).
    NewRoomClicked,
    /// The "+" on the Direct messages header — opens the user-search overlay
    /// to start a DM by name (handled at the app level).
    NewDirectMessageClicked,
}

pub fn view<'a>(
    state: &'a State,
    notification_modes: &'a HashMap<String, NotificationMode>,
    calls: &'a crate::screens::call::State,
    media: &'a crate::media_cache::State,
) -> Element<'a, Message> {
    let mut list = column![].spacing(2).padding(8);

    // Space groups first: the space is a container, so it sits *above* the
    // rooms it contains, never next to them. The first space to list a room
    // claims it (a room can be in several spaces; showing it twice would
    // read as two rooms); DMs keep their own section even when a space
    // lists them.
    let mut claimed: HashSet<&str> = HashSet::new();
    for space in state.rooms.iter().filter(|r| r.is_space) {
        let child_ids: HashSet<&str> = state
            .space_children
            .get(&space.room_id)
            .map(|ids| ids.iter().map(String::as_str).collect())
            .unwrap_or_default();
        let children: Vec<&RoomSummary> = state
            .rooms
            .iter()
            .filter(|r| {
                !r.is_space
                    && !r.is_dm
                    && child_ids.contains(r.room_id.as_str())
                    && !claimed.contains(r.room_id.as_str())
            })
            .collect();
        claimed.extend(children.iter().map(|r| r.room_id.as_str()));

        list = list.push(space_row(space, media));
        for room in children {
            list = list.push(
                container(room_row(state, room, notification_modes, calls, media)).padding(
                    iced::Padding { left: 18.0, ..iced::Padding::ZERO },
                ),
            );
        }
    }

    let dms: Vec<&RoomSummary> = state.rooms.iter().filter(|r| r.is_dm && !r.is_space).collect();
    let rooms: Vec<&RoomSummary> = state
        .rooms
        .iter()
        .filter(|r| !r.is_dm && !r.is_space && !claimed.contains(r.room_id.as_str()))
        .collect();
    // Both sections render their header unconditionally (even when empty) so
    // the "+" affordance is always reachable — the DM one opens the user
    // search, the Rooms one creates a solo room. Their "+" buttons carry
    // different messages, so this is two explicit blocks rather than a loop.
    list = list.push(section_header("Direct messages", Message::NewDirectMessageClicked));
    for room in dms {
        list = list.push(room_row(state, room, notification_modes, calls, media));
    }
    list = list.push(section_header("Rooms", Message::NewRoomClicked));
    for room in rooms {
        list = list.push(room_row(state, room, notification_modes, calls, media));
    }

    container(
        scrollable(list)
            .height(Length::Fill)
            .direction(iced::widget::scrollable::Direction::Vertical(
                iced::widget::scrollable::Scrollbar::new().width(6).scroller_width(6),
            ))
            .style(crate::theme::thin_scrollbar),
    )
    .width(Length::Fixed(240.0))
    .height(Length::Fill)
    .style(crate::theme::panel)
    .into()
}

/// A sidebar section header: the label plus a trailing "+" icon button that
/// fires `add_message` (new room / new DM depending on the section).
fn section_header(label: &str, add_message: Message) -> Element<'_, Message> {
    let add = button(
        text(crate::theme::icon::ADD).font(crate::theme::ICON_FONT).size(12).style(text::secondary),
    )
    .on_press(add_message)
    .padding([2, 6])
    .style(crate::theme::ghost_button);

    container(
        row![
            text(label).size(11).font(crate::theme::SEMIBOLD_FONT).style(text::secondary),
            space::horizontal(),
            add,
        ]
        .align_y(iced::Center)
        .width(Length::Fill),
    )
    .width(Length::Fill)
    .padding([6, 4])
    .into()
}

/// A space group header: clicking opens the space explorer (browse/join its
/// rooms) rather than a timeline — a space room has no useful timeline of
/// its own. Rendered semibold with a chevron so it reads as the container
/// of the indented rooms beneath it. Right-click still offers leave/forget
/// like any other row.
fn space_row<'a>(
    space: &'a RoomSummary,
    media: &'a crate::media_cache::State,
) -> Element<'a, Message> {
    let label = row![
        crate::media_cache::avatar(media, space.avatar_url.as_deref(), &space.name, 26),
        crate::theme::remote_text(space.name.clone())
            .size(14)
            .font(crate::theme::SEMIBOLD_FONT)
            .width(Length::Fill),
        text(crate::theme::icon::CHEVRON_RIGHT)
            .font(crate::theme::ICON_FONT)
            .size(10)
            .style(text::secondary),
    ]
    .spacing(8)
    .align_y(iced::Center);

    let row_button = button(label)
        .on_press(Message::SpaceClicked(space.room_id.clone()))
        .width(Length::Fill)
        .padding([6, 8])
        .style(crate::theme::ghost_button);

    iced::widget::mouse_area(row_button)
        .on_right_press(Message::RoomRightClicked(space.room_id.clone()))
        .into()
}

fn room_row<'a>(
    state: &'a State,
    room: &'a RoomSummary,
    notification_modes: &'a HashMap<String, NotificationMode>,
    calls: &'a crate::screens::call::State,
    media: &'a crate::media_cache::State,
) -> Element<'a, Message> {
    let is_selected = state.selected_room_id.as_deref() == Some(room.room_id.as_str());
    let is_muted = notification_modes.get(&room.room_id) == Some(&NotificationMode::Mute);

    let mut label = row![].spacing(8).align_y(iced::Center);
    label = label.push(crate::media_cache::avatar(media, room.avatar_url.as_deref(), &room.name, 26));
    let name_size = 14;
    if is_muted {
        label = label.push(
            crate::theme::remote_text(room.name.clone())
                .size(name_size)
                .style(text::secondary)
                .width(Length::Fill),
        );
        label = label.push(text("muted").size(10).style(text::secondary));
    } else {
        label = label
            .push(crate::theme::remote_text(room.name.clone()).size(name_size).width(Length::Fill));
    }
    if calls.has_active_call(&room.room_id) {
        label = label.push(
            text(crate::theme::icon::CALL)
                .font(crate::theme::ICON_FONT)
                .size(11)
                .style(text::success),
        );
    }
    if room.unread_count > 0 && !is_muted {
        label = label.push(
            container(text(room.unread_count.to_string()).size(11))
                .padding([1, 7])
                .style(crate::theme::pill_badge),
        );
    }

    let row_button = button(label)
        .on_press(Message::RoomClicked(room.room_id.clone()))
        .width(Length::Fill)
        .padding([6, 8]);

    let styled = if is_selected {
        row_button.style(crate::theme::selected_ghost_button)
    } else {
        row_button.style(crate::theme::ghost_button)
    };

    // Right-click opens the leave/forget prompt; left-click still selects.
    iced::widget::mouse_area(styled)
        .on_right_press(Message::RoomRightClicked(room.room_id.clone()))
        .into()
}
