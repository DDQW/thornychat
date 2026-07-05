pub mod appearance;
pub mod general;
pub mod notifications;
pub mod room_admin;

use iced::widget::{button, column, row, text};
use iced::{Element, Task};

use crate::theme_config::ThemeConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    General,
    Notifications,
    RoomAdmin,
    Appearance,
}

const TABS: [(Tab, &str); 4] = [
    (Tab::General, "General"),
    (Tab::Notifications, "Notifications"),
    (Tab::RoomAdmin, "Room Admin"),
    (Tab::Appearance, "Appearance"),
];

#[derive(Debug, Clone)]
pub struct State {
    pub tab: Tab,
    pub appearance: appearance::State,
}

impl State {
    /// Built from the currently-active theme so the Appearance tab's draft
    /// hex fields reflect reality on first open, rather than some
    /// context-free default that might not match what's actually loaded.
    pub fn new(theme: &ThemeConfig) -> Self {
        // Opens straight to Appearance — General/Notifications/Room Admin
        // are still inert placeholders, so landing there first would just
        // show a "coming soon" message.
        Self { tab: Tab::Appearance, appearance: appearance::State::synced_from(theme) }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    TabSelected(Tab),
    Appearance(appearance::Message),
}

pub fn update(state: &mut State, theme: &mut ThemeConfig, message: Message) -> Task<Message> {
    match message {
        Message::TabSelected(tab) => {
            state.tab = tab;
            Task::none()
        }
        Message::Appearance(msg) => {
            appearance::update(&mut state.appearance, theme, msg).map(Message::Appearance)
        }
    }
}

pub fn view<'a>(state: &'a State, theme: &'a ThemeConfig) -> Element<'a, Message> {
    let mut tabs = row![].spacing(4);
    for (tab, label) in TABS {
        let style =
            if tab == state.tab { crate::theme::selected_ghost_button } else { crate::theme::ghost_button };
        tabs = tabs.push(
            button(text(label).size(13)).on_press(Message::TabSelected(tab)).style(style).padding([6, 12]),
        );
    }

    let body: Element<'_, Message> = match state.tab {
        Tab::General => general::view(),
        Tab::Notifications => notifications::view(),
        Tab::RoomAdmin => room_admin::view(),
        Tab::Appearance => appearance::view(&state.appearance, theme).map(Message::Appearance),
    };

    column![tabs, body].spacing(16).into()
}
