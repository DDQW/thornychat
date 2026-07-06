//! Account-wide default notification mode for direct messages and group
//! chats — what a room follows when it has no per-room override (set from
//! the room header, see `screens::timeline`).

use iced::widget::{column, pick_list, row, text};
use iced::{Element, Length, Task};

use client_core::events::{NotificationMode, NotificationScope};

/// Mirrors `screens::timeline::NotifyChoice` — a `ui`-local `Display`
/// wrapper, needed because `pick_list` requires `ToString` and the orphan
/// rule blocks implementing it on `NotificationMode` directly from this
/// crate. No fourth "inherit" variant here since this picker sets the
/// default itself rather than a per-room override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeChoice {
    All,
    Mentions,
    Mute,
}

const MODE_CHOICES: [ModeChoice; 3] = [ModeChoice::All, ModeChoice::Mentions, ModeChoice::Mute];

impl ModeChoice {
    fn from_mode(mode: NotificationMode) -> Self {
        match mode {
            NotificationMode::AllMessages => ModeChoice::All,
            NotificationMode::MentionsAndKeywordsOnly => ModeChoice::Mentions,
            NotificationMode::Mute => ModeChoice::Mute,
        }
    }

    fn to_mode(self) -> NotificationMode {
        match self {
            ModeChoice::All => NotificationMode::AllMessages,
            ModeChoice::Mentions => NotificationMode::MentionsAndKeywordsOnly,
            ModeChoice::Mute => NotificationMode::Mute,
        }
    }
}

impl std::fmt::Display for ModeChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ModeChoice::All => "All messages",
            ModeChoice::Mentions => "Mentions only",
            ModeChoice::Mute => "Mute",
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct State;

#[derive(Debug, Clone)]
pub enum Message {
    DirectDefaultChanged(ModeChoice),
    GroupDefaultChanged(ModeChoice),
}

pub fn update(_state: &mut State, message: Message) -> (Task<Message>, super::Effect) {
    let (scope, choice) = match message {
        Message::DirectDefaultChanged(choice) => (NotificationScope::DirectMessages, choice),
        Message::GroupDefaultChanged(choice) => (NotificationScope::GroupChats, choice),
    };
    (Task::none(), super::Effect::SetDefaultNotificationMode { scope, mode: choice.to_mode() })
}

pub fn view<'a>(_state: &'a State, default_modes: (NotificationMode, NotificationMode)) -> Element<'a, Message> {
    let (direct_messages, group_chats) = default_modes;

    let direct_row = row![
        text("Direct messages").size(13).width(Length::Fixed(160.0)),
        pick_list(MODE_CHOICES, Some(ModeChoice::from_mode(direct_messages)), Message::DirectDefaultChanged)
            .text_size(13)
            .padding([6, 10]),
    ]
    .spacing(8)
    .align_y(iced::Center);

    let group_row = row![
        text("Group chats").size(13).width(Length::Fixed(160.0)),
        pick_list(MODE_CHOICES, Some(ModeChoice::from_mode(group_chats)), Message::GroupDefaultChanged)
            .text_size(13)
            .padding([6, 10]),
    ]
    .spacing(8)
    .align_y(iced::Center);

    column![text("Default notifications").size(14).font(crate::theme::SEMIBOLD_FONT), direct_row, group_row]
        .spacing(10)
        .into()
}
