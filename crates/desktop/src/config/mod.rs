//! Persisted, profile-independent preferences. Each section keeps the JSON
//! file it always had under `%APPDATA%\ThornyChat\ThornyChat\config`, so
//! settings survive the move from the iced UI. The frontend reads and writes
//! them as one [`Settings`] value (`get_settings` / `set_settings`).

use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub mod chat;
pub mod connectors;
pub mod encryption;
pub mod log;
pub mod privacy;
pub mod spellcheck;
pub mod theme;

pub use chat::ChatConfig;
pub use connectors::ConnectorsConfig;
pub use encryption::EncryptionConfig;
pub use log::LogConfig;
pub use privacy::PrivacyConfig;
pub use spellcheck::SpellcheckConfig;
pub use theme::ThemeConfig;

/// Everything the Settings screen edits, in one value.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeConfig,
    pub chat: ChatConfig,
    pub privacy: PrivacyConfig,
    pub encryption: EncryptionConfig,
    pub connectors: ConnectorsConfig,
    pub spellcheck: SpellcheckConfig,
    pub log: LogConfig,
}

impl Settings {
    pub fn load() -> Self {
        Self {
            theme: ThemeConfig::load_or_default(),
            chat: ChatConfig::load_or_default(),
            privacy: PrivacyConfig::load_or_default(),
            encryption: EncryptionConfig::load_or_default(),
            connectors: ConnectorsConfig::load_or_default(),
            spellcheck: SpellcheckConfig::load_or_default(),
            log: LogConfig::load_or_default(),
        }
    }

    /// Writes only the sections that differ from `previous`, so changing one
    /// toggle doesn't rewrite (and re-date) every file. Returns the sanitized
    /// value actually stored, which the caller hands back to the frontend.
    pub fn save_changes(self, previous: &Settings) -> std::io::Result<Settings> {
        let settings = Settings {
            theme: self.theme.sanitized(),
            connectors: self.connectors.sanitized(),
            ..self
        };
        if settings.theme != previous.theme {
            write_json(ThemeConfig::theme_path(), &settings.theme)?;
        }
        if settings.chat != previous.chat {
            write_json(ChatConfig::config_path(), &settings.chat)?;
        }
        if settings.privacy != previous.privacy {
            write_json(PrivacyConfig::config_path(), &settings.privacy)?;
        }
        if settings.encryption != previous.encryption {
            write_json(EncryptionConfig::config_path(), &settings.encryption)?;
        }
        if settings.connectors != previous.connectors {
            write_json(ConnectorsConfig::config_path(), &settings.connectors)?;
        }
        if settings.spellcheck != previous.spellcheck {
            write_json(SpellcheckConfig::config_path(), &settings.spellcheck)?;
        }
        if settings.log != previous.log {
            write_json(LogConfig::config_path(), &settings.log)?;
        }
        Ok(settings)
    }
}

/// Reads a config file, treating anything missing or unparseable as "use the
/// defaults" — a hand-broken file must never stop the app from starting.
pub(crate) fn read_json<T: serde::de::DeserializeOwned>(path: Option<PathBuf>) -> Option<T> {
    let contents = std::fs::read_to_string(path?).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Temp-file-then-rename, so a crash mid-write can't leave a truncated config
/// that silently resets the user's settings on the next launch.
pub(crate) fn write_json<T: Serialize>(path: Option<PathBuf>, value: &T) -> std::io::Result<()> {
    let Some(path) = path else {
        return Err(std::io::Error::other("could not resolve the config directory"));
    };
    let contents = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp_name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Settings` is the one shape the frontend sends back, so an older or
    /// partial value (a missing section) must fill in from defaults rather
    /// than fail to deserialize.
    #[test]
    fn a_partial_settings_value_fills_in_defaults() {
        let settings: Settings =
            serde_json::from_str(r#"{"privacy":{"send_read_receipts":true}}"#).unwrap();
        assert!(settings.privacy.send_read_receipts);
        assert!(settings.chat.show_membership_events);
        assert_eq!(settings.theme, ThemeConfig::default());
    }

    #[test]
    fn write_then_read_round_trips_and_leaves_no_temp_file() {
        let dir =
            std::env::temp_dir().join(format!("thornychat-config-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("chat.json");
        let config = ChatConfig { show_membership_events: false, hide_members: true };
        write_json(Some(path.clone()), &config).unwrap();
        let back: ChatConfig = read_json(Some(path)).unwrap();
        assert_eq!(back, config);
        let leftovers: Vec<_> =
            std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(leftovers.len(), 1, "only chat.json should remain, found {leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
