//! Marking misspelled words *inside* the composer.
//!
//! iced's `text_editor` can recolour byte ranges per line through a
//! [`Highlighter`], which is how a typo gets flagged where you made it rather
//! than in a bar above the box. Its API offers colour and font — not a wavy
//! underline — so a flagged word is drawn in the theme's danger colour.
//!
//! Nothing here talks to the speller. `update` decides which words are
//! misspelled (that's where COM is allowed to live — see [`crate::spellcheck`])
//! and hands the verdicts down as [`Settings`]; this module is pure data, and
//! is re-run by the renderer on every relayout.
//!
//! Words are matched by *text*, not by range, so the same typo repeated is
//! flagged in every position and there's no line-index bookkeeping to drift
//! out of sync with the buffer.

use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;

use iced::advanced::text::highlighter::Format;

/// One whitespace-delimited token of a line, plus the alphanumeric core the
/// speller actually sees.
///
/// The distinction matters in both directions: skip decisions
/// ([`is_checkable`]) need the full token to recognise a URL or a mention,
/// while a replacement must land on the core alone so `"helo,"` keeps its
/// comma.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word<'a> {
    /// Byte range of `core` within the line it came from.
    pub range: Range<usize>,
    /// Byte range of `raw` within that same line.
    pub raw_range: Range<usize>,
    /// The token trimmed to its outermost alphanumeric characters.
    pub core: &'a str,
    /// The raw whitespace-delimited token.
    pub raw: &'a str,
}

impl<'a> Word<'a> {
    /// Trims `raw` to its alphanumeric core. `None` for a token with no
    /// alphanumeric character at all (bare punctuation, an emoji).
    fn new(line: &'a str, raw_range: Range<usize>) -> Option<Self> {
        let raw = &line[raw_range.clone()];
        let core_start =
            raw.char_indices().find(|(_, c)| c.is_alphanumeric()).map(|(i, _)| i)?;
        let core_end = raw
            .char_indices()
            .rev()
            .find(|(_, c)| c.is_alphanumeric())
            .map(|(i, c)| i + c.len_utf8())?;
        let range = (raw_range.start + core_start)..(raw_range.start + core_end);
        Some(Word { core: &line[range.clone()], range, raw, raw_range })
    }
}

/// Every word in `line`, in order, with byte ranges into `line`.
///
/// Splits on whitespace only — the speller does its own word breaking, and
/// splitting on punctuation here would tear "don't" and "e.g." apart before it
/// ever saw them. Tokens with no alphanumeric core are skipped.
pub fn words(line: &str) -> impl Iterator<Item = Word<'_>> {
    let mut cursor = 0usize;
    std::iter::from_fn(move || loop {
        let rest = line.get(cursor..)?;
        let lead = rest.find(|c: char| !c.is_whitespace())?;
        let start = cursor + lead;
        let end = line[start..]
            .find(char::is_whitespace)
            .map(|i| start + i)
            .unwrap_or(line.len());
        cursor = end;
        if let Some(word) = Word::new(line, start..end) {
            return Some(word);
        }
        // Punctuation-only token — keep scanning rather than stopping.
    })
}

/// Whether a raw token is ordinary prose worth spell-checking — filters out
/// the things chat is full of that a dictionary would wrongly flag: mentions,
/// emoji shortcodes, URLs/paths, code-ish identifiers, acronyms, and anything
/// carrying a digit.
pub fn is_checkable(raw: &str) -> bool {
    // Needs at least two letters to be a word worth checking.
    if raw.chars().filter(|c| c.is_alphabetic()).count() < 2 {
        return false;
    }
    // Mentions and emoji shortcodes.
    if raw.starts_with('@') || raw.starts_with(':') || raw.contains('@') {
        return false;
    }
    // URLs / paths / snake_case identifiers.
    if raw.contains("://")
        || raw.contains('/')
        || raw.contains('\\')
        || raw.contains('_')
        || raw.starts_with("www.")
    {
        return false;
    }
    // Versions, IDs, l33t — anything with a digit.
    if raw.chars().any(|c| c.is_numeric()) {
        return false;
    }
    // ALL-CAPS acronyms (GG, LOL) and MixedCase code identifiers (camelCase,
    // PascalCase): flag neither. A plain Capitalized first letter is fine —
    // autocorrect guards proper nouns separately (see `starts_lowercase`).
    let letters: Vec<char> = raw.chars().filter(|c| c.is_alphabetic()).collect();
    let all_upper = letters.iter().all(|c| c.is_uppercase());
    let internal_upper = letters.iter().skip(1).any(|c| c.is_uppercase());
    !(all_upper || internal_upper)
}

/// The misspelled words to mark, as the composer last computed them.
///
/// iced re-runs the whole highlighter whenever settings compare unequal, so
/// equality is the monotonic `revision` alone — comparing two `HashSet`s on
/// every relayout would be the expensive part of an otherwise cheap pass. The
/// composer bumps `revision` only when the set actually changes.
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub revision: u64,
    pub misspelled: Arc<HashSet<String>>,
}

impl PartialEq for Settings {
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision
    }
}

/// Colours the words in [`Settings::misspelled`] wherever they appear.
#[derive(Debug)]
pub struct Highlighter {
    settings: Settings,
    current_line: usize,
}

impl iced::advanced::text::Highlighter for Highlighter {
    type Settings = Settings;
    type Highlight = ();
    type Iterator<'a> = std::vec::IntoIter<(Range<usize>, ())>;

    fn new(settings: &Self::Settings) -> Self {
        Self { settings: settings.clone(), current_line: 0 }
    }

    fn update(&mut self, new_settings: &Self::Settings) {
        self.settings = new_settings.clone();
        self.current_line = 0;
    }

    fn change_line(&mut self, line: usize) {
        self.current_line = line;
    }

    fn highlight_line(&mut self, line: &str) -> Self::Iterator<'_> {
        self.current_line += 1;
        // The overwhelmingly common case (a clean draft, or spell check
        // turned off) shouldn't tokenise anything at all.
        if self.settings.misspelled.is_empty() {
            return Vec::new().into_iter();
        }
        words(line)
            .filter(|word| {
                is_checkable(word.raw) && self.settings.misspelled.contains(word.core)
            })
            .map(|word| (word.range, ()))
            .collect::<Vec<_>>()
            .into_iter()
    }

    fn current_line(&self) -> usize {
        self.current_line
    }
}

/// Draws a flagged word in the theme's danger colour. A plain `fn` because
/// that's what `text_editor::highlight_with` takes.
pub fn format(_highlight: &(), theme: &iced::Theme) -> Format<iced::Font> {
    Format { color: Some(theme.extended_palette().danger.base.color), font: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::advanced::text::Highlighter as _;

    fn collect(line: &str) -> Vec<(Range<usize>, &str, &str)> {
        words(line).map(|w| (w.range, w.core, w.raw)).collect()
    }

    #[test]
    fn words_split_on_whitespace_and_trim_to_the_core() {
        assert_eq!(
            collect("i recieved teh file"),
            vec![
                (0..1, "i", "i"),
                (2..10, "recieved", "recieved"),
                (11..14, "teh", "teh"),
                (15..19, "file", "file"),
            ]
        );
    }

    #[test]
    fn punctuation_is_trimmed_from_the_core_but_kept_in_raw() {
        // The range excludes the comma so a replacement can't eat it; the raw
        // token keeps it so the skip heuristics see the whole thing.
        assert_eq!(collect("(wat,)"), vec![(1..4, "wat", "(wat,)")]);
        // Internal punctuation stays — the speller does its own word breaking.
        assert_eq!(collect("don't"), vec![(0..5, "don't", "don't")]);
    }

    #[test]
    fn ranges_are_utf8_byte_offsets() {
        // 'é' is two bytes — the range must land on char boundaries.
        let line = "a café";
        let found = collect(line);
        assert_eq!(found[1].0, 2..7);
        assert_eq!(&line[found[1].0.clone()], "café");
    }

    #[test]
    fn tokens_without_a_core_are_skipped_not_terminal() {
        // The "---" must not end the iteration and hide "teh" behind it.
        assert_eq!(
            collect("ok --- teh"),
            vec![(0..2, "ok", "ok"), (7..10, "teh", "teh")]
        );
    }

    #[test]
    fn irregular_whitespace_does_not_shift_ranges() {
        let line = "  teh\tfile  ";
        assert_eq!(collect(line), vec![(2..5, "teh", "teh"), (6..10, "file", "file")]);
    }

    #[test]
    fn checkable_accepts_prose_rejects_chat_tokens() {
        assert!(is_checkable("teh"));
        assert!(is_checkable("hello"));
        assert!(is_checkable("Hello")); // capitalized is fine for the bar

        assert!(!is_checkable("a")); // needs 2+ letters
        assert!(!is_checkable("GG")); // acronym
        assert!(!is_checkable("camelCase")); // code
        assert!(!is_checkable("v2")); // has a digit
        assert!(!is_checkable("@bob")); // mention
        assert!(!is_checkable(":smile:")); // emoji shortcode
        assert!(!is_checkable("http://x.com")); // url
        assert!(!is_checkable("a/b")); // path
        assert!(!is_checkable("co_op")); // identifier
    }

    fn highlighter(words: &[&str]) -> Highlighter {
        let misspelled = words.iter().map(|w| w.to_string()).collect();
        Highlighter::new(&Settings { revision: 1, misspelled: Arc::new(misspelled) })
    }

    #[test]
    fn flagged_words_are_highlighted_where_they_appear() {
        let mut h = highlighter(&["recieved", "teh"]);
        let ranges: Vec<Range<usize>> =
            h.highlight_line("i recieved teh file").map(|(r, ())| r).collect();
        assert_eq!(ranges, vec![2..10, 11..14]);
    }

    #[test]
    fn a_repeated_typo_is_flagged_every_time() {
        let mut h = highlighter(&["teh"]);
        let ranges: Vec<Range<usize>> =
            h.highlight_line("teh cat and teh dog").map(|(r, ())| r).collect();
        assert_eq!(ranges, vec![0..3, 12..15]);
    }

    #[test]
    fn unflagged_tokens_matching_a_flagged_word_are_still_skipped() {
        // "teh" inside a URL is not prose, even though the word is flagged.
        let mut h = highlighter(&["teh"]);
        let ranges: Vec<Range<usize>> =
            h.highlight_line("http://teh/x teh").map(|(r, ())| r).collect();
        assert_eq!(ranges, vec![13..16]);
    }

    #[test]
    fn an_empty_set_highlights_nothing() {
        let mut h = Highlighter::new(&Settings::default());
        assert_eq!(h.highlight_line("i recieved teh file").count(), 0);
    }

    #[test]
    fn settings_compare_by_revision_only() {
        let a = Settings { revision: 7, misspelled: Arc::new(HashSet::new()) };
        let b = Settings {
            revision: 7,
            misspelled: Arc::new(["teh".to_string()].into_iter().collect()),
        };
        // Same revision means "nothing changed" — the composer is responsible
        // for bumping it whenever the set does.
        assert_eq!(a, b);
        assert_ne!(a, Settings { revision: 8, misspelled: a.misspelled.clone() });
    }
}
