//! Activity-connector preferences: which game launchers ThornyChat watches so
//! it can auto-post an IRC-style emote (`* you plays Half-Life`) into the room
//! you're currently viewing when the game you're playing changes. Persisted as
//! `connectors.json`, profile-independent because the installed launchers are
//! a property of the machine, not any one account.
//!
//! Everything ships **off**: like read receipts and typing, broadcasting what
//! you're playing is activity-sharing, so a fresh install tells other people
//! nothing until you opt in per launcher in Settings → Connectors.

use std::path::PathBuf;

use client_core::store::AppPaths;
use serde::{Deserialize, Serialize};

/// The minimum poll interval we'll honor regardless of the stored value —
/// registry/process reads are cheap, but there's no reason to hammer them, and
/// a zero/tiny value would busy-loop the poll timer.
pub const MIN_POLL_INTERVAL_SECS: u64 = 5;

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
    /// [`MIN_POLL_INTERVAL_SECS`].
    pub poll_interval_secs: u64,
}

impl Default for ConnectorsConfig {
    /// Nothing shared until opted in; a 30s poll cadence.
    ///
    /// Thirty rather than fifteen because the GOG/Epic path takes a full
    /// `CreateToolhelp32Snapshot` of every process on the machine, and this
    /// timer runs for as long as the app does — on a client meant to stay
    /// open for days, halving that is worth more than halving the delay on a
    /// "now playing" line nobody is waiting on.
    fn default() -> Self {
        Self {
            steam_enabled: false,
            gog_enabled: false,
            epic_enabled: false,
            announce_stop: false,
            poll_interval_secs: 30,
        }
    }
}

impl ConnectorsConfig {
    pub fn config_path() -> Option<PathBuf> {
        AppPaths::global_config_dir().ok().map(|dir| dir.join("connectors.json"))
    }

    /// A missing or corrupt file falls back to the defaults — every connector
    /// off.
    pub fn load_or_default() -> Self {
        super::read_json::<Self>(Self::config_path()).unwrap_or_default().sanitized()
    }

    /// Raises a hand-edited or stale poll interval to the floor.
    pub fn sanitized(mut self) -> Self {
        self.poll_interval_secs = self.poll_interval_secs.max(MIN_POLL_INTERVAL_SECS);
        self
    }

    /// True when at least one launcher is watched — gates the poll loop so a
    /// fresh install does no polling at all.
    pub fn any_enabled(&self) -> bool {
        self.steam_enabled || self.gog_enabled || self.epic_enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tiny_poll_interval_is_raised_to_the_floor() {
        let config = ConnectorsConfig { poll_interval_secs: 0, ..Default::default() }.sanitized();
        assert_eq!(config.poll_interval_secs, MIN_POLL_INTERVAL_SECS);
    }

    #[test]
    fn nothing_is_enabled_by_default() {
        assert!(!ConnectorsConfig::default().any_enabled());
    }
}
