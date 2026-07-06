//! User privacy preferences: what your activity reveals to other people and
//! to third-party servers. Persisted as a small JSON file next to the theme
//! (`%APPDATA%\ThornyChat\ThornyChat\config\privacy.json`), profile-independent so
//! the client's privacy posture is the same across every account on the
//! machine.
//!
//! Every option defaults to the most private setting — see [`Default`]. The
//! goal is that a fresh install shares nothing about you until you opt in,
//! rather than the Matrix norm of broadcasting reads/typing/presence out of
//! the box.

use std::path::PathBuf;

use client_core::store::AppPaths;
use serde::{Deserialize, Serialize};

/// The privacy knobs the UI exposes. `Copy` (all plain flags) so it can be
/// handed to fire-and-forget save tasks by value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrivacyConfig {
    /// Send *public* read receipts (`m.read`), which are federated and tell
    /// other users exactly which message you've read and when. When off, the
    /// client instead sends a *private* read receipt (`m.read.private`): your
    /// read position still advances server-side — so unread badges clear on
    /// your own devices — but it is never shared with anyone else.
    pub send_read_receipts: bool,
    /// Broadcast "typing…" notifications while you compose a message. When
    /// off, other people never see that you're typing.
    pub send_typing_notifications: bool,
    /// Fetch previews (unfurls) for links in messages. When off, neither your
    /// homeserver nor third-party sites (Twitter, Steam, their image CDNs)
    /// are contacted to expand links you receive — which would otherwise
    /// reveal your IP address and what you're reading.
    pub enable_link_previews: bool,
}

impl Default for PrivacyConfig {
    /// High privacy: every activity-sharing behavior starts disabled.
    fn default() -> Self {
        Self {
            send_read_receipts: false,
            send_typing_notifications: false,
            enable_link_previews: false,
        }
    }
}

impl PrivacyConfig {
    /// `%APPDATA%\ThornyChat\ThornyChat\config\privacy.json` — global, like the
    /// theme, since a privacy stance is a property of this install rather
    /// than any single account.
    pub fn config_path() -> Option<PathBuf> {
        AppPaths::global_config_dir().ok().map(|dir| dir.join("privacy.json"))
    }

    pub fn load() -> Option<Self> {
        let path = Self::config_path()?;
        let contents = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&contents).ok()
    }

    /// A missing or unreadable/corrupt file falls back to the private
    /// defaults — failing *open* to less sharing, never more.
    pub fn load_or_default() -> Self {
        Self::load().unwrap_or_default()
    }

    pub fn to_json_pretty(&self) -> Option<String> {
        serde_json::to_string_pretty(self).ok()
    }
}
