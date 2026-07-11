//! Remembered window geometry — size, position, and maximized state,
//! persisted as `%APPDATA%\ThornyChat\ThornyChat\config\window.json` next to
//! the theme/privacy/chat configs (global: a property of this install, not
//! any one account). Loaded synchronously at startup because the values feed
//! `iced::window::Settings`, which — like the default font — is a static
//! builder-time setting, not a reactive closure.

use std::path::PathBuf;

use client_core::store::AppPaths;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    /// Logical size of the non-maximized window (what a maximized window
    /// returns to on unmaximize).
    pub width: f32,
    pub height: f32,
    /// Logical desktop coordinates of the top-left corner. `None` until the
    /// first save — the OS then places the window (iced's default).
    pub x: Option<f32>,
    pub y: Option<f32>,
    /// Reopen maximized, with `width`/`height` as the restore frame.
    pub maximized: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        // Matches `iced::window::Settings::default()`'s size, so a missing
        // file and a fresh install behave identically.
        Self { width: 1024.0, height: 768.0, x: None, y: None, maximized: false }
    }
}

impl WindowConfig {
    pub fn config_path() -> Option<PathBuf> {
        AppPaths::global_config_dir().ok().map(|dir| dir.join("window.json"))
    }

    pub fn load_or_default() -> Self {
        Self::config_path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|contents| serde_json::from_str::<Self>(&contents).ok())
            .map(Self::sanitized)
            .unwrap_or_default()
    }

    /// A hand-edited file can carry anything; non-finite or degenerate
    /// values must never reach `window::Settings` (same posture as
    /// `ThemeConfig::sanitized` for `ui_scale`).
    fn sanitized(mut self) -> Self {
        if !(self.width.is_finite() && self.height.is_finite())
            || self.width < 320.0
            || self.height < 240.0
        {
            let default = Self::default();
            self.width = default.width;
            self.height = default.height;
        }
        if !matches!((self.x, self.y), (Some(x), Some(y)) if x.is_finite() && y.is_finite()) {
            self.x = None;
            self.y = None;
        }
        self
    }

    pub fn size(&self) -> iced::Size {
        iced::Size::new(self.width, self.height)
    }

    /// The stored position, unless it would strand the window outside the
    /// virtual screen (a monitor unplugged since last run, say) — then the
    /// OS places it as if nothing were remembered.
    pub fn position(&self) -> iced::window::Position {
        match (self.x, self.y) {
            (Some(x), Some(y)) if position_reachable(x, y) => {
                iced::window::Position::Specific(iced::Point::new(x, y))
            }
            _ => iced::window::Position::default(),
        }
    }

    /// Writes the config file off the update thread (same shape as
    /// `ChatConfig::save`).
    pub async fn save(self) {
        let Some(path) = Self::config_path() else { return };
        let Ok(contents) = serde_json::to_string_pretty(&self) else { return };
        if let Some(parent) = path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        if let Err(error) = tokio::fs::write(path, contents).await {
            tracing::warn!(%error, "failed to save window geometry");
        }
    }
}

/// Loose visibility check against the Win32 virtual screen (the bounding box
/// of every attached monitor). Stored coordinates are logical while the
/// metrics are physical pixels, so the comparison is deliberately generous:
/// it only needs to catch the monitor-was-removed case, where the stored
/// point is off by an entire screen, not split hairs at the edges.
fn position_reachable(x: f32, y: f32) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };
    let (left, top, width, height) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN) as f32,
            GetSystemMetrics(SM_YVIRTUALSCREEN) as f32,
            GetSystemMetrics(SM_CXVIRTUALSCREEN) as f32,
            GetSystemMetrics(SM_CYVIRTUALSCREEN) as f32,
        )
    };
    if width <= 0.0 || height <= 0.0 {
        // Metrics unavailable — don't second-guess the stored position.
        return true;
    }
    // A point ~inside the title bar must land on some monitor, so the
    // window can always be grabbed and dragged.
    let (probe_x, probe_y) = (x + 100.0, y + 20.0);
    probe_x >= left && probe_x <= left + width && probe_y >= top && probe_y <= top + height
}
