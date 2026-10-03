//! Spell-check preferences: whether the composer marks misspellings, and
//! whether it fixes obvious typos as you type. The checking itself is the
//! Windows speller ([`crate::spellcheck`]); these are the on/off knobs the
//! page reads. Persisted as `spellcheck.json`, profile-independent.

use std::path::PathBuf;

use client_core::store::AppPaths;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpellcheckConfig {
    /// Mark misspellings in the composer and offer suggestions for them.
    /// Non-destructive: it never rewrites your text by itself.
    pub enabled: bool,
    /// Apply the speller's high-confidence fix when a word is finished with a
    /// space (Backspace immediately after reverts it). Ships **off** — typing
    /// stays predictable until opted in.
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
