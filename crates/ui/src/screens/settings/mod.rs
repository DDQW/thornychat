pub mod appearance;
pub mod encryption;
pub mod general;
pub mod notifications;
pub mod privacy;
pub mod room_admin;

use iced::widget::{button, column, row, text};
use iced::{Element, Task};

use crate::encryption_config::EncryptionConfig;
use crate::privacy_config::PrivacyConfig;
use crate::screens::verification;
use crate::spellcheck_config::SpellcheckConfig;
use crate::theme_config::ThemeConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    General,
    Privacy,
    Encryption,
    Notifications,
    RoomAdmin,
    Appearance,
    Security,
}

const TABS: [(Tab, &str); 7] = [
    (Tab::General, "General"),
    (Tab::Privacy, "Privacy"),
    (Tab::Encryption, "Encryption"),
    (Tab::Notifications, "Notifications"),
    (Tab::RoomAdmin, "Room Admin"),
    (Tab::Appearance, "Appearance"),
    (Tab::Security, "Security"),
];

#[derive(Debug, Clone)]
pub struct State {
    pub tab: Tab,
    pub general: general::State,
    pub privacy: privacy::State,
    pub encryption: encryption::State,
    pub notifications: notifications::State,
    pub appearance: appearance::State,
}

impl State {
    /// Built from the currently-active theme so the Appearance tab's draft
    /// hex fields reflect reality, rather than some context-free default
    /// that might not match what's actually loaded. Built once at app
    /// construction; General's autostart toggle is re-synced from the
    /// registry each time the panel opens (see `Message::ToggleSettings`),
    /// while the Appearance drafts persist so an in-progress edit isn't lost.
    pub fn new(theme: &ThemeConfig) -> Self {
        Self {
            // Room Admin is still an inert placeholder (Phase 6 — room
            // settings, member management, power levels, room creation),
            // so it's excluded from the default landing tab.
            tab: Tab::General,
            general: general::State::new(),
            privacy: privacy::State,
            encryption: encryption::State,
            notifications: notifications::State,
            appearance: appearance::State::synced_from(theme),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    TabSelected(Tab),
    General(general::Message),
    Privacy(privacy::Message),
    Encryption(encryption::Message),
    Notifications(notifications::Message),
    Appearance(appearance::Message),
    /// Security tab hosts the encryption backup/recovery UI, which is owned
    /// by `screens::verification`; its messages ride through here and are
    /// bubbled back up (via `Effect::Verification`) to that module's update.
    Security(verification::Message),
}

/// Commands that need to reach the sync worker, bubbled up from a tab's
/// `update()` to the top-level `update.rs` — settings' own `update()` has no
/// `cmd_tx` to send through directly, so it hands the request up the same
/// way `screens::timeline`/`screens::verification` already do.
#[derive(Debug, Clone)]
pub enum Effect {
    None,
    Logout,
    SetDefaultNotificationMode {
        scope: client_core::events::NotificationScope,
        mode: client_core::events::NotificationMode,
    },
    /// A message from the Security tab, to be dispatched to
    /// `screens::verification::update` by the top-level effect handler
    /// (which owns `&mut app.verification`).
    Verification(verification::Message),
}

pub fn update(
    state: &mut State,
    theme: &mut ThemeConfig,
    privacy: &mut PrivacyConfig,
    encryption: &mut EncryptionConfig,
    spellcheck: &mut SpellcheckConfig,
    message: Message,
) -> (Task<Message>, Effect) {
    match message {
        Message::TabSelected(tab) => {
            state.tab = tab;
            (Task::none(), Effect::None)
        }
        Message::General(msg) => {
            let (task, effect) = general::update(&mut state.general, spellcheck, msg);
            (task.map(Message::General), effect)
        }
        Message::Privacy(msg) => {
            let task = privacy::update(privacy, msg).map(Message::Privacy);
            (task, Effect::None)
        }
        Message::Encryption(msg) => {
            let task = encryption::update(encryption, msg).map(Message::Encryption);
            (task, Effect::None)
        }
        Message::Notifications(msg) => {
            let (task, effect) = notifications::update(&mut state.notifications, msg);
            (task.map(Message::Notifications), effect)
        }
        Message::Appearance(msg) => {
            let task = appearance::update(&mut state.appearance, theme, msg).map(Message::Appearance);
            (task, Effect::None)
        }
        Message::Security(msg) => (Task::none(), Effect::Verification(msg)),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn view<'a>(
    state: &'a State,
    theme: &'a ThemeConfig,
    privacy: &'a PrivacyConfig,
    encryption: &'a EncryptionConfig,
    spellcheck: &'a SpellcheckConfig,
    account: general::AccountInfo<'a>,
    default_modes: (client_core::events::NotificationMode, client_core::events::NotificationMode),
    verification: &'a verification::State,
) -> Element<'a, Message> {
    let mut tabs = row![].spacing(4);
    for (tab, label) in TABS {
        let style =
            if tab == state.tab { crate::theme::selected_ghost_button } else { crate::theme::ghost_button };
        tabs = tabs.push(
            button(text(label).size(13)).on_press(Message::TabSelected(tab)).style(style).padding([6, 12]),
        );
    }

    let body: Element<'_, Message> = match state.tab {
        Tab::General => general::view(&state.general, account, spellcheck).map(Message::General),
        Tab::Privacy => privacy::view(privacy).map(Message::Privacy),
        Tab::Encryption => encryption::view(encryption).map(Message::Encryption),
        Tab::Notifications => notifications::view(&state.notifications, default_modes).map(Message::Notifications),
        Tab::RoomAdmin => room_admin::view(),
        Tab::Appearance => appearance::view(&state.appearance, theme).map(Message::Appearance),
        Tab::Security => verification::recovery_settings_view(verification).map(Message::Security),
    };

    column![tabs, body].spacing(16).into()
}
