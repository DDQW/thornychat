//! Remembered window geometry — size, position, and maximized state,
//! persisted as `%APPDATA%\ThornyChat\ThornyChat\config\window.json` next to
//! the theme/privacy/chat configs (global: a property of this install, not
//! any one account). Loaded synchronously at startup because the values feed
//! `iced::window::Settings`, which is read when a window is opened rather than
//! watched afterwards — and this app opens more than one over its life: the
//! window is closed and reopened across standby (see `platform::power`), each
//! time from the geometry as it stands then.

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
    /// Ask wgpu for the integrated GPU rather than the discrete one.
    ///
    /// `iced_wgpu` hardcodes `PowerPreference::HighPerformance`
    /// (`window/compositor.rs`), which on a hybrid-graphics laptop binds the
    /// discrete GPU for the life of the process and stops it powering down —
    /// tens of watts, all day, to draw a chat window. A 2D UI has no use for
    /// that card. Applied in `main` by setting `WGPU_POWER_PREF`, the env var
    /// wgpu already reads.
    ///
    /// Safe to default on: wgpu ranks candidates
    /// integrated < discrete < other < virtual < CPU
    /// (`wgpu-core/src/instance.rs`, `get_order`), so a machine with only a
    /// discrete card still gets that card, and the software rasterizer
    /// ("Microsoft Basic Render Driver") sorts last under either preference
    /// and is never reachable this way.
    pub prefer_integrated_gpu: bool,
    /// Pin wgpu to the Direct3D 12 backend instead of letting it load every
    /// backend and pick one.
    ///
    /// `iced_wgpu` defaults to `Backends::all()` (`settings.rs`), so Windows
    /// loads the Vulkan stack *and* the D3D12 stack *and* OpenGL, then uses
    /// one. Which one it picks is not a considered choice: `request_adapter`
    /// sorts only by device type, and both backends report the same
    /// `DiscreteGpu`, so the tie breaks on enumeration order.
    ///
    /// Measured on this machine (RX 7900 XTX, 9x180s samples per arm, same
    /// logged-in profile, equal downtime):
    ///
    /// |            | Vulkan | Dx12  |
    /// |------------|--------|-------|
    /// | CPU        | 33.97 ms/s | 34.43 ms/s (no difference) |
    /// | threads    | 167    | 159   |
    /// | working set| 221 MB | 194 MB |
    /// | VRAM       | 176 MB | 123 MB |
    /// | modules    | 80     | 72    |
    ///
    /// DXGI and `d3d12.dll` load either way (they are on the presentation
    /// path), so Vulkan was purely additive — `vulkan-1.dll` plus the AMD ICD
    /// on top of a stack already paid for. D3D12 is also the safer thing to
    /// pin on Windows: it ships in-box and has a WARP fallback device, where a
    /// working Vulkan ICD is a driver-install detail.
    ///
    /// Note this is *not* about the `SurfaceError::Other` hang
    /// (`docs/iced-surface-error-other-hang.md`): that was written up against
    /// DX12, but it reproduced on Vulkan here on 2026-09-03 (4.5 hours,
    /// ~529k error lines), so it is not backend-specific and is not a reason
    /// to prefer either one.
    pub prefer_dx12_backend: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        // Matches `iced::window::Settings::default()`'s size, so a missing
        // file and a fresh install behave identically.
        Self {
            width: 1024.0,
            height: 768.0,
            x: None,
            y: None,
            maximized: false,
            prefer_integrated_gpu: true,
            prefer_dx12_backend: true,
        }
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

    /// Points wgpu at the low-power adapter, unless the environment already
    /// says otherwise.
    ///
    /// Must run before iced builds its compositor (wgpu reads the variable
    /// when the adapter is requested) and before any other thread exists —
    /// `set_var` is process-global and unsynchronized. `main` satisfies both.
    /// An explicit `WGPU_POWER_PREF` always wins, so the escape hatch for a
    /// user who wants the discrete card keeps working even with the setting
    /// on; the accepted values are wgpu's own (`low`, `high`, `none`).
    pub fn apply_gpu_preference(&self) {
        if self.prefer_integrated_gpu && std::env::var_os("WGPU_POWER_PREF").is_none() {
            std::env::set_var("WGPU_POWER_PREF", "low");
        }
        // Pinning one backend is what actually stops the other driver stacks
        // from loading; a comma list would keep them all and buy nothing,
        // because wgpu orders candidates by device type, not by list position.
        if self.prefer_dx12_backend && std::env::var_os("WGPU_BACKEND").is_none() {
            std::env::set_var("WGPU_BACKEND", "dx12");
        }
    }

    /// The `iced::window::Settings` a window of this app opens with.
    ///
    /// Lives here rather than on `App` so it can be built (and tested) from a
    /// config alone: the app opens windows more than once now — the one that
    /// replaces it after standby included — and each open reads the geometry
    /// as it stands at that moment.
    ///
    /// `exit_on_close_request` is off because closing the window is no longer
    /// the same thing as quitting: `platform::power` closes it deliberately
    /// before the machine sleeps and the app keeps running. The user's own
    /// close arrives as `Message::WindowCloseRequested` instead, and that one
    /// exits.
    pub fn window_settings(&self, icon: Option<iced::window::Icon>) -> iced::window::Settings {
        iced::window::Settings {
            icon,
            size: self.size(),
            position: self.position(),
            maximized: self.maximized,
            exit_on_close_request: false,
            ..Default::default()
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A `window.json` written before `prefer_integrated_gpu` existed must
    /// still load, and must opt in — `#[serde(default)]` fills the missing
    /// field from `Default`, so the low-power preference reaches existing
    /// installs rather than only new ones.
    #[test]
    fn legacy_file_without_the_gpu_field_defaults_to_preferring_integrated() {
        let legacy = r#"{
            "width": 1920.0,
            "height": 1009.0,
            "x": -16000.0,
            "y": -16000.0,
            "maximized": true
        }"#;
        let config: WindowConfig = serde_json::from_str(legacy).expect("legacy file should parse");
        assert!(config.prefer_integrated_gpu);
        assert!(config.prefer_dx12_backend);
        assert!(config.maximized);
        assert_eq!(config.width, 1920.0);
    }

    /// The one test that touches `WGPU_POWER_PREF`, so it can set and restore
    /// it without racing the rest of the suite. Covers all three arms of
    /// `apply_gpu_preference`: opted in, opted out, and overridden by the
    /// environment.
    #[test]
    fn apply_gpu_preference_respects_the_setting_and_the_environment() {
        let previous = std::env::var_os("WGPU_POWER_PREF");
        let on = WindowConfig::default();
        let off = WindowConfig {
            prefer_integrated_gpu: false,
            prefer_dx12_backend: false,
            ..WindowConfig::default()
        };

        let previous_backend = std::env::var_os("WGPU_BACKEND");

        // Opted in, nothing preset: low-power adapter, D3D12 backend.
        std::env::remove_var("WGPU_POWER_PREF");
        std::env::remove_var("WGPU_BACKEND");
        on.apply_gpu_preference();
        assert_eq!(std::env::var("WGPU_POWER_PREF").as_deref(), Ok("low"));
        assert_eq!(std::env::var("WGPU_BACKEND").as_deref(), Ok("dx12"));

        // Opted out: leaves the environment alone, so iced_wgpu's own
        // HighPerformance / Backends::all() defaults stand.
        std::env::remove_var("WGPU_POWER_PREF");
        std::env::remove_var("WGPU_BACKEND");
        off.apply_gpu_preference();
        assert!(std::env::var_os("WGPU_POWER_PREF").is_none());
        assert!(std::env::var_os("WGPU_BACKEND").is_none());

        // User-set values always win, even with the settings on.
        std::env::set_var("WGPU_POWER_PREF", "high");
        std::env::set_var("WGPU_BACKEND", "vulkan");
        on.apply_gpu_preference();
        assert_eq!(std::env::var("WGPU_POWER_PREF").as_deref(), Ok("high"));
        assert_eq!(std::env::var("WGPU_BACKEND").as_deref(), Ok("vulkan"));

        match previous {
            Some(value) => std::env::set_var("WGPU_POWER_PREF", value),
            None => std::env::remove_var("WGPU_POWER_PREF"),
        }
        match previous_backend {
            Some(value) => std::env::set_var("WGPU_BACKEND", value),
            None => std::env::remove_var("WGPU_BACKEND"),
        }
    }

    /// Two things about a window's settings are load-bearing rather than
    /// cosmetic: the geometry has to come from the config as it stands (so the
    /// window reopened after standby lands where the user left it, not where
    /// the app started), and `exit_on_close_request` has to stay off (so the
    /// suspend path can close the window without quitting the app).
    #[test]
    fn window_settings_follow_the_current_geometry() {
        let config = WindowConfig {
            width: 1234.0,
            height: 567.0,
            maximized: true,
            ..WindowConfig::default()
        };

        let settings = config.window_settings(None);

        assert_eq!(settings.size, iced::Size::new(1234.0, 567.0));
        assert!(settings.maximized);
        assert!(
            !settings.exit_on_close_request,
            "iced must not turn a close into an exit — the suspend path closes this window too"
        );
    }

    /// An explicit `false` survives a round trip — the Settings toggle has to
    /// be able to turn this back off and have it stick.
    #[test]
    fn the_gpu_preference_round_trips() {
        let off = WindowConfig { prefer_integrated_gpu: false, ..WindowConfig::default() };
        let json = serde_json::to_string(&off).expect("serialize");
        let back: WindowConfig = serde_json::from_str(&json).expect("deserialize");
        assert!(!back.prefer_integrated_gpu);
    }
}
