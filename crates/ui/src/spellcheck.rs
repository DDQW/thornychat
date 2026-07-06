//! Spell checking backed by the Windows Spell Checking API
//! (`ISpellChecker`). This is the same engine Edge and the OS text fields
//! use, so it respects the user's Windows display language and their personal
//! dictionary ("Add to dictionary" here sticks system-wide), and ships no
//! bundled word lists of our own.
//!
//! The COM objects are apartment-bound, so the checker lives in a
//! thread-local and is only ever touched from the UI thread — which is where
//! iced runs `update`/`view`, and the only place this module is called. Every
//! function returns plain Rust data; no COM type ever leaves here.
//!
//! All entry points degrade to "nothing is misspelled" if the engine can't be
//! created (COM failure, or a Windows build/language with no speller), so the
//! composer never has to care whether spell checking is actually available.

use std::cell::RefCell;
use std::ffi::c_void;

use windows::core::{HSTRING, PWSTR};
use windows::Win32::Globalization::{
    GetUserDefaultLocaleName, ISpellChecker, ISpellCheckerFactory, SpellCheckerFactory,
    CORRECTIVE_ACTION_GET_SUGGESTIONS, CORRECTIVE_ACTION_REPLACE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};

/// How many alternatives to surface in the suggestion bar. The Windows
/// speller can return a dozen+; more than a handful is just noise on one row.
const MAX_SUGGESTIONS: usize = 5;

/// `LOCALE_NAME_MAX_LENGTH` (chars, incl. the trailing NUL). The `windows`
/// crate doesn't re-export the SDK constant, so it's inlined here.
const LOCALE_NAME_MAX_LENGTH: usize = 85;

/// The speller's verdict for a single word.
#[derive(Debug, Default, Clone)]
pub struct Analysis {
    /// The word is flagged (needs a replacement or suggestions).
    pub misspelled: bool,
    /// A single high-confidence replacement the engine itself recommends
    /// (`CORRECTIVE_ACTION_REPLACE`, e.g. "teh" → "the"). This is the only
    /// thing autocorrect ever applies silently — bare "here are some guesses"
    /// misspellings are surfaced but never auto-changed.
    pub replacement: Option<String>,
    /// Ranked alternatives for the suggestion bar (includes `replacement`
    /// first when there is one).
    pub suggestions: Vec<String>,
}

thread_local! {
    /// `Uninit` until first use, then `Ready` or `Failed` — so a machine
    /// without a usable speller pays one failed init, not one per keystroke.
    static ENGINE: RefCell<Slot> = const { RefCell::new(Slot::Uninit) };
}

enum Slot {
    Uninit,
    Failed,
    Ready(ISpellChecker),
}

/// Runs `f` with the thread-local checker, creating it on first call. Returns
/// `None` if the speller is (or has proven) unavailable.
fn with_checker<R>(f: impl FnOnce(&ISpellChecker) -> R) -> Option<R> {
    ENGINE.with_borrow_mut(|slot| {
        if matches!(slot, Slot::Uninit) {
            *slot = match create_checker() {
                Some(checker) => Slot::Ready(checker),
                None => {
                    tracing::info!("Windows spell checker unavailable; spell check disabled");
                    Slot::Failed
                }
            };
        }
        match slot {
            Slot::Ready(checker) => Some(f(checker)),
            _ => None,
        }
    })
}

/// Creates an `ISpellChecker` for the user's Windows language, falling back to
/// US English if that language has no installed speller.
fn create_checker() -> Option<ISpellChecker> {
    unsafe {
        // Defensive: WebView2/tray already put the UI thread in an STA, so
        // this usually just bumps the init refcount (S_FALSE). A different
        // existing mode returns an error we deliberately ignore — the speller
        // is an in-proc object that works in any apartment.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let factory: ISpellCheckerFactory =
            CoCreateInstance(&SpellCheckerFactory, None, CLSCTX_INPROC_SERVER).ok()?;

        let language = user_language();
        let tag = match factory.IsSupported(&HSTRING::from(&language)) {
            Ok(supported) if supported.as_bool() => language,
            _ => "en-US".to_string(),
        };
        factory.CreateSpellChecker(&HSTRING::from(&tag)).ok()
    }
}

/// The user's default locale name (e.g. `en-US`), or that as a fallback.
fn user_language() -> String {
    let mut buf = [0u16; LOCALE_NAME_MAX_LENGTH];
    let len = unsafe { GetUserDefaultLocaleName(&mut buf) };
    if len > 1 {
        // `len` counts the trailing NUL.
        String::from_utf16_lossy(&buf[..(len as usize - 1)])
    } else {
        "en-US".to_string()
    }
}

/// Checks a single word. Returns a blank (not-misspelled) analysis for the
/// empty string or when no speller is available.
pub fn analyze(word: &str) -> Analysis {
    if word.is_empty() {
        return Analysis::default();
    }
    with_checker(|checker| unsafe {
        let Ok(errors) = checker.Check(&HSTRING::from(word)) else {
            return Analysis::default();
        };
        // We pass one token, so the first error (if any) is the only one that
        // matters. `Next` leaves the out-param `None` when the enumeration is
        // empty — i.e. the word is spelled correctly.
        let mut error = None;
        let _ = errors.Next(&mut error);
        let Some(error) = error else {
            return Analysis::default();
        };
        // The speller word-breaks its input: a token with internal
        // punctuation ("teh.but") can produce an error covering only a
        // sub-token, whose Replacement must NOT be applied to the whole
        // word the composer passed in (autocorrect would silently delete
        // the rest of the token). Only honor an error spanning the entire
        // input; otherwise degrade to "not misspelled" as usual.
        // StartIndex/Length are UTF-16 code units (the input went in as an
        // HSTRING); the unwrap defaults route any COM failure into the
        // guard so it degrades safely.
        let start = error.StartIndex().unwrap_or(u32::MAX);
        let length = error.Length().unwrap_or(0);
        if start != 0 || length as usize != word.encode_utf16().count() {
            return Analysis::default();
        }
        let action = error.CorrectiveAction().unwrap_or_default();

        let replacement = if action == CORRECTIVE_ACTION_REPLACE {
            error.Replacement().ok().and_then(|p| read_and_free(p))
        } else {
            None
        };
        // Only REPLACE and GET_SUGGESTIONS are actionable in our UI; a DELETE
        // action ("remove the repeated word") has no sensible surface here.
        let misspelled =
            replacement.is_some() || action == CORRECTIVE_ACTION_GET_SUGGESTIONS;
        if !misspelled {
            return Analysis::default();
        }

        let mut suggestions = suggestions_for(checker, word);
        // Lead with the engine's own pick so the bar's first button and what
        // autocorrect would have done line up.
        if let Some(rep) = &replacement {
            if !suggestions.iter().any(|s| s == rep) {
                suggestions.insert(0, rep.clone());
                suggestions.truncate(MAX_SUGGESTIONS);
            }
        }
        Analysis { misspelled, replacement, suggestions }
    })
    .unwrap_or_default()
}

/// Adds `word` to the user's Windows dictionary so it stops being flagged
/// (here and in every other app that uses the OS speller).
pub fn add_to_dictionary(word: &str) {
    if word.is_empty() {
        return;
    }
    with_checker(|checker| unsafe {
        if let Err(error) = checker.Add(&HSTRING::from(word)) {
            tracing::warn!(%error, word, "failed to add word to the Windows dictionary");
        }
    });
}

/// Drains `ISpellChecker::Suggest` into an owned, capped list.
unsafe fn suggestions_for(checker: &ISpellChecker, word: &str) -> Vec<String> {
    let Ok(enumerator) = checker.Suggest(&HSTRING::from(word)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    loop {
        let mut buf = [PWSTR::null(); 1];
        let mut fetched = 0u32;
        // Pull one at a time; `fetched == 0` is the enumeration's end (S_FALSE
        // sets it to 0 too), which keeps us off the HRESULT-comparison
        // subtleties entirely.
        let _ = enumerator.Next(&mut buf, Some(&mut fetched));
        if fetched == 0 {
            break;
        }
        if let Some(s) = read_and_free(buf[0]) {
            out.push(s);
        }
        if out.len() >= MAX_SUGGESTIONS {
            break;
        }
    }
    out
}

/// Copies a callee-allocated wide string into an owned `String` and frees it
/// with `CoTaskMemFree`, as the Spell Checking API contract requires.
unsafe fn read_and_free(pwstr: PWSTR) -> Option<String> {
    if pwstr.is_null() {
        return None;
    }
    let value = pwstr.to_string().ok();
    CoTaskMemFree(Some(pwstr.0 as *const c_void));
    value
}
