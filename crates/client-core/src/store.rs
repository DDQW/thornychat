use std::path::PathBuf;

use crate::error::{CoreError, CoreResult};

/// Resolves `%APPDATA%\Synapse\<profile>\...` paths. `profile` allows
/// multiple accounts to keep fully separate SQLite stores; defaults to
/// "default" for a single-account install.
pub struct AppPaths {
    pub root: PathBuf,
}

impl AppPaths {
    pub fn for_profile(profile: &str) -> CoreResult<Self> {
        let base = directories::ProjectDirs::from("me", "Synapse", "Synapse")
            .ok_or_else(|| CoreError::Other("could not resolve %APPDATA% directory".into()))?;
        let root = base.data_dir().join(profile);
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    pub fn state_store_dir(&self) -> PathBuf {
        self.root.join("store")
    }

    pub fn media_cache_dir(&self) -> PathBuf {
        self.root.join("media-cache")
    }

    /// Cache for Twemoji SVGs fetched for unicode emoji rendering — a
    /// separate directory from `media_cache_dir` since it holds
    /// non-Matrix, non-account-scoped assets that could in principle be
    /// shared across profiles (kept per-profile for simplicity today).
    pub fn emoji_cache_dir(&self) -> PathBuf {
        self.root.join("emoji-cache")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }
}
