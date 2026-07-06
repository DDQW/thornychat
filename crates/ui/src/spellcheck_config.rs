//! Spell-check preferences: whether the composer flags misspellings and
//! whether it silently fixes obvious typos as you type. Persisted as a small
//! JSON file next to the theme and privacy config
//! (`%APPDATA%\ThornyChat\ThornyChat\config\spellcheck.json`), profile-independent
//! like the rest of the client's global preferences.
//!
//! The actual checking is done by the Windows speller — see
//! [`crate::spellcheck`]. These are just the on/off knobs the composer reads.

use std::path::PathBuf;

use client_core::store::AppPaths;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpellcheckConfig {
    /// Show the suggestion bar above the composer for a misspelled word.
    /// Non-destructive: it only ever appears, it never rewrites your text.
    pub enabled: bool,
    /// Silently apply the speller's high-confidence fix when you finish a
    /// word with a space (Backspace immediately after reverts it). Ships
    /// **off** — typing stays predictable until the user opts in.
    pub autocorrect: bool,
}

impl Default for SpellcheckConfig {
    fn default() -> Self {
        // Spell *checking* on (it's unobtrusive — a bar you can ignore),
        // silent auto-rewriting off (it changes text you didn't ask it to).
        Self { enabled: true, autocorrect: false }
    }
}

impl SpellcheckConfig {
    /// `%APPDATA%\ThornyChat\ThornyChat\config\spellcheck.json` — global, like the
    /// theme and privacy config, since it's a property of this install rather
    /// than any single account.
    pub fn config_path() -> Option<PathBuf> {
        AppPaths::global_config_dir().ok().map(|dir| dir.join("spellcheck.json"))
    }

    pub fn load() -> Option<Self> {
        let path = Self::config_path()?;
        let contents = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&contents).ok()
    }

    /// A missing or unreadable file falls back to the defaults (checking on,
    /// autocorrect off).
    pub fn load_or_default() -> Self {
        Self::load().unwrap_or_default()
    }

    pub fn to_json_pretty(&self) -> Option<String> {
        serde_json::to_string_pretty(self).ok()
    }
}
