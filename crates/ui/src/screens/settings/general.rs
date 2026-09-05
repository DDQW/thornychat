//! Account info, sign-out, Windows autostart, spell-check preferences, and
//! the log diagnostics (how much is logged, copying it out, clearing it).

use iced::widget::{button, column, pick_list, row, text, toggler};
use iced::{Element, Length, Task};

use crate::chat_config::ChatConfig;
use crate::log_config::{LogConfig, LogLevel};
use crate::spellcheck_config::SpellcheckConfig;
use crate::window_config::WindowConfig;

#[derive(Debug, Clone)]
pub struct State {
    confirm_logout: bool,
    /// Second step of the "Delete log files" button. Deleting logs is not
    /// undoable and the logs are the only record of what went wrong, so it
    /// asks first — same shape as the sign-out confirmation above.
    confirm_log_clear: bool,
    autostart_enabled: bool,
    log_copy_status: LogCopyStatus,
    log_clear_status: LogClearStatus,
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

/// Feedback line under the "Delete log files" button, mirroring
/// [`LogCopyStatus`] — the deletion is blocking disk I/O done off-thread.
#[derive(Debug, Clone, Default)]
enum LogClearStatus {
    #[default]
    Idle,
    Clearing,
    Cleared(LogsCleared),
    Failed(String),
}

/// What one clear actually did. `files` counts the rotated files removed;
/// the day's active file is emptied rather than removed (see
/// [`clear_logs_task`]), so it is reported separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LogsCleared {
    files: usize,
    emptied_active: bool,
    bytes: u64,
}

impl State {
    /// Reads the real registry state rather than assuming "off". Called each
    /// time the Settings panel opens (see `Message::ToggleSettings`), so the
    /// toggle reflects reality even if autostart was removed by hand (or by
    /// an uninstaller) since it was last open.
    pub fn new() -> Self {
        Self {
            confirm_logout: false,
            confirm_log_clear: false,
            autostart_enabled: crate::platform::autostart::is_enabled(),
            log_copy_status: LogCopyStatus::Idle,
            log_clear_status: LogClearStatus::Idle,
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
    PreferIntegratedGpuToggled(bool),
    PreferDx12BackendToggled(bool),
    /// Window-config autosave finished; nothing to do (mirrors `SpellcheckSaved`).
    WindowConfigSaved,
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
    /// How much to log, from `trace` down to nothing. Only read at startup
    /// (`app::logging::init`), so it lands on the next launch.
    LogLevelSelected(LogLevel),
    /// Logging-config autosave finished; nothing to do.
    LogConfigSaved,
    /// "Delete log files" pressed, and the confirmation that follows it.
    ClearLogsRequested,
    ClearLogsConfirmed,
    ClearLogsCancelled,
    ClearLogsFinished(Result<LogsCleared, String>),
}

#[allow(clippy::too_many_arguments)]
pub fn update(
    state: &mut State,
    spellcheck: &mut SpellcheckConfig,
    chat: &mut ChatConfig,
    window: &mut WindowConfig,
    log: &mut LogConfig,
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
        Message::PreferIntegratedGpuToggled(on) => {
            // Only read at startup (`WindowConfig::apply_gpu_preference`), so
            // this takes effect on the next launch — the copy note under the
            // toggle says so. Saving reuses the geometry file's own writer.
            window.prefer_integrated_gpu = on;
            (
                Task::perform(WindowConfig::save(*window), |()| Message::WindowConfigSaved),
                super::Effect::None,
            )
        }
        Message::PreferDx12BackendToggled(on) => {
            // Same story as the GPU preference: only read at startup, so this
            // lands on the next launch.
            window.prefer_dx12_backend = on;
            (
                Task::perform(WindowConfig::save(*window), |()| Message::WindowConfigSaved),
                super::Effect::None,
            )
        }
        Message::WindowConfigSaved => (Task::none(), super::Effect::None),
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
        Message::LogLevelSelected(level) => {
            log.level = level;
            (Task::perform(LogConfig::save(*log), |()| Message::LogConfigSaved), super::Effect::None)
        }
        Message::LogConfigSaved => (Task::none(), super::Effect::None),
        Message::ClearLogsRequested => {
            state.confirm_log_clear = true;
            (Task::none(), super::Effect::None)
        }
        Message::ClearLogsCancelled => {
            state.confirm_log_clear = false;
            (Task::none(), super::Effect::None)
        }
        Message::ClearLogsConfirmed => {
            state.confirm_log_clear = false;
            state.log_clear_status = LogClearStatus::Clearing;
            // A copy that was reported before the clear now describes a file
            // that no longer holds any of it; drop the stale line.
            state.log_copy_status = LogCopyStatus::Idle;
            (clear_logs_task(profile.to_string()), super::Effect::None)
        }
        Message::ClearLogsFinished(result) => {
            state.log_clear_status = match result {
                Ok(cleared) => LogClearStatus::Cleared(cleared),
                Err(error) => LogClearStatus::Failed(error),
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

/// Deletes this profile's log files.
///
/// The day's active file is emptied rather than deleted, and that is not a
/// detail worth hiding: the running process holds it open through the
/// non-blocking appender, and the appender only opens a new file at the next
/// daily rollover. Unlinking it would leave every line written between now and
/// tomorrow going to a file with no name — the log would look like it had
/// simply stopped. Truncating keeps the same handle valid; the appender writes
/// in append mode, so it carries on at the new end of the file, which is zero.
///
/// Only files whose name starts with the appender's own prefix are touched, so
/// a stray file someone dropped in the directory is left alone (same posture
/// as the appender's own retention pruning).
///
/// Blocking disk I/O, so it runs inside `spawn_blocking` like `copy_log_task`.
fn clear_logs_task(profile: String) -> Task<Message> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                let paths = client_core::store::AppPaths::for_profile(&profile)
                    .map_err(|error| format!("couldn't resolve app data directory: {error}"))?;
                let cleared = clear_log_files(&paths.logs_dir(), paths.latest_log_file().as_deref())?;
                // Logged after the truncation, so the emptied file says why it
                // is empty — a log that starts mid-session otherwise reads like
                // a crash. Nothing is written if logging is off, which is fine:
                // there was nothing to clear either.
                tracing::info!(
                    files = cleared.files,
                    bytes = cleared.bytes,
                    "log files cleared from Settings"
                );
                Ok(cleared)
            })
            .await
            .unwrap_or_else(|error| Err(format!("clear task panicked: {error}")))
        },
        Message::ClearLogsFinished,
    )
}

/// The file walk behind [`clear_logs_task`], split out so it can be tested
/// against a real directory without a profile or a running app.
///
/// `active` is the file the appender currently holds open, if any.
fn clear_log_files(logs_dir: &std::path::Path, active: Option<&std::path::Path>) -> Result<LogsCleared, String> {
    let entries = match std::fs::read_dir(logs_dir) {
        Ok(entries) => entries,
        // Nothing has been logged yet (or logging is off and the directory was
        // never created): a no-op, not a failure.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LogsCleared::default())
        }
        Err(error) => return Err(format!("couldn't read {}: {error}", logs_dir.display())),
    };

    let mut cleared = LogsCleared::default();
    let mut failures: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let path = entry.path();
        let is_ours = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(client_core::store::LOG_FILE_PREFIX));
        if !is_ours {
            continue;
        }
        let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);

        let outcome = if Some(path.as_path()) == active {
            std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&path)
                .map(|_| cleared.emptied_active = true)
        } else {
            std::fs::remove_file(&path).map(|()| cleared.files += 1)
        };
        match outcome {
            Ok(()) => cleared.bytes += size,
            Err(error) => failures.push(format!(
                "{}: {error}",
                path.file_name().unwrap_or(path.as_os_str()).to_string_lossy()
            )),
        }
    }

    if failures.is_empty() {
        Ok(cleared)
    } else {
        Err(failures.join("; "))
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
    window: &'a WindowConfig,
    log: &'a LogConfig,
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

    // Machine-level, like autostart above, and deliberately not part of the
    // shareable theme file: which GPU to draw on is a property of this PC.
    let graphics_section = column![
        text("Graphics").size(14).font(crate::theme::SEMIBOLD_FONT),
        spell_toggle(
            "Prefer the integrated GPU",
            "Draw on the low-power graphics chip instead of the dedicated card. On a laptop this \
             is the difference between the discrete GPU staying awake all day and never spinning \
             up at all. Machines with only one GPU are unaffected. Takes effect the next time \
             you start ThornyChat.",
            window.prefer_integrated_gpu,
            Message::PreferIntegratedGpuToggled,
        ),
        spell_toggle(
            "Use Direct3D 12",
            "Draw through Windows' own graphics API instead of loading the Vulkan and OpenGL \
             driver stacks alongside it. Measured here: 8 fewer threads, 27 MB less memory and \
             53 MB less video memory, with no change in CPU. Takes effect the next time you \
             start ThornyChat.",
            window.prefer_dx12_backend,
            Message::PreferDx12BackendToggled,
        ),
    ]
    .spacing(6);

    let spelling_section = column![
        text("Spelling").size(14).font(crate::theme::SEMIBOLD_FONT),
        spell_toggle(
            "Check spelling",
            "Mark misspelled words in red as you type, and offer fixes for the one \
             you click into. Uses the Windows spell checker and your personal \
             dictionary; nothing changes until you pick a suggestion.",
            spellcheck.enabled,
            Message::SpellcheckToggled,
        ),
        spell_toggle(
            "Autocorrect",
            "Fix a misspelled word the moment you finish it with a space. Press \
             Backspace right afterwards to get back what you typed.",
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
    let log_clear_status_text: Element<'_, Message> = match &state.log_clear_status {
        LogClearStatus::Idle => text("").size(12).into(),
        LogClearStatus::Clearing => text("Deleting…").size(12).style(text::secondary).into(),
        LogClearStatus::Cleared(cleared) => {
            text(describe_clear(*cleared)).size(12).style(text::secondary).into()
        }
        LogClearStatus::Failed(error) => {
            text(format!("Couldn't delete the logs: {error}")).size(12).style(text::danger).into()
        }
    };

    // Two steps, like the sign-out button: the logs are the only account of
    // what the app has been doing, and there is no undo.
    let clear_control: Element<'_, Message> = if state.confirm_log_clear {
        row![
            text("Delete every log file?").size(13),
            button(text("Yes").size(13))
                .on_press(Message::ClearLogsConfirmed)
                .style(crate::theme::ghost_button)
                .padding([6, 10]),
            button(text("No").size(13))
                .on_press(Message::ClearLogsCancelled)
                .style(crate::theme::ghost_button)
                .padding([6, 10]),
        ]
        .spacing(8)
        .align_y(iced::Center)
        .into()
    } else {
        button(text("Delete log files").size(13))
            .on_press(Message::ClearLogsRequested)
            .style(crate::theme::ghost_button)
            .padding([6, 12])
            .into()
    };

    let diagnostics_section = column![
        text("Diagnostics").size(14).font(crate::theme::SEMIBOLD_FONT),
        column![
            row![
                column![
                    text("Detail level").size(13),
                    text(
                        "How much ThornyChat writes to its log files. \"Off\" writes nothing at \
                         all and creates no files — the app still recovers on its own if the \
                         display wedges. Measured here against a live account: \"Detailed\" writes \
                         about 5x as much as \"Normal\" and \"Everything\" about 20x, nearly all \
                         of it from the Matrix library — they are for chasing a specific bug, \
                         not for every day. Takes effect the next time you start ThornyChat."
                    )
                    .size(11)
                    .style(text::secondary),
                ]
                .spacing(2)
                .width(Length::Fill),
                pick_list(LogLevel::ALL, Some(log.level), Message::LogLevelSelected)
                    .text_size(13)
                    .padding([6, 10]),
            ]
            .spacing(12)
            .align_y(iced::Center),
        ]
        .spacing(4),
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
        column![
            row![
                column![
                    text("Clear the log").size(13),
                    text(
                        "Delete every stored log file and empty the one in use. Do this before \
                         sharing a log, or to reclaim the space it is taking up."
                    )
                    .size(11)
                    .style(text::secondary),
                ]
                .spacing(2)
                .width(Length::Fill),
                clear_control,
            ]
            .spacing(12)
            .align_y(iced::Center),
            log_clear_status_text,
        ]
        .spacing(4),
    ]
    .spacing(12);

    column![
        account_section,
        sign_out_section,
        autostart_section,
        graphics_section,
        spelling_section,
        timeline_section,
        diagnostics_section,
    ]
    .spacing(20)
    .into()
}

/// The result line after a clear. Reports the file still being written
/// separately from the ones that are gone: it is emptied rather than deleted,
/// so a log directory that still holds a file afterwards is expected, not a
/// button that half-worked.
fn describe_clear(cleared: LogsCleared) -> String {
    if cleared.files == 0 && !cleared.emptied_active {
        return "Nothing to delete — there are no log files.".to_string();
    }
    let files = match cleared.files {
        0 => "No stored files to delete".to_string(),
        1 => "Deleted 1 log file".to_string(),
        count => format!("Deleted {count} log files"),
    };
    let active = if cleared.emptied_active { ", emptied today's" } else { "" };
    format!("{files}{active} — {} freed.", format_bytes(cleared.bytes))
}

/// Byte counts at a glance; a log archive runs from a few kilobytes to
/// gigabytes (see `docs/log-archive-findings-2026-09.md`), and raw bytes are
/// unreadable at the top of that range.
fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let bytes = bytes as f64;
    if bytes < KIB {
        format!("{bytes:.0} bytes")
    } else if bytes < KIB * KIB {
        format!("{:.1} KB", bytes / KIB)
    } else if bytes < KIB * KIB * KIB {
        format!("{:.1} MB", bytes / (KIB * KIB))
    } else {
        format!("{:.2} GB", bytes / (KIB * KIB * KIB))
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    /// Fresh directory under the OS temp dir, removed on drop (same helper
    /// shape as `client_core::media`'s eviction tests).
    struct TempLogsDir(PathBuf);

    impl TempLogsDir {
        fn new(tag: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("thornychat-logs-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn write(&self, name: &str, len: usize) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, vec![b'x'; len]).unwrap();
            path
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempLogsDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The whole point of the button: rotated files go, and the space they
    /// took is reported.
    #[test]
    fn rotated_files_are_deleted() {
        let dir = TempLogsDir::new("rotated");
        dir.write("thornychat.log.2026-09-01", 100);
        dir.write("thornychat.log.2026-09-02", 200);

        let cleared = clear_log_files(dir.path(), None).expect("clear should succeed");

        assert_eq!(cleared.files, 2);
        assert!(!cleared.emptied_active);
        assert_eq!(cleared.bytes, 300);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    /// The file the appender still holds open must survive as an empty file:
    /// unlinking it would send every line written before tomorrow's rollover
    /// to a file with no name.
    #[test]
    fn the_active_file_is_emptied_not_removed() {
        let dir = TempLogsDir::new("active");
        dir.write("thornychat.log.2026-09-03", 500);
        let active = dir.write("thornychat.log.2026-09-04", 900);

        let cleared = clear_log_files(dir.path(), Some(&active)).expect("clear should succeed");

        assert_eq!(cleared.files, 1, "only the rotated file is removed");
        assert!(cleared.emptied_active);
        assert_eq!(cleared.bytes, 1400, "both files' contents are reclaimed");
        assert!(active.exists(), "the appender's file must keep its name");
        assert_eq!(std::fs::metadata(&active).unwrap().len(), 0);
    }

    /// The reason the active file is truncated rather than deleted, checked
    /// against the real filesystem: `tracing_appender` holds it open with
    /// `OpenOptions::append(true).create(true)` (`rolling.rs`), and Windows
    /// has opinions about files that are open. Truncating through a second
    /// handle has to succeed, and the appender's next write has to land at the
    /// new start of the file rather than back at the old offset.
    #[test]
    fn truncating_works_while_the_appender_holds_the_file_open() {
        use std::io::Write;

        let dir = TempLogsDir::new("open-handle");
        let active = dir.write("thornychat.log.2026-09-04", 0);
        let mut appender = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&active)
            .expect("open like tracing-appender does");
        appender.write_all(b"before the clear
").unwrap();
        appender.flush().unwrap();

        let cleared = clear_log_files(dir.path(), Some(&active)).expect("clear should succeed");
        assert!(cleared.emptied_active);

        appender.write_all(b"after the clear
").unwrap();
        appender.flush().unwrap();

        assert_eq!(
            std::fs::read_to_string(&active).unwrap(),
            "after the clear
",
            "the appender must carry on writing from the truncated start"
        );
    }

    /// Same posture as the appender's own retention pruning: this deletes the
    /// app's log files, not whatever else happens to sit in the directory.
    #[test]
    fn foreign_files_are_left_alone() {
        let dir = TempLogsDir::new("foreign");
        dir.write("thornychat.log.2026-09-01", 10);
        let other = dir.write("notes.txt", 10);
        let subdir = dir.path().join("thornychat.log.d");
        std::fs::create_dir(&subdir).unwrap();

        let cleared = clear_log_files(dir.path(), None).expect("clear should succeed");

        assert_eq!(cleared.files, 1);
        assert!(other.exists(), "a file that isn't ours must not be deleted");
        assert!(subdir.exists(), "a directory must not be deleted, prefix or not");
    }

    /// Logging off (or a first run) means no directory at all, and pressing
    /// the button then is a no-op rather than an error.
    #[test]
    fn a_missing_directory_is_not_a_failure() {
        let dir = TempLogsDir::new("missing");
        let absent = dir.path().join("never-created");

        let cleared = clear_log_files(&absent, None).expect("a missing directory is fine");

        assert_eq!(cleared, LogsCleared::default());
        assert_eq!(describe_clear(cleared), "Nothing to delete — there are no log files.");
    }

    #[test]
    fn the_result_line_says_what_happened() {
        let both = LogsCleared { files: 3, emptied_active: true, bytes: 2048 };
        assert_eq!(describe_clear(both), "Deleted 3 log files, emptied today's — 2.0 KB freed.");

        let only_active = LogsCleared { files: 0, emptied_active: true, bytes: 512 };
        assert_eq!(
            describe_clear(only_active),
            "No stored files to delete, emptied today's — 512 bytes freed."
        );

        let one = LogsCleared { files: 1, emptied_active: false, bytes: 1_048_576 };
        assert_eq!(describe_clear(one), "Deleted 1 log file — 1.0 MB freed.");
    }

    /// A cleared archive can be anything from a few kilobytes to the 17 GB
    /// day in `docs/log-archive-findings-2026-09.md`.
    #[test]
    fn byte_counts_stay_readable_at_every_scale() {
        assert_eq!(format_bytes(0), "0 bytes");
        assert_eq!(format_bytes(999), "999 bytes");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(17 * 1024 * 1024 * 1024), "17.00 GB");
    }
}
