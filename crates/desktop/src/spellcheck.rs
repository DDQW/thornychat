//! Spell checking backed by the Windows Spell Checking API (`ISpellChecker`):
//! the engine Edge and the OS text fields use, so it follows the user's
//! Windows language and personal dictionary ("Add to dictionary" here sticks
//! system-wide), and ships no word lists of our own.
//!
//! WebView2's own checker is no substitute: in this app it marks nothing at
//! all (checked on the built exe, 2026-10), and it could neither leave chat
//! slang alone nor drive autocorrect. The page does the tokenising, marking
//! and autocorrect bookkeeping (`frontend/src/lib/spell.ts`); this module
//! answers its per-word questions.
//!
//! The COM objects are apartment-bound, so they live on one dedicated STA
//! thread and every request is shipped to it over a channel — never touched
//! from Tauri's async command threads. No COM type leaves that thread.
//!
//! Everything degrades to "nothing is misspelled" when the engine can't be
//! created (COM failure, or a Windows build/language with no speller), so the
//! page never has to care whether spell checking is actually available.

use std::ffi::c_void;
use std::sync::{mpsc, OnceLock};
use std::time::Duration;

use serde::Serialize;
use windows::core::{HSTRING, PWSTR};
use windows::Win32::Globalization::{
    GetUserDefaultLocaleName, ISpellChecker, ISpellCheckerFactory, SpellCheckerFactory,
    CORRECTIVE_ACTION_GET_SUGGESTIONS, CORRECTIVE_ACTION_REPLACE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};

#[cfg(test)]
mod corpus;

/// How many alternatives to offer. The Windows speller can return a dozen+;
/// more than a handful is just noise in one row.
const MAX_SUGGESTIONS: usize = 5;

/// `LOCALE_NAME_MAX_LENGTH` (chars, incl. the trailing NUL). The `windows`
/// crate doesn't re-export the SDK constant, so it's inlined here.
const LOCALE_NAME_MAX_LENGTH: usize = 85;

/// Upper bounds on one request from the page. A chat draft is nowhere near
/// either; they only stop a runaway caller from parking the speller thread.
const MAX_WORDS_PER_CHECK: usize = 2_000;
const MAX_WORD_CHARS: usize = 64;

/// How long a command waits for the speller thread before answering "not
/// misspelled". A healthy call takes well under a millisecond.
const REPLY_TIMEOUT: Duration = Duration::from_secs(3);

/// The speller's verdict for a single word.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Analysis {
    /// The word is flagged (needs a replacement or suggestions).
    pub misspelled: bool,
    /// A single high-confidence replacement the engine itself recommends
    /// (`CORRECTIVE_ACTION_REPLACE`, e.g. "teh" → "the"). Autocorrect trusts
    /// this outright; a bare "here are some guesses" misspelling only gets
    /// rewritten when the guesses point at one unambiguous answer (see
    /// [`pick_correction`]).
    pub replacement: Option<String>,
    /// Ranked alternatives for the suggestion bar (`replacement` first when
    /// there is one).
    pub suggestions: Vec<String>,
}

/// What the Settings screen shows about the engine.
#[derive(Debug, Clone, Serialize)]
pub struct SpellStatus {
    pub available: bool,
    /// The BCP-47 tag the checker was created for (e.g. `en-US`).
    pub language: Option<String>,
}

/// A live `ISpellChecker`, only ever created and used on the speller thread.
struct Engine {
    checker: ISpellChecker,
    language: String,
}

impl Engine {
    /// Creates a checker for the user's Windows language, falling back to US
    /// English if that language has no installed speller.
    fn create() -> Option<Self> {
        unsafe {
            let factory: ISpellCheckerFactory =
                CoCreateInstance(&SpellCheckerFactory, None, CLSCTX_INPROC_SERVER).ok()?;
            let preferred = user_language();
            let language = match factory.IsSupported(&HSTRING::from(&preferred)) {
                Ok(supported) if supported.as_bool() => preferred,
                _ => "en-US".to_string(),
            };
            let checker = factory.CreateSpellChecker(&HSTRING::from(&language)).ok()?;
            Some(Self { checker, language })
        }
    }

    /// Checks a single word. Blank (not misspelled) for the empty string.
    fn analyze(&self, word: &str) -> Analysis {
        if word.is_empty() {
            return Analysis::default();
        }
        unsafe {
            let Some(Verdict { replacement }) = whole_word_error(&self.checker, word) else {
                return Analysis::default();
            };
            let mut suggestions = suggestions_for(&self.checker, word);
            // Lead with the engine's own pick so the first suggestion and what
            // autocorrect would have done line up.
            if let Some(rep) = &replacement {
                if !suggestions.iter().any(|s| s == rep) {
                    suggestions.insert(0, rep.clone());
                    suggestions.truncate(MAX_SUGGESTIONS);
                }
            }
            Analysis { misspelled: true, replacement, suggestions }
        }
    }

    /// Whether the speller flags `word`, without asking it for alternatives.
    ///
    /// `ISpellChecker::Suggest` is by far the expensive call here, and marking
    /// up the draft needs one boolean per word — never a suggestion list.
    fn is_misspelled(&self, word: &str) -> bool {
        !word.is_empty() && unsafe { whole_word_error(&self.checker, word).is_some() }
    }

    /// The word autocorrect should substitute for `word`, if any.
    ///
    /// The engine's own `CORRECTIVE_ACTION_REPLACE` pick covers most
    /// transposition typos but not doubled/dropped letters ("adress", "untill",
    /// "concious"), which come back as bare guesses — so this falls back to the
    /// guesses when one of them is a clear near miss of what was typed (see
    /// [`pick_correction`]). That fallback is the difference between autocorrect
    /// firing on those and it effectively never firing at all.
    fn top_correction(&self, word: &str) -> Option<String> {
        let analysis = self.analyze(word);
        if !analysis.misspelled {
            return None;
        }
        pick_correction(word, analysis.replacement.as_deref(), &analysis.suggestions)
    }

    /// Adds `word` to the user's Windows dictionary so it stops being flagged
    /// (here and in every other app that uses the OS speller).
    fn add(&self, word: &str) {
        if word.is_empty() {
            return;
        }
        if let Err(error) = unsafe { self.checker.Add(&HSTRING::from(word)) } {
            tracing::warn!(%error, word, "failed to add word to the Windows dictionary");
        }
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
/// `replacement` carries the engine's own high-confidence pick when it has one.
struct Verdict {
    replacement: Option<String>,
}

/// Runs `ISpellChecker::Check` and returns the error covering the *entire*
/// input, if there is one.
///
/// The speller word-breaks its input: a token with internal punctuation
/// ("teh.but") can produce an error covering only a sub-token, whose
/// Replacement must NOT be applied to the whole word the page passed in
/// (autocorrect would silently delete the rest of the token). Only an error
/// spanning the whole input is honoured; anything else degrades to "not
/// misspelled", as does any COM failure along the way.
unsafe fn whole_word_error(checker: &ISpellChecker, word: &str) -> Option<Verdict> {
    let errors = checker.Check(&HSTRING::from(word)).ok()?;
    // One token in, so the first error (if any) is the only one that matters.
    // `Next` leaves the out-param `None` when the enumeration is empty — i.e.
    // the word is spelled correctly.
    let mut error = None;
    let _ = errors.Next(&mut error);
    let error = error?;
    // StartIndex/Length are UTF-16 code units (the input went in as an
    // HSTRING); the defaults route any COM failure into the guard.
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
    // Only REPLACE and GET_SUGGESTIONS are actionable here; a DELETE action
    // ("remove the repeated word") has no sensible surface in a composer.
    if replacement.is_none() && action != CORRECTIVE_ACTION_GET_SUGGESTIONS {
        return None;
    }
    Some(Verdict { replacement })
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
        // One at a time; `fetched == 0` is the enumeration's end (S_FALSE sets
        // it to 0 too), which keeps us off the HRESULT-comparison subtleties.
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

// --- the speller thread ---

/// Work shipped to the speller thread. It gets the engine, or `None` when
/// there is no usable speller on this machine.
type Job = Box<dyn FnOnce(Option<&Engine>) + Send>;

/// The speller thread's inbox, started on first use. `None` if the thread
/// couldn't even be spawned.
fn inbox() -> Option<&'static mpsc::Sender<Job>> {
    static INBOX: OnceLock<Option<mpsc::Sender<Job>>> = OnceLock::new();
    INBOX
        .get_or_init(|| {
            let (sender, jobs) = mpsc::channel::<Job>();
            match std::thread::Builder::new().name("spellcheck".into()).spawn(move || serve(jobs)) {
                Ok(_) => Some(sender),
                Err(error) => {
                    tracing::warn!(%error, "could not start the spell-check thread");
                    None
                }
            }
        })
        .as_ref()
}

/// The speller thread: one STA, one engine, jobs in arrival order. It runs
/// for the life of the process (the inbox is never dropped).
///
/// No message pump: nothing is ever marshalled into this apartment — every
/// call on the engine is made from here — so there are no incoming calls a
/// pump would need to deliver.
fn serve(jobs: mpsc::Receiver<Job>) {
    // A fresh thread, so this is the first (and only) initialisation; a
    // failure leaves `create` to fail too, which is the degraded path.
    if let Err(error) = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok() {
        tracing::warn!(%error, "could not initialise COM on the spell-check thread");
    }
    let engine = Engine::create();
    match &engine {
        Some(engine) => tracing::info!(language = %engine.language, "Windows spell checker ready"),
        None => tracing::info!("Windows spell checker unavailable; spell check disabled"),
    }
    for job in jobs {
        job(engine.as_ref());
    }
}

/// Runs `job` on the speller thread and waits (asynchronously) for its answer.
/// `None` if the thread is gone, the job panicked, or it took too long.
async fn on_speller<R: Send + 'static>(job: impl FnOnce(Option<&Engine>) -> R + Send + 'static) -> Option<R> {
    let (reply, answer) = tokio::sync::oneshot::channel();
    inbox()?
        .send(Box::new(move |engine| {
            let _ = reply.send(job(engine));
        }))
        .ok()?;
    match tokio::time::timeout(REPLY_TIMEOUT, answer).await {
        Ok(Ok(value)) => Some(value),
        Ok(Err(_)) => None,
        Err(_) => {
            tracing::warn!("the spell checker did not answer in time");
            None
        }
    }
}

/// [`on_speller`] for tests, which have no async runtime.
#[cfg(test)]
fn on_speller_blocking<R: Send + 'static>(job: impl FnOnce(Option<&Engine>) -> R + Send + 'static) -> Option<R> {
    let (reply, answer) = mpsc::channel();
    inbox()?
        .send(Box::new(move |engine| {
            let _ = reply.send(job(engine));
        }))
        .ok()?;
    answer.recv_timeout(REPLY_TIMEOUT).ok()
}

/// A word worth handing to the speller: bounded, so a pasted wall of
/// characters can't occupy it.
fn checkable_length(word: &str) -> bool {
    !word.is_empty() && word.chars().count() <= MAX_WORD_CHARS
}

// --- commands ---

/// Whether the engine is usable, and in which language. Also warms it up, so
/// the first keystroke doesn't pay for creating it.
#[tauri::command]
pub async fn spell_status() -> SpellStatus {
    on_speller(|engine| SpellStatus {
        available: engine.is_some(),
        language: engine.map(|engine| engine.language.clone()),
    })
    .await
    .unwrap_or(SpellStatus { available: false, language: None })
}

/// One verdict per word, in order: `true` for misspelled.
#[tauri::command]
pub async fn spell_check(words: Vec<String>) -> Vec<bool> {
    let count = words.len();
    if count > MAX_WORDS_PER_CHECK {
        return vec![false; count];
    }
    on_speller(move |engine| {
        let Some(engine) = engine else { return vec![false; words.len()] };
        words.iter().map(|word| checkable_length(word) && engine.is_misspelled(word)).collect()
    })
    .await
    .unwrap_or_else(|| vec![false; count])
}

/// Ranked alternatives for a misspelled word (empty when it isn't one).
#[tauri::command]
pub async fn spell_suggest(word: String) -> Vec<String> {
    if !checkable_length(&word) {
        return Vec::new();
    }
    on_speller(move |engine| engine.map(|engine| engine.analyze(&word).suggestions).unwrap_or_default())
        .await
        .unwrap_or_default()
}

/// The fix autocorrect should apply to `word`, if there is a confident one.
#[tauri::command]
pub async fn spell_correction(word: String) -> Option<String> {
    if !checkable_length(&word) {
        return None;
    }
    on_speller(move |engine| engine.and_then(|engine| engine.top_correction(&word))).await.flatten()
}

/// Adds `word` to the user's Windows dictionary.
#[tauri::command]
pub async fn spell_add(word: String) {
    if !checkable_length(&word) {
        return;
    }
    on_speller(move |engine| {
        if let Some(engine) = engine {
            engine.add(&word);
        }
    })
    .await;
}

// --- choosing a correction ---

/// Chooses between the engine's high-confidence replacement and its ranked
/// guesses. Split out from [`Engine::top_correction`] so the guard is testable
/// without a live speller.
///
/// A `CORRECTIVE_ACTION_REPLACE` pick is trusted as-is — the engine gets those
/// right ("teh" to "the", "yuo" to "you", "recieve" to "receive").
///
/// A mere guess has to be within a couple of edits of what was typed: the
/// speller's first idea about a word it doesn't recognise at all can be
/// arbitrarily far away, and silently swapping *that* in is the behaviour that
/// makes autocorrect hated. Distance decides the shortlist; among the
/// candidates that tie for closest, the engine's own ranking decides, because
/// for real typos that ranking is good ("disappointet" ranks "disappointed"
/// ahead of the equally-close "disappointer").
///
/// Where the ranking is *not* good is chat slang the dictionary simply lacks —
/// asked about "im" the engine leads with "mi". That is handled upstream by
/// not checking those words at all (`isCheckable` in the page's `spell.ts`),
/// rather than by second-guessing the ranking here.
fn pick_correction(word: &str, replacement: Option<&str>, suggestions: &[String]) -> Option<String> {
    if let Some(replacement) = replacement {
        return (replacement != word).then(|| replacement.to_string());
    }
    // Short words sit one edit away from plenty of unrelated words, so they
    // get a tighter budget than long ones.
    let typed: Vec<char> = word.to_lowercase().chars().collect();
    let max = if typed.len() <= 4 { 1 } else { 2 };
    suggestions
        .iter()
        .filter_map(|suggestion| {
            let candidate: Vec<char> = suggestion.to_lowercase().chars().collect();
            // A candidate differing only in case isn't a typo fix worth making
            // silently — and it shouldn't crowd out one that is.
            if candidate == typed {
                return None;
            }
            let distance = edit_distance_at_most(&typed, &candidate, max)?;
            Some((distance, suggestion))
        })
        // `min_by_key` keeps the first of an equal-minimum run, and the
        // suggestions arrive ranked — so this is "closest, ties to the engine's
        // preference".
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, suggestion)| suggestion.clone())
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
            let mut best = (previous[j] + cost).min(previous[j + 1] + 1).min(current[j] + 1);
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
        assert_eq!(pick_correction("teh", Some("the"), &[]), Some("the".to_string()));
        // ...but a "fix" identical to what was typed is not a fix.
        assert_eq!(pick_correction("teh", Some("teh"), &[]), None);
    }

    #[test]
    fn suggestions_apply_only_when_they_are_near_misses() {
        // An out-of-budget guess is ignored outright — it neither wins nor
        // ties, so it can't block the real fix behind it.
        let suggestions = vec!["nonsense".to_string(), "receive".to_string()];
        assert_eq!(pick_correction("recieve", None, &suggestions), Some("receive".to_string()));

        // The speller's best guess at an unknown word can be miles away; that
        // one is left for the suggestion bar rather than applied silently.
        let far = vec!["quarterly".to_string()];
        assert_eq!(pick_correction("qwertyx", None, &far), None);

        // Short words get the tighter 1-edit budget: "cst" → "cost" (1) is in,
        // "cst" → "chest" (2) is out.
        assert_eq!(pick_correction("cst", None, &["cost".to_string()]), Some("cost".to_string()));
        assert_eq!(pick_correction("cst", None, &["chest".to_string()]), None);
    }

    #[test]
    fn case_only_differences_are_not_corrections() {
        assert_eq!(pick_correction("teh", None, &["Teh".to_string()]), None);
    }

    #[test]
    fn the_ranking_breaks_a_tie_between_equally_close_guesses() {
        // The case this exists for: both are one edit from "disappointet",
        // and the engine ranks the overwhelmingly likelier one first.
        let suggestions = vec!["disappointed".to_string(), "disappointer".to_string()];
        assert_eq!(pick_correction("disappointet", None, &suggestions), Some("disappointed".to_string()));

        // Same shape, shorter word: "the" and "ten" are both one edit from
        // "teh", and "the" is what the engine offers first.
        let teh = vec!["the".to_string(), "ten".to_string()];
        assert_eq!(pick_correction("teh", None, &teh), Some("the".to_string()));
    }

    #[test]
    fn a_closer_guess_still_wins_over_the_ranking() {
        // Ranked first but two edits out; the one-edit guess behind it is the
        // unambiguous answer, so the ranking doesn't get to veto it.
        let suggestions = vec!["privileged".to_string(), "privilege".to_string()];
        assert_eq!(pick_correction("priviledge", None, &suggestions), Some("privilege".to_string()));
    }

    #[test]
    fn oversized_input_never_reaches_the_speller() {
        assert!(!checkable_length(""));
        assert!(checkable_length("receive"));
        assert!(!checkable_length(&"a".repeat(MAX_WORD_CHARS + 1)));
        // Counted in chars: 64 two-byte letters are still one word.
        assert!(checkable_length(&"é".repeat(MAX_WORD_CHARS)));
    }

    /// The live engine, through the same thread the commands use. Skipped
    /// where there is no speller (see the module docs).
    #[test]
    fn the_speller_thread_answers_and_flags_a_typo() {
        let Some(Some(verdicts)) = on_speller_blocking(|engine| {
            engine.map(|engine| (engine.is_misspelled("teh"), engine.is_misspelled("the")))
        }) else {
            println!("no speller available — skipping");
            return;
        };
        assert_eq!(verdicts, (true, false));
    }

    #[test]
    fn concurrent_callers_are_served_one_at_a_time_on_one_thread() {
        // Every job runs on the same named thread, whichever thread sent it:
        // the apartment rule the module exists to respect.
        let names: Vec<String> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        on_speller_blocking(|_| std::thread::current().name().map(str::to_string))
                            .flatten()
                            .unwrap_or_default()
                    })
                })
                .collect();
            handles.into_iter().map(|handle| handle.join().unwrap()).collect()
        });
        assert!(names.iter().all(|name| name == "spellcheck"), "{names:?}");
    }
}
