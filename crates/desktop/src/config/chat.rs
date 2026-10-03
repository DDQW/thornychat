//! Timeline display preferences — local, cosmetic choices about what the
//! timeline shows, as opposed to privacy (what *others* see). Persisted as
//! `chat.json`, profile-independent like the rest of the global preferences.

use std::path::PathBuf;

use client_core::store::AppPaths;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChatConfig {
    /// Show membership changes (joins, leaves, kicks, bans, invites, knocks)
    /// as compact system lines in the timeline. When off they're hidden
    /// entirely — useful in rooms bridged to IRC, where join/leave churn is
    /// constant. Ships **on**: hiding is the opt-out.
    pub show_membership_events: bool,
    /// Member panel hidden (the header toggle), inverted so the default
    /// (`false`) means shown. Persisted so the choice survives restarts.
    pub hide_members: bool,
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self { show_membership_events: true, hide_members: false }
    }
}

impl ChatConfig {
    pub fn config_path() -> Option<PathBuf> {
        AppPaths::global_config_dir().ok().map(|dir| dir.join("chat.json"))
    }

    /// A missing or unreadable file falls back to the defaults.
    pub fn load_or_default() -> Self {
        super::read_json(Self::config_path()).unwrap_or_default()
    }
}
