//! New-conversation encryption defaults: whether direct messages and rooms
//! this client *creates* turn on end-to-end encryption. Persisted globally
//! alongside the theme/privacy config
//! (`%APPDATA%\ThornyChat\ThornyChat\config\encryption.json`), profile-independent
//! so the stance is the same across every account on the machine.
//!
//! Both default to OFF (unencrypted): that matches the plain DMs most Matrix
//! communities actually use and avoids the key-management friction of
//! encrypted rooms. Encryption is fixed when a room is created, so this only
//! affects conversations started from here on — existing ones are untouched.

use std::path::PathBuf;

use client_core::store::AppPaths;
use serde::{Deserialize, Serialize};

/// The encryption defaults the UI exposes. `Copy` (plain flags) so it can be
/// handed to fire-and-forget save tasks by value, like `PrivacyConfig`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EncryptionConfig {
    /// Turn on end-to-end encryption for direct messages this client creates.
    pub encrypt_direct_messages: bool,
    /// Turn on end-to-end encryption for (non-DM) rooms this client creates.
    pub encrypt_rooms: bool,
}

impl Default for EncryptionConfig {
    /// Unencrypted by default (see the module docs for why).
    fn default() -> Self {
        Self { encrypt_direct_messages: false, encrypt_rooms: false }
    }
}

impl EncryptionConfig {
    /// `%APPDATA%\ThornyChat\ThornyChat\config\encryption.json` — global, like the
    /// theme and privacy config.
    pub fn config_path() -> Option<PathBuf> {
        AppPaths::global_config_dir().ok().map(|dir| dir.join("encryption.json"))
    }

    pub fn load() -> Option<Self> {
        let path = Self::config_path()?;
        let contents = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&contents).ok()
    }

    /// A missing or corrupt file falls back to the unencrypted defaults.
    pub fn load_or_default() -> Self {
        Self::load().unwrap_or_default()
    }

    pub fn to_json_pretty(&self) -> Option<String> {
        serde_json::to_string_pretty(self).ok()
    }
}
