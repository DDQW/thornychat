//! Timeline display preferences — local, cosmetic choices about what the
//! timeline shows, as opposed to privacy (what *others* see). Persisted as a
//! small JSON file next to the theme/privacy/spellcheck config
//! (`%APPDATA%\ThornyChat\ThornyChat\config\chat.json`), profile-independent
//! like the rest of the client's global preferences.

use std::path::PathBuf;

use client_core::store::AppPaths;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChatConfig {
    /// Show membership changes (joins, leaves, kicks, bans, invites, knocks)
    /// as compact system lines in the timeline. When off they're hidden
    /// entirely — useful in rooms bridged to IRC, where join/leave churn is
    /// constant. Ships **on**: the events are written out by default, with
    /// hiding as the opt-out.
    pub show_membership_events: bool,
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self { show_membership_events: true }
    }
}

impl ChatConfig {
    /// `%APPDATA%\ThornyChat\ThornyChat\config\chat.json` — global, like the
    /// theme, privacy, and spellcheck configs, since it's a property of this
    /// install rather than any single account.
    pub fn config_path() -> Option<PathBuf> {
        AppPaths::global_config_dir().ok().map(|dir| dir.join("chat.json"))
    }

    pub fn load() -> Option<Self> {
        let path = Self::config_path()?;
        let contents = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&contents).ok()
    }

    /// A missing or unreadable file falls back to the defaults (membership
    /// events shown).
    pub fn load_or_default() -> Self {
        Self::load().unwrap_or_default()
    }

    pub fn to_json_pretty(&self) -> Option<String> {
        serde_json::to_string_pretty(self).ok()
    }
}
