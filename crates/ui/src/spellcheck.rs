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
    /// (`CORRECTIVE_ACTION_REPLACE`, e.g. "teh" → "the"). Autocorrect trusts
    /// this outright; a bare "here are some guesses" misspelling only gets
    /// rewritten when the guesses point at one unambiguous answer (see
    /// [`pick_correction`]).
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

/// The speller's verdict for a token it flagged. `Some` means "misspelled";
/// `replacement` carries the engine's own high-confidence pick when it has
/// one.
struct Verdict {
    replacement: Option<String>,
}

/// Runs `ISpellChecker::Check` and returns the error covering the *entire*
/// input, if there is one.
///
/// The speller word-breaks its input: a token with internal punctuation
/// ("teh.but") can produce an error covering only a sub-token, whose
/// Replacement must NOT be applied to the whole word the composer passed in
/// (autocorrect would silently delete the rest of the token). Only an error
/// spanning the whole input is honored; anything else degrades to "not
/// misspelled", as does any COM failure along the way.
unsafe fn whole_word_error(checker: &ISpellChecker, word: &str) -> Option<Verdict> {
    let errors = checker.Check(&HSTRING::from(word)).ok()?;
    // We pass one token, so the first error (if any) is the only one that
    // matters. `Next` leaves the out-param `None` when the enumeration is
    // empty — i.e. the word is spelled correctly.
    let mut error = None;
    let _ = errors.Next(&mut error);
    let error = error?;
    // StartIndex/Length are UTF-16 code units (the input went in as an
    // HSTRING); the unwrap defaults route any COM failure into the guard so
    // it degrades safely.
    let start = error.StartIndex().unwrap_or(u32::MAX);
    let length = error.Length().unwrap_or(0);
    if start != 0 || length as usize != word.encode_utf16().count() {
        return None;
    }
    let action = error.CorrectiveAction().unwrap_or_default();
    let replacement = if action == CORRECTIVE_ACTION_REPLACE {
        error.Replacement().ok().and_then(|p| read_and_free(p))
    } else {
        None
    };
    // Only REPLACE and GET_SUGGESTIONS are actionable in our UI; a DELETE
    // action ("remove the repeated word") has no sensible surface here.
    if replacement.is_none() && action != CORRECTIVE_ACTION_GET_SUGGESTIONS {
        return None;
    }
    Some(Verdict { replacement })
}

/// Checks a single word. Returns a blank (not-misspelled) analysis for the
/// empty string or when no speller is available.
pub fn analyze(word: &str) -> Analysis {
    if word.is_empty() {
        return Analysis::default();
    }
    with_checker(|checker| unsafe {
        let Some(Verdict { replacement }) = whole_word_error(checker, word) else {
            return Analysis::default();
        };
        let mut suggestions = suggestions_for(checker, word);
        // Lead with the engine's own pick so the bar's first button and what
        // autocorrect would have done line up.
        if let Some(rep) = &replacement {
            if !suggestions.iter().any(|s| s == rep) {
                suggestions.insert(0, rep.clone());
                suggestions.truncate(MAX_SUGGESTIONS);
            }
        }
        Analysis { misspelled: true, replacement, suggestions }
    })
    .unwrap_or_default()
}

/// Whether the speller flags `word`, without asking it for alternatives.
///
/// `ISpellChecker::Suggest` is by far the expensive call here, and marking up
/// the draft needs one boolean per word — never a suggestion list. The bar and
/// autocorrect still go through [`analyze`], which does both.
pub fn is_misspelled(word: &str) -> bool {
    if word.is_empty() {
        return false;
    }
    with_checker(|checker| unsafe { whole_word_error(checker, word).is_some() })
        .unwrap_or(false)
}

/// The word autocorrect should substitute for `word`, if any.
///
/// The engine's own `CORRECTIVE_ACTION_REPLACE` pick covers most transposition
/// typos but not doubled/dropped letters ("adress", "untill", "concious"),
/// which come back as bare guesses — so this falls back to the guesses when
/// exactly one of them is a clear near miss of what was typed (see
/// [`pick_correction`]). That fallback is the difference between autocorrect
/// firing on those and it effectively never firing at all.
pub fn top_correction(word: &str) -> Option<String> {
    let analysis = analyze(word);
    if !analysis.misspelled {
        return None;
    }
    pick_correction(word, analysis.replacement.as_deref(), &analysis.suggestions)
}

/// Chooses between the engine's high-confidence replacement and its ranked
/// guesses. Split out from [`top_correction`] so the guard is testable
/// without a live speller.
///
/// A `CORRECTIVE_ACTION_REPLACE` pick is trusted as-is — the engine gets those
/// right ("teh" → "the", "yuo" → "you", "recieve" → "receive").
///
/// A mere suggestion has to earn it twice over. It must be within a couple of
/// edits of what was typed — the speller's first guess at a word it doesn't
/// recognise at all can be arbitrarily far away — *and* it must be strictly
/// closer than every other guess in the list. The engine's ranking is not
/// evidence: asked about "im" it offers "mi", "imp", "I'm", "am", "in", all
/// one edit away, and leads with the worst of them. When several candidates
/// tie, there is no obvious answer to apply silently, so the word stays
/// flagged and the suggestion bar does the offering instead.
fn pick_correction(
    word: &str,
    replacement: Option<&str>,
    suggestions: &[String],
) -> Option<String> {
    if let Some(replacement) = replacement {
        return (replacement != word).then(|| replacement.to_string());
    }
    // Short words sit one edit away from plenty of unrelated words, so they
    // get a tighter budget than long ones.
    let typed: Vec<char> = word.to_lowercase().chars().collect();
    let max = if typed.len() <= 4 { 1 } else { 2 };
    let mut best: Option<(usize, &String)> = None;
    let mut tied = false;
    for suggestion in suggestions {
        let candidate: Vec<char> = suggestion.to_lowercase().chars().collect();
        // A candidate differing only in case isn't a typo fix worth making
        // silently, and it shouldn't block one either.
        if candidate == typed {
            continue;
        }
        let Some(distance) = edit_distance_at_most(&typed, &candidate, max) else {
            continue;
        };
        match best {
            // A new closest guess clears any tie the old one was in.
            None => best = Some((distance, suggestion)),
            Some((closest, _)) if distance < closest => {
                best = Some((distance, suggestion));
                tied = false;
            }
            Some((closest, _)) if distance == closest => tied = true,
            Some(_) => {}
        }
    }
    match best {
        Some((_, suggestion)) if !tied => Some(suggestion.clone()),
        _ => None,
    }
}

/// Edit distance between two char slices, or `None` as soon as it's certain to
/// exceed `max`. Bounded so scanning a suggestion list stays cheap: the row
/// minimum only ever grows, so a row already entirely over budget can't come
/// back under it.
///
/// Swapping two adjacent characters counts as **one** edit, not two
/// (Damerau-Levenshtein, restricted to adjacent transpositions). That is the
/// difference between "teh" → "the" being in budget and out of it — and a
/// transposition is the single most common way to mistype a word, so charging
/// it double would rule out exactly the corrections worth making.
fn edit_distance_at_most(a: &[char], b: &[char], max: usize) -> Option<usize> {
    if a.len().abs_diff(b.len()) > max {
        return None;
    }
    // Three rows: the transposition case reaches back two rows, not one.
    let mut two_back = vec![0usize; b.len() + 1];
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for (i, ac) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, bc) in b.iter().enumerate() {
            let cost = usize::from(ac != bc);
            let mut best =
                (previous[j] + cost).min(previous[j + 1] + 1).min(current[j] + 1);
            if i > 0 && j > 0 && *ac == b[j - 1] && a[i - 1] == *bc {
                best = best.min(two_back[j - 1] + 1);
            }
            current[j + 1] = best;
        }
        if current.iter().min().copied().unwrap_or(usize::MAX) > max {
            return None;
        }
        // Rotate: `previous` becomes the two-rows-back row, `current` the
        // previous one, and the stale row is reused as the next scratch.
        std::mem::swap(&mut two_back, &mut previous);
        std::mem::swap(&mut previous, &mut current);
    }
    let distance = previous[b.len()];
    (distance <= max).then_some(distance)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn bounded_distance_gives_up_past_the_budget() {
        assert_eq!(edit_distance_at_most(&chars("cat"), &chars("cat"), 1), Some(0));
        assert_eq!(edit_distance_at_most(&chars("cst"), &chars("cost"), 1), Some(1));
        // Over budget, and cheaply so — the length gap alone rules it out.
        assert_eq!(edit_distance_at_most(&chars("qwertyx"), &chars("q"), 2), None);
        assert_eq!(edit_distance_at_most(&chars("teh"), &chars("bun"), 1), None);
    }

    #[test]
    fn a_transposition_costs_one_edit_not_two() {
        // The whole reason for the Damerau term: at plain Levenshtein's 2,
        // "teh" (3 chars, so a budget of 1) could never be corrected to "the".
        assert_eq!(edit_distance_at_most(&chars("teh"), &chars("the"), 1), Some(1));
        assert_eq!(edit_distance_at_most(&chars("recieve"), &chars("receive"), 1), Some(1));
        // Non-adjacent swaps are still two edits, as they should be.
        assert_eq!(edit_distance_at_most(&chars("tehs"), &chars("thes"), 1), Some(1));
        assert_eq!(edit_distance_at_most(&chars("abc"), &chars("cba"), 1), None);
    }

    #[test]
    fn distance_counts_chars_not_bytes() {
        // 'é' is two bytes; swapping it for 'e' is one edit, not two.
        assert_eq!(edit_distance_at_most(&chars("café"), &chars("cafe"), 1), Some(1));
    }

    #[test]
    fn engine_replacement_is_trusted_as_is() {
        // A REPLACE pick skips the distance guard entirely — the engine is
        // more confident than we are.
        assert_eq!(
            pick_correction("teh", Some("the"), &[]),
            Some("the".to_string())
        );
        // ...but a "fix" identical to what was typed is not a fix.
        assert_eq!(pick_correction("teh", Some("teh"), &[]), None);
    }

    #[test]
    fn suggestions_apply_only_when_they_are_near_misses() {
        // An out-of-budget guess is ignored outright — it neither wins nor
        // ties, so it can't block the real fix behind it.
        let suggestions = vec!["nonsense".to_string(), "receive".to_string()];
        assert_eq!(
            pick_correction("recieve", None, &suggestions),
            Some("receive".to_string())
        );

        // The speller's best guess at an unknown word can be miles away; that
        // one is left for the bar to offer rather than applied silently.
        let far = vec!["quarterly".to_string()];
        assert_eq!(pick_correction("qwertyx", None, &far), None);

        // Short words get the tighter 1-edit budget: "cst" → "cost" (1) is in,
        // "cst" → "chest" (2) is out.
        assert_eq!(
            pick_correction("cst", None, &["cost".to_string()]),
            Some("cost".to_string())
        );
        assert_eq!(pick_correction("cst", None, &["chest".to_string()]), None);
    }

    #[test]
    fn case_only_differences_are_not_corrections() {
        assert_eq!(pick_correction("teh", None, &["Teh".to_string()]), None);
    }

    #[test]
    fn a_tie_between_near_misses_is_left_to_the_bar() {
        // "the" and "ten" are both one edit from "teh". Ranking alone isn't
        // enough to pick one silently — for "im" the live speller ranks the
        // nonsense "mi" ahead of "I'm" and "in", all at one edit, which is
        // exactly the swap this rule exists to refuse.
        let suggestions = vec!["the".to_string(), "ten".to_string()];
        assert_eq!(pick_correction("teh", None, &suggestions), None);
        let im = vec!["mi".to_string(), "imp".to_string(), "in".to_string()];
        assert_eq!(pick_correction("im", None, &im), None);
    }

    #[test]
    fn a_strictly_closer_guess_wins_over_the_ranking() {
        // Ranked first but two edits out; the one-edit guess behind it is the
        // unambiguous answer, so the ranking doesn't get to veto it.
        let suggestions = vec!["privileged".to_string(), "privilege".to_string()];
        assert_eq!(
            pick_correction("priviledge", None, &suggestions),
            Some("privilege".to_string())
        );
    }
}
