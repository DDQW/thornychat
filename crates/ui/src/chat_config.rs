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
    /// Member panel hidden (the header 👥 toggle), inverted so the default
    /// (`false`) means shown. Persisted so the choice survives restarts, not
    /// just room switches — the panel state itself lives on the one long-
    /// lived timeline `State`.
    pub hide_members: bool,
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self { show_membership_events: true, hide_members: false }
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

    /// Writes the config file off the update thread. Both the Settings
    /// membership-events toggle and the timeline's member-panel toggle
    /// funnel through here so the write logic lives once.
    pub async fn save(self) {
        let (Some(path), Some(contents)) = (Self::config_path(), self.to_json_pretty()) else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        if let Err(error) = tokio::fs::write(path, contents).await {
            tracing::warn!(%error, "failed to save timeline settings");
        }
    }
}
