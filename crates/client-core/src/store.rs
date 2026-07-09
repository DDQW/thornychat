use std::path::PathBuf;

use crate::error::{CoreError, CoreResult};

/// Resolves `%APPDATA%\ThornyChat\ThornyChat\data\<profile>\...` paths.
/// `profile` allows multiple accounts to keep fully separate SQLite stores;
/// defaults to "default" for a single-account install.
pub struct AppPaths {
    pub root: PathBuf,
}

/// Resolves the per-app directory root, migrating data left behind by the
/// app's pre-rename identity ("Synapse") before anything is created under
/// the new name.
fn project_dirs() -> CoreResult<directories::ProjectDirs> {
    let dirs = directories::ProjectDirs::from("me", "ThornyChat", "ThornyChat")
        .ok_or_else(|| CoreError::Other("could not resolve %APPDATA% directory".into()))?;
    migrate_legacy_synapse_dir(&dirs);
    Ok(dirs)
}

/// One-time move of `%APPDATA%\Synapse\Synapse` (the app's name before the
/// ThornyChat rename) to `%APPDATA%\ThornyChat\ThornyChat`, carrying over
/// logins, E2EE stores, and settings. Only runs while nothing exists under
/// the new name yet — a cheap existence check on every later resolution.
fn migrate_legacy_synapse_dir(new: &directories::ProjectDirs) {
    let Some(old) = directories::ProjectDirs::from("me", "Synapse", "Synapse") else {
        return;
    };
    // data_dir is `<app root>\data`, so parent() is the whole per-app root —
    // moving it carries data and config across in one rename.
    let (Some(old_root), Some(new_root)) = (old.data_dir().parent(), new.data_dir().parent())
    else {
        return;
    };
    if new_root.exists() || !old_root.exists() {
        return;
    }
    if let Some(org_dir) = new_root.parent() {
        let _ = std::fs::create_dir_all(org_dir);
    }
    match std::fs::rename(old_root, new_root) {
        Ok(()) => {
            tracing::info!(from = %old_root.display(), to = %new_root.display(), "migrated legacy Synapse data directory");
            // The old organization folder is now empty; remove_dir refuses
            // to delete a non-empty directory, so this can't eat anything.
            if let Some(org_dir) = old_root.parent() {
                let _ = std::fs::remove_dir(org_dir);
            }
        }
        Err(error) => {
            // Most likely the old synapse.exe is still running and holds the
            // sqlite files open. Starting fresh is recoverable: the old data
            // stays put, and the move can be redone by hand.
            tracing::warn!(%error, "could not migrate legacy Synapse data directory; starting fresh");
        }
    }
}

impl AppPaths {
    pub fn for_profile(profile: &str) -> CoreResult<Self> {
        let base = project_dirs()?;
        let root = base.data_dir().join(profile);
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// Profile-independent config directory
    /// (`%APPDATA%\ThornyChat\ThornyChat\config`) — for settings that apply
    /// across every profile, like the active theme, rather than living
    /// inside any one account's data directory.
    pub fn global_config_dir() -> CoreResult<PathBuf> {
        let base = project_dirs()?;
        let dir = base.config_dir().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
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

    /// The log file most recently written to — the one the running process
    /// is currently appending to (daily rotation via `tracing_appender`,
    /// named `thornychat.log.<date>`). `None` if the log directory doesn't
    /// exist yet or holds no files.
    pub fn latest_log_file(&self) -> Option<PathBuf> {
        std::fs::read_dir(self.logs_dir())
            .ok()?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
            .max_by_key(|entry| entry.metadata().and_then(|m| m.modified()).ok())
            .map(|entry| entry.path())
    }
}
