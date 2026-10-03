//! Tauri commands that are not part of the Matrix bridge: settings,
//! third-party cards, links, files, and small shell operations.

use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Manager, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

use crate::bridge::Bridge;
use crate::config::Settings;
use crate::embeds;

/// The settings as last saved, shared with the game-activity poller.
#[derive(Clone)]
pub struct SettingsStore(pub Arc<Mutex<Settings>>);

#[derive(Serialize)]
pub struct AppInfo {
    pub version: &'static str,
    pub profile: String,
    pub logging_enabled: bool,
}

#[tauri::command]
pub fn app_info(bridge: State<'_, Arc<Bridge>>, settings: State<'_, SettingsStore>) -> AppInfo {
    let logging_enabled = !settings.0.lock().expect("settings lock poisoned").log.level.is_off();
    AppInfo { version: env!("CARGO_PKG_VERSION"), profile: bridge.profile.clone(), logging_enabled }
}

#[tauri::command]
pub fn get_settings(settings: State<'_, SettingsStore>) -> Settings {
    settings.0.lock().expect("settings lock poisoned").clone()
}

/// The built-in themes the Appearance tab offers as starting points.
#[tauri::command]
pub fn theme_presets() -> Vec<crate::config::ThemeConfig> {
    vec![crate::config::ThemeConfig::thornychat_dark(), crate::config::ThemeConfig::thornychat_light()]
}

/// Saves the sections that changed and returns the sanitized result (the
/// frontend adopts it, so a clamped value shows up in the UI immediately).
#[tauri::command]
pub fn set_settings(
    app: AppHandle,
    settings: State<'_, SettingsStore>,
    new: Settings,
) -> Result<Settings, String> {
    let mut current = settings.0.lock().expect("settings lock poisoned");
    let saved = new.save_changes(&current).map_err(|error| format!("couldn't save settings: {error}"))?;
    if saved.theme.ui_scale != current.theme.ui_scale {
        apply_zoom(&app, saved.theme.ui_scale);
    }
    *current = saved.clone();
    Ok(saved)
}

/// The UI scale is the webview zoom, so text, spacing, icons and images scale
/// together (the iced build scaled the whole window the same way).
pub fn apply_zoom(app: &AppHandle, scale: f32) {
    if let Some(window) = app.get_webview_window("main") {
        if let Err(error) = window.set_zoom(f64::from(scale)) {
            tracing::warn!(%error, "could not apply the UI scale");
        }
    }
}

/// Resolves a tweet link through FxTwitter. `Ok(None)` means the URL isn't a
/// tweet link at all (the page falls back to the homeserver preview).
#[tauri::command]
pub async fn fetch_tweet(url: String) -> Result<Option<embeds::tweets::TweetData>, String> {
    let Some(api_url) = embeds::tweets::tweet_api_url(&url) else { return Ok(None) };
    embeds::tweets::fetch(api_url).await.map(Some).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn fetch_steam_app(url: String) -> Result<Option<embeds::steam::SteamAppData>, String> {
    let Some(api_url) = embeds::steam::steam_api_url(&url) else { return Ok(None) };
    embeds::steam::fetch(api_url).await.map(Some).map_err(|error| error.to_string())
}

/// Opens a link in the default browser. Only web and mail links: the URL comes
/// from message text, and `open::that` would happily launch a local path or a
/// registered protocol handler.
#[tauri::command]
pub fn open_external(url: String) -> Result<(), String> {
    if !is_openable_link(&url) {
        return Err("only http, https and mailto links can be opened".into());
    }
    open::that(&url).map_err(|error| format!("couldn't open the link: {error}"))
}

fn is_openable_link(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|parsed| matches!(parsed.scheme(), "http" | "https" | "mailto"))
}

/// Saves a cached/downloaded Matrix media file where the user picks, offering
/// `suggested_name` (the real filename, not the mxc id). Returns `false` when
/// the dialog was cancelled.
#[tauri::command]
pub async fn save_media(
    app: AppHandle,
    bridge: State<'_, Arc<Bridge>>,
    mxc_url: String,
    suggested_name: String,
) -> Result<bool, String> {
    let running = bridge.running().ok_or("not signed in")?;
    let source = client_core::media::fetch_file(&running.client, &bridge.media_dir, &mxc_url)
        .await
        .map_err(|error| format!("couldn't download the file: {error}"))?;

    let name = sanitize_file_name(&suggested_name);
    let destination = tauri::async_runtime::spawn_blocking(move || {
        app.dialog().file().set_file_name(&name).blocking_save_file()
    })
    .await
    .map_err(|error| error.to_string())?;
    let Some(destination) = destination else { return Ok(false) };
    let destination = destination.into_path().map_err(|error| error.to_string())?;

    tokio::fs::copy(&source, &destination)
        .await
        .map_err(|error| format!("couldn't write {}: {error}", destination.display()))?;
    Ok(true)
}

/// A filename from the network is only ever a *suggestion* in a save dialog,
/// but strip path separators and control characters anyway so the dialog
/// can't be pre-pointed at another directory.
fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').to_string();
    if trimmed.is_empty() { "file".to_string() } else { trimmed }
}

/// The small per-profile files the page keeps. A closed list rather than a
/// path, so the page can't be talked into reading or writing anywhere else —
/// and the files stay where the iced build kept them.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileFile {
    /// Emoji use counts behind the picker's "Frequently used" section.
    EmojiUsage,
    /// The grow-with-use sticker collection.
    Stickers,
    /// The room that was open when the app last ran.
    LastRoom,
}

impl ProfileFile {
    fn path(self, bridge: &Bridge) -> std::path::PathBuf {
        match self {
            ProfileFile::EmojiUsage => bridge.emoji_dir.join("usage.json"),
            ProfileFile::Stickers => bridge.emoji_dir.join("stickers.json"),
            ProfileFile::LastRoom => bridge.profile_dir.join("last-room"),
        }
    }
}

/// Upper bound on what the page may store in one profile file.
const MAX_PROFILE_FILE_BYTES: usize = 1024 * 1024;

#[tauri::command]
pub async fn load_profile_file(
    bridge: State<'_, Arc<Bridge>>,
    file: ProfileFile,
) -> Result<Option<String>, String> {
    match tokio::fs::read_to_string(file.path(&bridge)).await {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

#[tauri::command]
pub async fn save_profile_file(
    bridge: State<'_, Arc<Bridge>>,
    file: ProfileFile,
    contents: String,
) -> Result<(), String> {
    if contents.len() > MAX_PROFILE_FILE_BYTES {
        return Err("too large to store".into());
    }
    let path = file.path(&bridge);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|error| error.to_string())?;
    }
    // Temp-then-rename, like the config files: a crash mid-write must not
    // leave a truncated history file behind.
    let mut tmp_name = path.file_name().map(|name| name.to_os_string()).unwrap_or_default();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    tokio::fs::write(&tmp, contents).await.map_err(|error| error.to_string())?;
    tokio::fs::rename(&tmp, &path).await.map_err(|error| error.to_string())
}

#[tauri::command]
pub fn autostart_enabled() -> bool {
    crate::autostart::is_enabled()
}

#[tauri::command]
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    crate::autostart::set_enabled(enabled).map_err(|error| error.to_string())
}

/// The newest log file's text, for the "Copy log" button.
#[tauri::command]
pub async fn read_log(bridge: State<'_, Arc<Bridge>>) -> Result<String, String> {
    let profile = bridge.profile.clone();
    tauri::async_runtime::spawn_blocking(move || crate::diagnostics::read_latest_log(&profile))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn clear_logs(bridge: State<'_, Arc<Bridge>>) -> Result<String, String> {
    let profile = bridge.profile.clone();
    tauri::async_runtime::spawn_blocking(move || crate::diagnostics::clear_logs(&profile))
        .await
        .map_err(|error| error.to_string())?
}

/// Brings the window to the front (tray click, second launch, notification).
pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        focus(&window);
    }
}

pub fn focus(window: &WebviewWindow) {
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_and_mail_links_open() {
        for ok in ["https://example.com/a?b=c", "http://example.com", "mailto:someone@example.com"] {
            assert!(is_openable_link(ok), "{ok}");
        }
        for bad in ["file:///C:/Windows/System32/calc.exe", "javascript:alert(1)", "ms-settings:", "C:\\Windows\\notepad.exe", "\\\\server\\share", "data:text/html,hi", "", "example.com"] {
            assert!(!is_openable_link(bad), "{bad:?}");
        }
    }

    #[test]
    fn suggested_file_names_cannot_name_a_directory() {
        assert_eq!(sanitize_file_name("report.pdf"), "report.pdf");
        assert_eq!(sanitize_file_name("..\\..\\evil.exe"), "_.._evil.exe");
        assert_eq!(sanitize_file_name("a/b:c*d.txt"), "a_b_c_d.txt");
        assert_eq!(sanitize_file_name("   "), "file");
        assert_eq!(sanitize_file_name("..."), "file");
        assert_eq!(sanitize_file_name("tab\tname"), "tab_name");
    }
}
