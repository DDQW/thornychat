//! Account info, sign-out, Windows autostart, and spell-check preferences.

use iced::widget::{button, column, row, text, toggler};
use iced::{Element, Length, Task};

use crate::chat_config::ChatConfig;
use crate::spellcheck_config::SpellcheckConfig;

#[derive(Debug, Clone)]
pub struct State {
    confirm_logout: bool,
    autostart_enabled: bool,
    log_copy_status: LogCopyStatus,
}

/// Feedback line under the "Copy log to clipboard" button — the read (disk)
/// and write (clipboard) are both blocking I/O, done off-thread, so this
/// tracks the in-flight/result state across that round trip.
#[derive(Debug, Clone, Default)]
enum LogCopyStatus {
    #[default]
    Idle,
    Copying,
    Copied {
        bytes: usize,
    },
    Failed(String),
}

impl State {
    /// Reads the real registry state rather than assuming "off". Called each
    /// time the Settings panel opens (see `Message::ToggleSettings`), so the
    /// toggle reflects reality even if autostart was removed by hand (or by
    /// an uninstaller) since it was last open.
    pub fn new() -> Self {
        Self {
            confirm_logout: false,
            autostart_enabled: crate::platform::autostart::is_enabled(),
            log_copy_status: LogCopyStatus::Idle,
        }
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
    ShowMembershipEventsToggled(bool),
    /// Autosave task finished; nothing to do (mirrors `privacy::Saved`).
    SpellcheckSaved,
    /// Chat/timeline-config autosave task finished; nothing to do.
    ChatConfigSaved,
    /// "Copy log to clipboard" pressed — read today's log file and write it
    /// to the system clipboard, off-thread (both are blocking I/O).
    CopyLogRequested,
    LogCopyFinished(Result<usize, String>),
}

pub fn update(
    state: &mut State,
    spellcheck: &mut SpellcheckConfig,
    chat: &mut ChatConfig,
    profile: &str,
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
        Message::ShowMembershipEventsToggled(on) => {
            chat.show_membership_events = on;
            (save_chat_config_task(*chat), super::Effect::None)
        }
        Message::SpellcheckSaved => (Task::none(), super::Effect::None),
        Message::ChatConfigSaved => (Task::none(), super::Effect::None),
        Message::CopyLogRequested => {
            state.log_copy_status = LogCopyStatus::Copying;
            (copy_log_task(profile.to_string()), super::Effect::None)
        }
        Message::LogCopyFinished(result) => {
            state.log_copy_status = match result {
                Ok(bytes) => LogCopyStatus::Copied { bytes },
                Err(error) => LogCopyStatus::Failed(error),
            };
            (Task::none(), super::Effect::None)
        }
    }
}

/// Reads the most recently written log file for `profile` and writes its
/// contents to the system clipboard. Both steps are blocking I/O
/// (`arboard::Clipboard` retry-waits when another process holds the
/// clipboard, same as `clipboard_paste::read`), so this runs entirely inside
/// `spawn_blocking` rather than on the update thread.
fn copy_log_task(profile: String) -> Task<Message> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                let path = client_core::store::AppPaths::for_profile(&profile)
                    .map_err(|error| format!("couldn't resolve app data directory: {error}"))?
                    .latest_log_file()
                    .ok_or_else(|| "no log file found yet".to_string())?;
                let contents = std::fs::read_to_string(&path)
                    .map_err(|error| format!("couldn't read {}: {error}", path.display()))?;
                let bytes = contents.len();
                let mut clipboard = arboard::Clipboard::new()
                    .map_err(|error| format!("clipboard unavailable: {error}"))?;
                clipboard
                    .set_text(contents)
                    .map_err(|error| format!("couldn't write to clipboard: {error}"))?;
                Ok(bytes)
            })
            .await
            .unwrap_or_else(|error| Err(format!("copy task panicked: {error}")))
        },
        Message::LogCopyFinished,
    )
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

/// Persists the timeline/chat preferences off the update thread, like the
/// spell-check autosave above.
fn save_chat_config_task(chat: ChatConfig) -> Task<Message> {
    Task::future(async move {
        chat.save().await;
        Message::ChatConfigSaved
    })
}

pub fn view<'a>(
    state: &'a State,
    account: AccountInfo<'a>,
    spellcheck: &'a SpellcheckConfig,
    chat: &'a ChatConfig,
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

    let timeline_section = column![
        text("Timeline").size(14).font(crate::theme::SEMIBOLD_FONT),
        spell_toggle(
            "Show membership changes",
            "Show joins, leaves, kicks, and bans as compact lines in the timeline. \
             Turn off to hide them entirely — handy in rooms bridged to IRC, where \
             join/leave churn is constant.",
            chat.show_membership_events,
            Message::ShowMembershipEventsToggled,
        ),
    ]
    .spacing(12);

    let log_status_text: Element<'_, Message> = match &state.log_copy_status {
        LogCopyStatus::Idle => text("").size(12).into(),
        LogCopyStatus::Copying => text("Copying…").size(12).style(text::secondary).into(),
        LogCopyStatus::Copied { bytes } => {
            text(format!("Copied ({bytes} bytes) — paste it wherever you're sending it."))
                .size(12)
                .style(text::secondary)
                .into()
        }
        LogCopyStatus::Failed(error) => {
            text(format!("Couldn't copy the log: {error}")).size(12).style(text::danger).into()
        }
    };
    let diagnostics_section = column![
        text("Diagnostics").size(14).font(crate::theme::SEMIBOLD_FONT),
        column![
            row![
                text("Log file").size(13).width(Length::Fill),
                button(text("Copy log to clipboard").size(13))
                    .on_press(Message::CopyLogRequested)
                    .style(crate::theme::ghost_button)
                    .padding([6, 12]),
            ]
            .spacing(8)
            .align_y(iced::Center),
            log_status_text,
        ]
        .spacing(4),
    ]
    .spacing(6);

    column![
        account_section,
        sign_out_section,
        autostart_section,
        spelling_section,
        timeline_section,
        diagnostics_section,
    ]
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
