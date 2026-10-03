//! Windows toasts for push-rule hits (mentions, DMs, keywords).
//!
//! Shown with `tauri-winrt-notification` directly rather than through
//! `tauri-plugin-notification`: the plugin (by way of notify-rust) has no say
//! in what a click does, so a toast could only ever be dismissed. Here a click
//! brings the window back and opens the conversation the toast was about.
//!
//! Windows attributes a toast to the AppUserModelID it is sent under, and only
//! shows it if that ID belongs to a Start Menu shortcut. The installer
//! (`cargo xtask installer`) creates that shortcut with the bundle identifier
//! as its ID — the same `Woelki.ThornyChat` the iced build used — so an
//! installed ThornyChat toasts as itself: its own name and icon, its own entry
//! under Settings → System → Notifications, kept in the notification centre.
//! An exe run straight from the build directory has no shortcut and borrows
//! PowerShell's registered ID instead, so toasts still appear while developing.

use std::path::Path;

use tauri::{AppHandle, Emitter};
use tauri_winrt_notification::Toast;

/// Event the page listens for to open a room; the payload is the room id.
pub const OPEN_ROOM_EVENT: &str = "open-room";

/// Shows a toast that opens `room_id` when clicked. Never blocks the caller:
/// the WinRT calls happen on a blocking thread, and a failure is only logged.
pub fn show(app: &AppHandle, room_id: String, title: String, body: String) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let app_id = app_id(&app.config().identifier, std::env::current_exe().ok().as_deref());
        let on_click = app.clone();
        let result = Toast::new(&app_id)
            .title(&title)
            .text1(&body)
            .on_activated(move |_| {
                crate::commands::show_main_window(&on_click);
                if let Err(error) = on_click.emit(OPEN_ROOM_EVENT, room_id.clone()) {
                    tracing::warn!(%error, "could not open the room a toast was about");
                }
                Ok(())
            })
            .show();
        if let Err(error) = result {
            tracing::warn!(%error, "could not show a notification");
        }
    });
}

/// The AppUserModelID to toast under: the bundle identifier, unless `exe` is a
/// cargo build output (`target\debug`, `target\<triple>\release`, …), which no
/// installed shortcut points at.
fn app_id(identifier: &str, exe: Option<&Path>) -> String {
    if exe.is_some_and(is_build_output) {
        Toast::POWERSHELL_APP_ID.to_string()
    } else {
        identifier.to_string()
    }
}

/// Whether `exe` sits in a cargo profile directory under `target`, with or
/// without a target-triple level in between.
fn is_build_output(exe: &Path) -> bool {
    let named = |path: Option<&Path>, name: &str| {
        path.and_then(Path::file_name).is_some_and(|file| file.eq_ignore_ascii_case(name))
    };
    let Some(profile) = exe.parent() else { return false };
    let is_profile = ["debug", "release"].iter().any(|name| named(Some(profile), name));
    is_profile && (named(profile.parent(), "target") || named(profile.parent().and_then(Path::parent), "target"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "Woelki.ThornyChat";

    fn id_for(exe: &str) -> String {
        app_id(ID, Some(Path::new(exe)))
    }

    #[test]
    fn an_installed_exe_toasts_as_itself() {
        assert_eq!(id_for(r"C:\Users\me\AppData\Local\ThornyChat\thornychat.exe"), ID);
        assert_eq!(id_for(r"C:\Program Files\ThornyChat\thornychat.exe"), ID);
    }

    #[test]
    fn a_build_output_borrows_powershell_s_identity() {
        for exe in [
            r"C:\src\thornychat\target\debug\thornychat.exe",
            r"C:\src\thornychat\target\release\thornychat.exe",
            // This repo builds for an explicit triple (.cargo/config.toml).
            r"C:\src\thornychat\target\x86_64-pc-windows-msvc\debug\thornychat.exe",
            r"C:\src\thornychat\TARGET\x86_64-pc-windows-msvc\Release\thornychat.exe",
        ] {
            assert_eq!(id_for(exe), Toast::POWERSHELL_APP_ID, "{exe}");
        }
    }

    #[test]
    fn a_folder_merely_called_target_is_not_a_build() {
        assert_eq!(id_for(r"C:\target\ThornyChat\thornychat.exe"), ID);
        assert_eq!(id_for(r"C:\Users\me\debug\thornychat.exe"), ID);
    }

    #[test]
    fn an_unknown_exe_path_toasts_as_itself() {
        assert_eq!(app_id(ID, None), ID);
    }
}
