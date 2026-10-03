//! New-conversation encryption defaults: whether direct messages and rooms
//! this client *creates* turn on end-to-end encryption. Persisted globally as
//! `encryption.json`, profile-independent so the stance is the same across
//! every account on the machine.
//!
//! Both default to OFF (unencrypted): that matches the plain DMs most Matrix
//! communities actually use and avoids the key-management friction of
//! encrypted rooms. Encryption is fixed when a room is created, so this only
//! affects conversations started from here on — existing ones are untouched.

use std::path::PathBuf;

use client_core::store::AppPaths;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EncryptionConfig {
    /// Turn on end-to-end encryption for direct messages this client creates.
    pub encrypt_direct_messages: bool,
    /// Turn on end-to-end encryption for (non-DM) rooms this client creates.
    pub encrypt_rooms: bool,
}

impl EncryptionConfig {
    pub fn config_path() -> Option<PathBuf> {
        AppPaths::global_config_dir().ok().map(|dir| dir.join("encryption.json"))
    }

    /// A missing or corrupt file falls back to the unencrypted defaults.
    pub fn load_or_default() -> Self {
        super::read_json(Self::config_path()).unwrap_or_default()
    }
}
