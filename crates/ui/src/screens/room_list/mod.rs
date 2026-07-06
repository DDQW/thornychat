//! Room list sidebar: a "Spaces" section (each row opens the space
//! explorer overlay — a space has no timeline of its own), then flat
//! DM/room sections, with a local filter box on top. Nesting joined rooms
//! *under* their parent space is still deferred until
//! `client_core::rooms::spaces` tracks parent/child relationships.

use std::collections::HashMap;

use client_core::events::{NotificationMode, RoomSummary};
use iced::widget::{button, column, container, row, scrollable, text, text_input};
use iced::{Element, Length};

#[derive(Debug, Clone, Default)]
pub struct State {
    pub rooms: Vec<RoomSummary>,
    pub selected_room_id: Option<String>,
    pub filter: String,
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
    FilterChanged(String),
}

pub fn view<'a>(
    state: &'a State,
    notification_modes: &'a HashMap<String, NotificationMode>,
    calls: &'a crate::screens::call::State,
    media: &'a crate::media_cache::State,
) -> Element<'a, Message> {
    let filter = state.filter.trim().to_lowercase();
    let visible = |room: &&client_core::events::RoomSummary| {
        filter.is_empty() || room.name.to_lowercase().contains(&filter)
    };
    let spaces: Vec<&client_core::events::RoomSummary> =
        state.rooms.iter().filter(|r| r.is_space).filter(visible).collect();
    let dms: Vec<&client_core::events::RoomSummary> =
        state.rooms.iter().filter(|r| r.is_dm && !r.is_space).filter(visible).collect();
    let rooms: Vec<&client_core::events::RoomSummary> =
        state.rooms.iter().filter(|r| !r.is_dm && !r.is_space).filter(visible).collect();

    let mut list = column![].spacing(2).padding(8);
    if !spaces.is_empty() {
        list = list.push(section_header("Spaces"));
        for space in spaces {
            list = list.push(space_row(space, media));
        }
    }
    for (label, group) in [("Direct messages", dms), ("Rooms", rooms)] {
        if group.is_empty() {
            continue;
        }
        list = list.push(section_header(label));
        for room in group {
            list = list.push(room_row(state, room, notification_modes, calls, media));
        }
    }

    let search = text_input("Filter rooms...", &state.filter)
        .on_input(Message::FilterChanged)
        .padding(6)
        .size(13);

    container(column![
        container(search).padding(8),
        scrollable(list)
            .height(Length::Fill)
            .direction(iced::widget::scrollable::Direction::Vertical(
                iced::widget::scrollable::Scrollbar::new().width(6).scroller_width(6),
            ))
            .style(crate::theme::thin_scrollbar),
    ])
    .width(Length::Fixed(240.0))
    .height(Length::Fill)
    .style(crate::theme::panel)
    .into()
}

fn section_header(label: &str) -> Element<'_, Message> {
    container(text(label).size(11).font(crate::theme::SEMIBOLD_FONT).style(text::secondary))
        .padding([6, 4])
        .into()
}

/// A space row: clicking opens the space explorer (browse/join its rooms)
/// rather than a timeline — a space room has no useful timeline of its own.
/// Right-click still offers leave/forget like any other row.
fn space_row<'a>(
    space: &'a RoomSummary,
    media: &'a crate::media_cache::State,
) -> Element<'a, Message> {
    let label = row![
        crate::media_cache::avatar(media, space.avatar_url.as_deref(), &space.name, 26),
        text(space.name.clone()).size(14).width(Length::Fill),
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
            text(room.name.clone()).size(name_size).style(text::secondary).width(Length::Fill),
        );
        label = label.push(text("muted").size(10).style(text::secondary));
    } else {
        label = label.push(text(room.name.clone()).size(name_size).width(Length::Fill));
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
