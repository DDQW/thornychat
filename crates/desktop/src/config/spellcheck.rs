//! Spell-check preferences. Checking itself is the webview's own (WebView2
//! marks typos in the composer with the Windows dictionaries); these are the
//! on/off knobs. Persisted as `spellcheck.json`, profile-independent.

use std::path::PathBuf;

use client_core::store::AppPaths;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpellcheckConfig {
    /// Mark misspellings in the composer. Non-destructive: it only ever
    /// underlines, it never rewrites your text.
    pub enabled: bool,
    /// Silently apply the speller's high-confidence fix when a word is
    /// finished. Ships **off** — typing stays predictable until opted in.
    pub autocorrect: bool,
}

impl Default for SpellcheckConfig {
    fn default() -> Self {
        // Spell *checking* on (it's unobtrusive), silent auto-rewriting off
        // (it changes text you didn't ask it to).
        Self { enabled: true, autocorrect: false }
    }
}

impl SpellcheckConfig {
    pub fn config_path() -> Option<PathBuf> {
        AppPaths::global_config_dir().ok().map(|dir| dir.join("spellcheck.json"))
    }

    pub fn load_or_default() -> Self {
        super::read_json(Self::config_path()).unwrap_or_default()
    }
}
