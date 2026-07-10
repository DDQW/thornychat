//! Activity-connector preferences: which game launchers ThornyChat watches so
//! it can auto-post an IRC-style emote (`* you plays Half-Life`) into the room
//! you're currently viewing when the game you're playing changes. Persisted as
//! a small JSON file next to the other global configs
//! (`%APPDATA%\ThornyChat\ThornyChat\config\connectors.json`), profile-independent
//! because the installed launchers are a property of the machine, not any one
//! account.
//!
//! Everything ships **off**: like read receipts and typing, broadcasting what
//! you're playing is activity-sharing, so a fresh install tells other people
//! nothing until you opt in per launcher in Settings → Connectors.

use std::path::PathBuf;

use client_core::store::AppPaths;
use serde::{Deserialize, Serialize};

/// The minimum poll interval we'll honor regardless of the stored value —
/// registry/process reads are cheap, but there's no reason to hammer them, and
/// a zero/tiny value would busy-loop the timer subscription.
pub const MIN_POLL_INTERVAL_SECS: u64 = 5;

/// Which launchers to watch and how the change is announced. `Copy` (all plain
/// flags + one integer) so it can be handed by value to the fire-and-forget
/// save task and to the off-thread detection pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConnectorsConfig {
    /// Watch Steam's `RunningAppID` registry key and announce the running game.
    pub steam_enabled: bool,
    /// Watch installed GOG Galaxy games (registry) and match running processes.
    pub gog_enabled: bool,
    /// Watch installed Epic Games (manifests) and match running processes.
    pub epic_enabled: bool,
    /// Also post `* you stopped playing X` when you quit a game. Off by default
    /// — the "started/switched" line is the interesting one; the stop line is
    /// extra noise most of the time.
    pub announce_stop: bool,
    /// How often to poll for a game change, in seconds. Clamped up to
    /// [`MIN_POLL_INTERVAL_SECS`] at the point the timer is built.
    pub poll_interval_secs: u64,
}

impl Default for ConnectorsConfig {
    /// Nothing shared until opted in; a sensible 15s poll cadence.
    fn default() -> Self {
        Self {
            steam_enabled: false,
            gog_enabled: false,
            epic_enabled: false,
            announce_stop: false,
            poll_interval_secs: 15,
        }
    }
}

impl ConnectorsConfig {
    /// `%APPDATA%\ThornyChat\ThornyChat\config\connectors.json` — global, like the
    /// theme and privacy configs, since the installed launchers are a property
    /// of the machine rather than any single account.
    pub fn config_path() -> Option<PathBuf> {
        AppPaths::global_config_dir().ok().map(|dir| dir.join("connectors.json"))
    }

    pub fn load() -> Option<Self> {
        let path = Self::config_path()?;
        let contents = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&contents).ok()
    }

    /// A missing or unreadable/corrupt file falls back to the defaults — every
    /// connector off.
    pub fn load_or_default() -> Self {
        Self::load().unwrap_or_default()
    }

    pub fn to_json_pretty(&self) -> Option<String> {
        serde_json::to_string_pretty(self).ok()
    }

    /// True when at least one launcher is watched — gates the poll timer
    /// subscription so a fresh install does no polling at all.
    pub fn any_enabled(&self) -> bool {
        self.steam_enabled || self.gog_enabled || self.epic_enabled
    }
}
