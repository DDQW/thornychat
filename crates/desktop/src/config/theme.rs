//! The user-configurable theme: colors, typography, and density. Persisted
//! as a single shareable JSON file so a theme can be handed to someone else
//! and dropped back in via the Appearance settings tab. The frontend turns
//! these values into CSS custom properties.

use std::path::PathBuf;

use client_core::store::AppPaths;
use serde::{Deserialize, Serialize};

/// A color stored as `#RRGGBB`/`#RRGGBBAA` hex in JSON — human-editable and
/// diffable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ThemeColor {
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

impl ThemeColor {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub fn to_hex(self) -> String {
        let Self { r, g, b, a } = self;
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
        let byte = |range: std::ops::Range<usize>| u8::from_str_radix(&hex[range], 16).ok();
        match hex.len() {
            3 => {
                let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
                let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
                let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
                Some(Self::new(r, g, b))
            }
            6 => Some(Self::new(byte(0..2)?, byte(2..4)?, byte(4..6)?)),
            8 => Some(Self { r: byte(0..2)?, g: byte(2..4)?, b: byte(4..6)?, a: byte(6..8)? }),
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
/// always-dark lightbox backdrop are deliberately excluded, as is the
/// timeline's sender-name hash palette.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    pub name: String,
    pub dark: bool,

    pub background: ThemeColor,
    /// Panels, sidebar, header bar.
    pub surface: ThemeColor,
    /// Hover / selected-row backgrounds.
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
    /// `/me` action-message ("emote") text. The sender's name keeps its own
    /// per-user color; only the action text uses this tint.
    pub emote: ThemeColor,

    /// `None` uses the system UI font.
    pub font_family: Option<String>,
    /// 0.8-1.5, default 1.0. Applied as the webview zoom, so text, padding,
    /// icons and images scale together.
    pub ui_scale: f32,
    /// 0-16 logical pixels.
    pub corner_radius: f32,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self::thornychat_dark()
    }
}

impl ThemeConfig {
    /// "Midnight" — a Cinny-matched dark theme. Neutrals are true (non-blue-
    /// tinted) grays with evenly-stepped elevation (bg → surface →
    /// surface_strong). The accent is a soft lavender rather than a saturated
    /// blue, and success/danger are lifted pastels to stay legible on a dark
    /// ground.
    pub fn thornychat_dark() -> Self {
        Self {
            name: "ThornyChat Dark".into(),
            dark: true,
            background: ThemeColor::new(0x1A, 0x1A, 0x1A),
            surface: ThemeColor::new(0x26, 0x26, 0x26),
            surface_strong: ThemeColor::new(0x33, 0x33, 0x33),
            text: ThemeColor::new(0xF2, 0xF2, 0xF2),
            muted_text: ThemeColor::new(0x99, 0x99, 0x99),
            accent: ThemeColor::new(0xBD, 0xB6, 0xEC),
            accent_text: ThemeColor::new(0x2C, 0x28, 0x43),
            success: ThemeColor::new(0x85, 0xE0, 0xBA),
            danger: ThemeColor::new(0xE6, 0x9D, 0x9D),
            emote: ThemeColor::new(0xC9, 0xA9, 0xE0),
            font_family: None,
            ui_scale: 1.0,
            corner_radius: 10.0,
        }
    }

    /// "Daylight" — the light counterpart: near-white surfaces, near-black
    /// text, and a deep blue accent.
    pub fn thornychat_light() -> Self {
        Self {
            name: "ThornyChat Light".into(),
            dark: false,
            background: ThemeColor::new(0xF2, 0xF2, 0xF2),
            surface: ThemeColor::new(0xFF, 0xFF, 0xFF),
            surface_strong: ThemeColor::new(0xE5, 0xE5, 0xE5),
            text: ThemeColor::new(0x00, 0x00, 0x00),
            muted_text: ThemeColor::new(0x5C, 0x5C, 0x5C),
            accent: ThemeColor::new(0x18, 0x58, 0xD5),
            accent_text: ThemeColor::new(0xFF, 0xFF, 0xFF),
            success: ThemeColor::new(0x00, 0x84, 0x4C),
            danger: ThemeColor::new(0xC4, 0x0E, 0x0E),
            emote: ThemeColor::new(0x6B, 0x4F, 0xA0),
            font_family: None,
            ui_scale: 1.0,
            corner_radius: 10.0,
        }
    }

    /// `%APPDATA%\ThornyChat\ThornyChat\config\theme.json` — profile-independent,
    /// since the theme applies across every account on the machine.
    pub fn theme_path() -> Option<PathBuf> {
        AppPaths::global_config_dir().ok().map(|dir| dir.join("theme.json"))
    }

    /// Clamps numeric knobs into their valid ranges (and replaces NaN/inf).
    /// The Appearance sliders already constrain live edits, but a hand-edited
    /// or imported theme.json can carry any value — an out-of-range or NaN
    /// `ui_scale` would render a window that's zero-sized or degenerate, with
    /// no in-app way to recover (Settings is scaled to invisibility too).
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

    /// A missing or unreadable file falls back to the dark theme.
    pub fn load_or_default() -> Self {
        super::read_json::<Self>(Self::theme_path()).map(Self::sanitized).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parses_every_documented_form() {
        let expected = ThemeColor::new(0x11, 0x22, 0x33);
        assert_eq!(ThemeColor::parse_hex("#123"), Some(expected));
        assert_eq!(ThemeColor::parse_hex("112233"), Some(expected));
        assert_eq!(ThemeColor::parse_hex(" #112233 "), Some(expected));
        let translucent = ThemeColor::parse_hex("#11223380").unwrap();
        assert_eq!(translucent.to_hex(), "#11223380");
    }

    #[test]
    fn bad_hex_is_rejected_without_panicking() {
        for bad in ["", "#12", "#GGGGGG", "ééé", "ééééé", "#1234567"] {
            assert_eq!(ThemeColor::parse_hex(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_theme_round_trips_through_json() {
        for theme in [ThemeConfig::thornychat_dark(), ThemeConfig::thornychat_light()] {
            let json = serde_json::to_string(&theme).unwrap();
            assert_eq!(serde_json::from_str::<ThemeConfig>(&json).unwrap(), theme);
        }
    }

    #[test]
    fn out_of_range_numbers_are_clamped() {
        let theme = ThemeConfig { ui_scale: f32::NAN, corner_radius: 99.0, ..Default::default() }.sanitized();
        assert_eq!(theme.ui_scale, 1.0);
        assert_eq!(theme.corner_radius, 16.0);
        let theme = ThemeConfig { ui_scale: 0.0, ..Default::default() }.sanitized();
        assert_eq!(theme.ui_scale, 0.8);
    }
}
