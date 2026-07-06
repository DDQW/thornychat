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
        Self::thornychat_dark()
    }
}

impl ThemeConfig {
    /// "Midnight" — a cool-slate dark theme. The neutrals carry a faint
    /// blue-gray tint (not pure gray) so surfaces read as deliberately
    /// layered rather than flat, with evenly-stepped elevation
    /// (bg → surface → surface_strong). The accent is a brighter, friendlier
    /// azure than Windows' `#0078D4`, and success/danger are lifted to
    /// stay legible on a dark ground.
    pub fn thornychat_dark() -> Self {
        Self {
            name: "ThornyChat Dark".into(),
            dark: true,
            background: ThemeColor::new(0x14, 0x16, 0x1C),
            surface: ThemeColor::new(0x1C, 0x1F, 0x28),
            surface_strong: ThemeColor::new(0x2A, 0x2E, 0x3A),
            text: ThemeColor::new(0xE7, 0xEA, 0xF0),
            muted_text: ThemeColor::new(0x97, 0xA0, 0xAF),
            accent: ThemeColor::new(0x5B, 0x8C, 0xF5),
            accent_text: ThemeColor::new(0xFF, 0xFF, 0xFF),
            success: ThemeColor::new(0x3F, 0xB8, 0x68),
            danger: ThemeColor::new(0xEF, 0x6B, 0x6B),
            font_family: None,
            ui_scale: 1.0,
            corner_radius: 6.0,
        }
    }

    /// "Daylight" — the light counterpart. Backgrounds are soft cool whites
    /// rather than stark `#FFFFFF`, text is a cool near-black rather than
    /// pure black (calmer, less hard-edged), and the accent deepens to a
    /// richer blue so white text stays crisp on accent-filled buttons and
    /// badges. Same blue family as Midnight for a consistent identity.
    pub fn thornychat_light() -> Self {
        Self {
            name: "ThornyChat Light".into(),
            dark: false,
            background: ThemeColor::new(0xFB, 0xFB, 0xFD),
            surface: ThemeColor::new(0xF0, 0xF2, 0xF6),
            surface_strong: ThemeColor::new(0xE2, 0xE5, 0xEC),
            text: ThemeColor::new(0x1B, 0x1E, 0x26),
            muted_text: ThemeColor::new(0x5A, 0x61, 0x70),
            accent: ThemeColor::new(0x29, 0x5F, 0xD6),
            accent_text: ThemeColor::new(0xFF, 0xFF, 0xFF),
            success: ThemeColor::new(0x1F, 0x9D, 0x57),
            danger: ThemeColor::new(0xD9, 0x3B, 0x3B),
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
            // The widgets that draw text on accent backgrounds read
            // `primary.{weak,strong}.text` (unread pill → `strong.text`,
            // selected row / call button → `weak.text`), NOT
            // `primary.base.text`. Overriding only `base` left the user's
            // "Text on accent" pick with no visible effect. Assign `.text`
            // directly (not via `Pair::new`, which would run it through
            // `readable()` and swap it for black/white on a contrast miss)
            // while keeping the generated `.color` backgrounds.
            extended.primary.base = Pair::new(palette.primary, accent_text);
            extended.primary.base.text = accent_text;
            extended.primary.weak.text = accent_text;
            extended.primary.strong.text = accent_text;
            extended
        })
    }

    /// `%APPDATA%\ThornyChat\ThornyChat\config\theme.json` — profile-independent,
    /// since the theme applies across every account on the machine.
    pub fn theme_path() -> Option<PathBuf> {
        AppPaths::global_config_dir().ok().map(|dir| dir.join("theme.json"))
    }

    /// Clamps numeric knobs into their valid ranges (and replaces NaN/inf).
    /// The Appearance sliders already constrain live edits, but a
    /// hand-edited or imported theme.json can carry any value — an out-of-
    /// range or NaN `ui_scale` feeds straight into iced's `scale_factor` and
    /// renders a window that's zero-sized or degenerate, with no in-app way
    /// to recover (Settings is scaled to invisibility too).
    pub fn sanitized(mut self) -> Self {
        if !self.ui_scale.is_finite() {
            self.ui_scale = 1.0;
        }
        self.ui_scale = self.ui_scale.clamp(0.8, 1.5);
        if !self.corner_radius.is_finite() {
            self.corner_radius = 6.0;
        }
        self.corner_radius = self.corner_radius.clamp(0.0, 16.0);
        self
    }

    pub fn load() -> Option<Self> {
        let path = Self::theme_path()?;
        let contents = std::fs::read_to_string(path).ok()?;
        serde_json::from_str::<Self>(&contents).ok().map(Self::sanitized)
    }

    pub fn load_or_default() -> Self {
        Self::load().unwrap_or_default()
    }

    pub fn to_json_pretty(&self) -> Option<String> {
        serde_json::to_string_pretty(self).ok()
    }
}
