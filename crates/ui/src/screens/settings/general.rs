//! Account info, sign-out, Windows autostart, and spell-check preferences.

use iced::widget::{button, column, row, text, toggler};
use iced::{Element, Length, Task};

use crate::spellcheck_config::SpellcheckConfig;

#[derive(Debug, Clone)]
pub struct State {
    confirm_logout: bool,
    autostart_enabled: bool,
}

impl State {
    /// Reads the real registry state rather than assuming "off". Called each
    /// time the Settings panel opens (see `Message::ToggleSettings`), so the
    /// toggle reflects reality even if autostart was removed by hand (or by
    /// an uninstaller) since it was last open.
    pub fn new() -> Self {
        Self { confirm_logout: false, autostart_enabled: crate::platform::autostart::is_enabled() }
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

/// Read-only account fields sourced from `App` (`own_user_id`/`client`) —
/// computed at the `view.rs` call site so this module never needs to depend
/// on `matrix_sdk::Client` directly.
pub struct AccountInfo<'a> {
    pub user_id: Option<&'a str>,
    pub homeserver: Option<String>,
    pub device_id: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    RequestLogout,
    ConfirmLogout,
    CancelLogout,
    AutostartToggled(bool),
    SpellcheckToggled(bool),
    AutocorrectToggled(bool),
    /// Autosave task finished; nothing to do (mirrors `privacy::Saved`).
    SpellcheckSaved,
}

pub fn update(
    state: &mut State,
    spellcheck: &mut SpellcheckConfig,
    message: Message,
) -> (Task<Message>, super::Effect) {
    match message {
        Message::RequestLogout => {
            state.confirm_logout = true;
            (Task::none(), super::Effect::None)
        }
        Message::ConfirmLogout => {
            state.confirm_logout = false;
            (Task::none(), super::Effect::Logout)
        }
        Message::CancelLogout => {
            state.confirm_logout = false;
            (Task::none(), super::Effect::None)
        }
        Message::AutostartToggled(enabled) => {
            match crate::platform::autostart::set_enabled(enabled) {
                Ok(()) => state.autostart_enabled = enabled,
                Err(error) => tracing::warn!(%error, "failed to update autostart registration"),
            }
            (Task::none(), super::Effect::None)
        }
        Message::SpellcheckToggled(on) => {
            spellcheck.enabled = on;
            (save_spellcheck_task(*spellcheck), super::Effect::None)
        }
        Message::AutocorrectToggled(on) => {
            spellcheck.autocorrect = on;
            (save_spellcheck_task(*spellcheck), super::Effect::None)
        }
        Message::SpellcheckSaved => (Task::none(), super::Effect::None),
    }
}

/// Persists the spell-check preferences off the update thread, like the
/// privacy/appearance autosaves.
fn save_spellcheck_task(spellcheck: SpellcheckConfig) -> Task<Message> {
    let (Some(path), Some(contents)) =
        (SpellcheckConfig::config_path(), spellcheck.to_json_pretty())
    else {
        return Task::none();
    };
    Task::future(async move {
        if let Some(parent) = path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        if let Err(error) = tokio::fs::write(path, contents).await {
            tracing::warn!(%error, "failed to save spell-check settings");
        }
        Message::SpellcheckSaved
    })
}

pub fn view<'a>(
    state: &'a State,
    account: AccountInfo<'a>,
    spellcheck: &'a SpellcheckConfig,
) -> Element<'a, Message> {
    let info_row = |label: &'static str, value: String| {
        row![text(label).size(12).width(Length::Fixed(110.0)), text(value).size(13)].spacing(8)
    };

    let account_section = column![
        text("Account").size(14).font(crate::theme::SEMIBOLD_FONT),
        info_row("User ID", account.user_id.unwrap_or("-").to_string()),
        info_row("Homeserver", account.homeserver.unwrap_or_else(|| "-".to_string())),
        info_row("Device ID", account.device_id.unwrap_or_else(|| "-".to_string())),
    ]
    .spacing(6);

    let sign_out_section: Element<'_, Message> = if state.confirm_logout {
        row![
            text(format!("Sign out of {}?", account.user_id.unwrap_or("this account"))).size(13),
            button(text("Yes").size(13))
                .on_press(Message::ConfirmLogout)
                .style(crate::theme::ghost_button)
                .padding([6, 10]),
            button(text("No").size(13))
                .on_press(Message::CancelLogout)
                .style(crate::theme::ghost_button)
                .padding([6, 10]),
        ]
        .spacing(8)
        .align_y(iced::Center)
        .into()
    } else {
        button(text("Sign out").size(13))
            .on_press(Message::RequestLogout)
            .style(crate::theme::ghost_button)
            .padding([6, 12])
            .into()
    };

    let autostart_section = column![
        text("Startup").size(14).font(crate::theme::SEMIBOLD_FONT),
        row![
            text("Start with Windows").size(13).width(Length::Fill),
            toggler(state.autostart_enabled).on_toggle(Message::AutostartToggled),
        ]
        .spacing(8)
        .align_y(iced::Center),
    ]
    .spacing(6);

    let spelling_section = column![
        text("Spelling").size(14).font(crate::theme::SEMIBOLD_FONT),
        spell_toggle(
            "Check spelling",
            "Show suggestions above the message box for a misspelled word. Uses the \
             Windows spell checker and your personal dictionary; nothing changes until \
             you tap a suggestion.",
            spellcheck.enabled,
            Message::SpellcheckToggled,
        ),
        spell_toggle(
            "Autocorrect",
            "Silently fix an obvious typo when you finish a word with a space. Press \
             Backspace right afterwards to undo the change.",
            spellcheck.autocorrect,
            Message::AutocorrectToggled,
        ),
    ]
    .spacing(12);

    column![account_section, sign_out_section, autostart_section, spelling_section]
        .spacing(20)
        .into()
}

/// A titled toggle with an explanatory sub-line (matches the Privacy tab's
/// layout so the two settings pages read the same).
fn spell_toggle<'a>(
    title: &'a str,
    description: &'a str,
    value: bool,
    on_toggle: impl Fn(bool) -> Message + 'a,
) -> Element<'a, Message> {
    row![
        column![
            text(title).size(13),
            text(description).size(11).style(text::secondary),
        ]
        .spacing(2)
        .width(Length::Fill),
        toggler(value).on_toggle(on_toggle),
    ]
    .spacing(12)
    .align_y(iced::Center)
    .into()
}
