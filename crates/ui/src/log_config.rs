//! How much detail the app writes to its log files — including "nothing at
//! all". Persisted as `%APPDATA%\ThornyChat\ThornyChat\config\logging.json`
//! next to the theme/privacy/window configs (global: a property of this
//! install, not any one account).
//!
//! Read exactly once, by `logging::init` before the first line is written, so
//! a change here lands on the next launch. That is also why this type lives in
//! `ui` but is consumed from `app`: the setting belongs with the other config
//! files the Settings screen edits, while the subscriber that obeys it is
//! built in the binary crate.

use std::path::PathBuf;

use client_core::store::AppPaths;
use serde::{Deserialize, Serialize};

/// The verbosity floor for everything the app and its dependencies log.
///
/// Maps onto `tracing`'s levels one-for-one, plus [`LogLevel::Off`], which is
/// not a level at all but the absence of one: no console output, no log files,
/// nothing created on disk.
///
/// What each one costs was measured against a live account on 2026-09-04
/// (`docs/log-volume-2026-09.md`): 180 s of ordinary use wrote 86 KB at
/// `info`, 474 KB at `debug` and 1.77 MB at `trace` — 5.5x and 20.5x — with
/// nearly all of the growth coming from matrix-sdk's sliding-sync internals
/// rather than from this app. `Off` wrote nothing at all: every file in the
/// log directory was byte-identical after a full launch and sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    /// Write nothing anywhere. The render watchdog keeps running regardless —
    /// see `app::logging::init` for why that survives this setting.
    Off,
    Error,
    Warn,
    /// The shipping default: enough to reconstruct what happened after the
    /// fact without the per-frame chatter of the SDK's own debug output.
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    /// Every level, in the order the Settings picker shows them: quietest
    /// first, so "off" and "errors only" sit at the top where someone looking
    /// to turn the noise down will start.
    pub const ALL: [LogLevel; 6] =
        [LogLevel::Off, LogLevel::Error, LogLevel::Warn, LogLevel::Info, LogLevel::Debug, LogLevel::Trace];

    /// The `EnvFilter` directive for this level (`RUST_LOG` syntax). Never
    /// called for [`LogLevel::Off`] — that case skips the filter entirely
    /// rather than installing an `off` directive, because an `off` filter also
    /// silences the render watchdog (see `app::logging::init`).
    pub fn directive(self) -> &'static str {
        match self {
            LogLevel::Off => "off",
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        }
    }

    pub fn is_off(self) -> bool {
        matches!(self, LogLevel::Off)
    }
}

impl std::fmt::Display for LogLevel {
    /// What the picker shows. Plain language first, with the level name in
    /// parentheses where the two differ, so a log someone is asked to send in
    /// ("set it to debug") is still findable by that name.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            LogLevel::Off => "Off — write nothing",
            LogLevel::Error => "Errors only",
            LogLevel::Warn => "Warnings and errors",
            LogLevel::Info => "Normal (info)",
            LogLevel::Debug => "Detailed (debug)",
            LogLevel::Trace => "Everything (trace)",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LogConfig {
    pub level: LogLevel,
}

impl LogConfig {
    pub fn config_path() -> Option<PathBuf> {
        AppPaths::global_config_dir().ok().map(|dir| dir.join("logging.json"))
    }

    /// A missing, unreadable, or hand-broken file falls back to `info`. Note
    /// this runs before the subscriber exists, so a parse failure cannot be
    /// logged — falling back silently is the only option, and the Settings
    /// picker will show the level actually in force.
    pub fn load_or_default() -> Self {
        Self::config_path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|contents| serde_json::from_str::<Self>(&contents).ok())
            .unwrap_or_default()
    }

    /// Writes the config file off the update thread (same shape as
    /// `ChatConfig::save`).
    pub async fn save(self) {
        let Some(path) = Self::config_path() else { return };
        let Ok(contents) = serde_json::to_string_pretty(&self) else { return };
        if let Some(parent) = path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        if let Err(error) = tokio::fs::write(path, contents).await {
            tracing::warn!(%error, "failed to save logging settings");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The file is the app's own, but it is also a plain JSON file a user may
    /// edit by hand — an unknown level must not wipe out the rest of the
    /// config or crash the launch path.
    #[test]
    fn an_unknown_level_falls_back_to_the_default() {
        assert!(serde_json::from_str::<LogConfig>(r#"{"level":"verbose"}"#).is_err());
        assert_eq!(LogConfig::default().level, LogLevel::Info);
    }

    #[test]
    fn every_level_round_trips() {
        for level in LogLevel::ALL {
            let json = serde_json::to_string(&LogConfig { level }).expect("serialize");
            let back: LogConfig = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back.level, level, "{json} did not round trip");
        }
    }

    /// An empty object is what a file written before this setting existed
    /// would look like; `#[serde(default)]` has to fill it in.
    #[test]
    fn an_empty_file_loads_as_info() {
        let config: LogConfig = serde_json::from_str("{}").expect("empty object should parse");
        assert_eq!(config.level, LogLevel::Info);
    }

    /// `Off` is the one variant with behaviour attached rather than a
    /// directive, so the two accessors must agree about which one it is.
    #[test]
    fn only_off_is_off() {
        for level in LogLevel::ALL {
            assert_eq!(level.is_off(), level == LogLevel::Off);
            assert_eq!(level.directive() == "off", level == LogLevel::Off);
        }
    }
}
