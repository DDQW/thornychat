//! The user-configurable theme: colors, typography, and density. Persisted
//! as a single shareable JSON file so a theme can be handed to someone else
//! and dropped back in via the Appearance settings tab.

use std::path::PathBuf;

use client_core::store::AppPaths;
use iced::theme::palette::{self, Pair};
use iced::theme::Palette;
use iced::{Color, Theme};
use serde::{Deserialize, Serialize};

/// A color stored as `#RRGGBB`/`#RRGGBBAA` hex in JSON — human-editable and
/// diffable, unlike `iced::Color`'s raw f32 components.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ThemeColor(Color);

impl ThemeColor {
    pub fn new(r: u8, g: u8, b: u8) -> Self {
        Self(Color::from_rgb8(r, g, b))
    }

    pub fn color(self) -> Color {
        self.0
    }

    pub fn to_hex(self) -> String {
        let [r, g, b, a] = self.0.into_rgba8();
        if a == 255 {
            format!("#{r:02X}{g:02X}{b:02X}")
        } else {
            format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
        }
    }

    /// Accepts `#RGB`/`RGB`/`#RRGGBB`/`RRGGBB`/`#RRGGBBAA`/`RRGGBBAA`. Guards
    /// on `is_ascii()` before byte-slicing — a pasted non-ASCII string that
    /// happens to land on 6 or 8 bytes could otherwise slice mid-character
    /// and panic.
    pub fn parse_hex(input: &str) -> Option<Self> {
        let hex = input.trim().trim_start_matches('#');
        if !hex.is_ascii() {
            return None;
        }
        match hex.len() {
            3 => {
                let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
                let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
                let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
                Some(Self(Color::from_rgb8(r, g, b)))
            }
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some(Self(Color::from_rgb8(r, g, b)))
            }
            8 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
                Some(Self(Color::from_rgba8(r, g, b, f32::from(a) / 255.0)))
            }
            _ => None,
        }
    }
}

impl From<ThemeColor> for String {
    fn from(color: ThemeColor) -> Self {
        color.to_hex()
    }
}

impl TryFrom<String> for ThemeColor {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse_hex(&value).ok_or_else(|| format!("invalid hex color: {value:?}"))
    }
}

/// The full set of user-adjustable appearance knobs. Colors cover the UI
/// chrome only — brand colors (Steam cards, video-platform icons) and the
/// always-dark lightbox/video-overlay backdrop are deliberately excluded, as
/// is the timeline's sender-name hash palette (a known residual risk against
/// unusual custom backgrounds, not addressed here).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    pub name: String,
    pub dark: bool,

    pub background: ThemeColor,
    /// Panels, sidebar, header bar — was an auto-derived weak mix of
    /// `background`/`text`, now explicit so it can be tuned for contrast.
    pub surface: ThemeColor,
    /// Hover / selected-row backgrounds — was an auto-derived strong mix.
    pub surface_strong: ThemeColor,
    pub text: ThemeColor,
    /// Secondary/timestamp text.
    pub muted_text: ThemeColor,
    /// Buttons, links, unread badges.
    pub accent: ThemeColor,
    /// Text drawn on top of accent-colored surfaces (e.g. unread pill text).
    pub accent_text: ThemeColor,
    pub success: ThemeColor,
    pub danger: ThemeColor,

    /// `None` keeps iced's default font. Takes effect on next launch —
    /// iced's `.default_font()` is a static builder-time setting, not a
    /// reactive one like `.theme()`/`.scale_factor()`.
    pub font_family: Option<String>,
    /// 0.8-1.5, default 1.0. Wired to iced's window `scale_factor`, so it
    /// scales text/padding/icons/images uniformly and takes effect live.
    pub ui_scale: f32,
    /// 0-16 logical pixels, default 6. Applied live via a small synced
    /// global (see `theme::sync_corner_radius`) rather than threaded through
    /// every style-function call site.
    pub corner_radius: f32,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self::synapse_dark()
    }
}

impl ThemeConfig {
    pub fn synapse_dark() -> Self {
        Self {
            name: "Synapse Dark".into(),
            dark: true,
            background: ThemeColor::new(0x20, 0x20, 0x20),
            surface: ThemeColor::new(0x2A, 0x2A, 0x2A),
            surface_strong: ThemeColor::new(0x36, 0x36, 0x36),
            text: ThemeColor::new(0xF3, 0xF3, 0xF3),
            muted_text: ThemeColor::new(0xA6, 0xA6, 0xA6),
            accent: ThemeColor::new(0x00, 0x78, 0xD4),
            accent_text: ThemeColor::new(0xFF, 0xFF, 0xFF),
            success: ThemeColor::new(0x0F, 0x7B, 0x0F),
            danger: ThemeColor::new(0xC4, 0x2B, 0x1C),
            font_family: None,
            ui_scale: 1.0,
            corner_radius: 6.0,
        }
    }

    pub fn synapse_light() -> Self {
        Self {
            name: "Synapse Light".into(),
            dark: false,
            background: ThemeColor::new(0xF3, 0xF3, 0xF3),
            surface: ThemeColor::new(0xE4, 0xE4, 0xE4),
            surface_strong: ThemeColor::new(0xD6, 0xD6, 0xD6),
            text: ThemeColor::new(0x20, 0x20, 0x20),
            muted_text: ThemeColor::new(0x5C, 0x5C, 0x5C),
            accent: ThemeColor::new(0x00, 0x78, 0xD4),
            accent_text: ThemeColor::new(0xFF, 0xFF, 0xFF),
            success: ThemeColor::new(0x0F, 0x7B, 0x0F),
            danger: ThemeColor::new(0xC4, 0x2B, 0x1C),
            font_family: None,
            ui_scale: 1.0,
            corner_radius: 6.0,
        }
    }

    /// Builds an `iced::Theme` via a custom `Palette -> Extended` generator
    /// (`Theme::custom_with_fn`) instead of iced's automatic HSL-mix
    /// derivation. Weak/strong primary and all of success/danger still come
    /// from iced's own derivation (sound, WCAG-readable-text-aware via
    /// `Pair::new`'s `readable()` fallback) — only the specific roles this
    /// theme exposes explicit control over are overridden.
    pub fn to_iced_theme(&self) -> Theme {
        let base = Palette {
            background: self.background.color(),
            text: self.text.color(),
            primary: self.accent.color(),
            success: self.success.color(),
            danger: self.danger.color(),
        };
        let text = self.text.color();
        let surface = self.surface.color();
        let surface_strong = self.surface_strong.color();
        let muted_text = self.muted_text.color();
        let accent_text = self.accent_text.color();

        Theme::custom_with_fn(self.name.clone(), base, move |palette| {
            let mut extended = palette::Extended::generate(palette);
            extended.background.base = Pair::new(palette.background, text);
            extended.background.weak = Pair::new(surface, text);
            extended.background.strong = Pair::new(surface_strong, text);
            // `Secondary` has no independent user-facing role today — it's
            // repurposed as a flat "muted text" hook, since iced's
            // `Extended` has no dedicated slot for that. Overriding both
            // fields of every `Pair` covers either read pattern: iced's own
            // `text::secondary` helper reads `.strong.color` directly as a
            // foreground color, while a hypothetical background-styled
            // consumer would read `.text`. Setting both to the same flat
            // value (not `Pair::new`, which would run this through
            // `readable()`) means the color the user picked is used
            // verbatim rather than silently auto-corrected.
            let muted = Pair { color: muted_text, text: muted_text };
            extended.secondary.base = muted;
            extended.secondary.weak = muted;
            extended.secondary.strong = muted;
            extended.primary.base = Pair::new(palette.primary, accent_text);
            extended
        })
    }

    /// `%APPDATA%\Synapse\Synapse\config\theme.json` — profile-independent,
    /// since the theme applies across every account on the machine.
    pub fn theme_path() -> Option<PathBuf> {
        AppPaths::global_config_dir().ok().map(|dir| dir.join("theme.json"))
    }

    pub fn load() -> Option<Self> {
        let path = Self::theme_path()?;
        let contents = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&contents).ok()
    }

    pub fn load_or_default() -> Self {
        Self::load().unwrap_or_default()
    }

    pub fn to_json_pretty(&self) -> Option<String> {
        serde_json::to_string_pretty(self).ok()
    }
}
