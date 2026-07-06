//! Message composer: @mention autocomplete, emoji picker (unicode + custom
//! emoji packs), and attachment staging — picked/pasted files wait as chips
//! above the input until Enter/Send, with any typed text riding out as the
//! first file's caption (MSC2530). This
//! module never talks to `client_core::sync`/`mpsc` directly — it only
//! produces `Effect`s, which the root dispatcher (`ui::update`) turns into
//! actual `ClientCommand` sends, generating and tracking the `request_id`
//! needed to correlate the eventual
//! `ClientEvent::CommandSucceeded`/`CommandFailed`.

use std::collections::HashMap;
use std::ops::Range;

use client_core::commands::RequestId;
use client_core::events::{EmojiPack, ReplyPreview, RoomMember};
use iced::widget::{button, column, container, row, text, text_input};
use iced::{Element, Length, Task};

use crate::spellcheck_config::SpellcheckConfig;

/// Which tab the composer's picker shows while open. Set by whichever button
/// opened it (emoji vs sticker) and by the in-panel tab bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PickerTab {
    #[default]
    Emoji,
    Sticker,
}

#[derive(Debug, Clone, Default)]
pub struct State {
    pub body: String,
    pub show_emoji_picker: bool,
    pub picker_tab: PickerTab,
    pub member_candidates: Vec<RoomMember>,
    /// Lowercased display names, index-parallel to `member_candidates`
    /// (built once per roster update) — the mention filter runs on every
    /// view rebuild while an '@word' ends the draft, and lowercasing the
    /// whole roster per frame allocated thousands of Strings in big rooms.
    pub member_candidates_lower: Vec<String>,
    /// Mentions the user has confirmed by clicking an autocomplete
    /// candidate; attached as `m.mentions` on send, then cleared.
    pub mentioned: Vec<(String, String)>,
    /// Set while composing a reply — shown as a banner above the input and
    /// attached as the rich-reply relation on send.
    pub replying_to: Option<ReplyPreview>,
    pub pending_request: Option<RequestId>,
    /// In-flight attachment upload, tracked separately from text sends: if
    /// it shared `pending_request`, an attachment's CommandSucceeded would
    /// run the SendSucceeded reset and wipe a typed-but-unsent draft.
    pub pending_attachment_request: Option<RequestId>,
    /// In-flight sticker send. A separate slot (like attachments) so a sticker
    /// send never runs the text-draft reset that a `SendSucceeded` would.
    pub pending_sticker_request: Option<RequestId>,
    /// Attachments staged in the composer (picked or pasted), shown as chips
    /// above the input. Nothing uploads until Enter/Send; the typed text (if
    /// any) goes out as the FIRST file's caption. While a batch is sending,
    /// the front entry is the in-flight upload — it stays staged until the
    /// server takes it, so a failure can be retried without re-picking.
    /// Dropped with the rest of the composer on room switch.
    pub staged_attachments: Vec<StagedAttachment>,
    /// How many entries at the front of `staged_attachments` belong to the
    /// Enter-batch currently sending. The pipeline stops there: files staged
    /// *during* an upload wait for their own Enter instead of being swept
    /// into a batch the user already dispatched.
    pub sending_remaining: usize,
    /// Text snapshot taken when Enter dispatched a batch (the trimmed body
    /// rides as the first file's caption). Held until that first send
    /// resolves: a failure puts the draft back instead of losing it.
    pub carried: Option<CarriedText>,
    pub error: Option<String>,
    /// Spell-check suggestion bar + autocorrect bookkeeping (all plain data;
    /// the Windows speller is only touched in `update`).
    pub spell: SpellState,
}

/// A file waiting in the composer to be sent (picked via the dialog or
/// pasted from the clipboard).
#[derive(Debug, Clone)]
pub struct StagedAttachment {
    pub filename: String,
    pub bytes: Vec<u8>,
    /// Sniffed from the filename once at staging time.
    pub mime: String,
    /// Chip thumbnail, pre-built once at staging time (`image/*` only) —
    /// building a fresh handle per view frame would re-decode and re-upload
    /// the texture every frame.
    pub preview: Option<iced::widget::image::Handle>,
}

/// The text/mentions/reply captured when Enter dispatched an attachment
/// batch — the trimmed body becomes the first file's caption. Kept until
/// that send resolves so a failure restores the draft instead of eating it.
#[derive(Debug, Clone)]
pub struct CarriedText {
    pub body: String,
    pub mentioned: Vec<(String, String)>,
    pub replying_to: Option<ReplyPreview>,
}

/// Spell-check state for the composer, recomputed on every edit. Holds only
/// the speller's plain-data verdict so `view` never has to talk to COM.
#[derive(Debug, Clone, Default)]
pub struct SpellState {
    /// The flagged word the suggestion bar targets, or `None`.
    pub flagged: Option<Flagged>,
    /// Set for exactly one edit after an autocorrect: if the next edit is the
    /// Backspace that would delete the space we just added, we restore the
    /// original word instead ("undo autocorrect", like a phone keyboard).
    pending_revert: Option<Revert>,
    /// Memo of the speller's verdict for the last word checked, so the
    /// synchronous COM call isn't repeated on every keystroke while the
    /// draft ends in whitespace and the trailing word hasn't changed.
    /// Keyed by the word (not its range — edits earlier in the body shift
    /// the range without changing the word).
    last_checked: Option<(String, crate::spellcheck::Analysis)>,
}

/// A misspelled word the suggestion bar is offering fixes for.
#[derive(Debug, Clone)]
pub struct Flagged {
    /// Byte range of the word within `State::body`.
    pub range: Range<usize>,
    pub word: String,
    pub suggestions: Vec<String>,
}

#[derive(Debug, Clone)]
struct Revert {
    /// Body value a single Backspace produces (corrected text minus the
    /// trailing space) — the trigger that means "undo the autocorrect".
    undo_trigger: String,
    /// Body to restore on undo (the user's original word, no trailing space).
    reverted: String,
}

impl SpellState {
    /// Re-runs the speller on the last completed word of `body` and updates
    /// the suggestion bar. Clears the bar (a no-op otherwise) when spell check
    /// is disabled, or the trailing word is still being typed / isn't prose.
    fn recompute(&mut self, body: &str, cfg: &SpellcheckConfig) {
        self.recompute_cached(body, cfg, None);
    }

    /// Like [`Self::recompute`], but reuses `cached` (an analysis of the
    /// trailing word computed this same edit, e.g. by autocorrect) instead of
    /// asking the speller again. Also memoizes per word so repeated calls
    /// while the trailing word is unchanged (every keystroke of a mid-body
    /// edit while the draft ends in whitespace) skip the COM round trip.
    fn recompute_cached(
        &mut self,
        body: &str,
        cfg: &SpellcheckConfig,
        cached: Option<crate::spellcheck::Analysis>,
    ) {
        self.flagged = None;
        if !cfg.enabled {
            return;
        }
        let Some((range, word, raw)) = last_completed_word(body) else {
            return;
        };
        if !is_checkable(&raw) {
            return;
        }
        let analysis = if let Some(analysis) = cached {
            // Fresh from this same edit (autocorrect ran the speller and
            // left the body untouched) — reuse it and refresh the memo.
            self.last_checked = Some((word.clone(), analysis.clone()));
            analysis
        } else {
            match &self.last_checked {
                Some((checked, verdict)) if *checked == word => verdict.clone(),
                _ => {
                    let fresh = crate::spellcheck::analyze(&word);
                    self.last_checked = Some((word.clone(), fresh.clone()));
                    fresh
                }
            }
        };
        if analysis.misspelled && !analysis.suggestions.is_empty() {
            self.flagged = Some(Flagged { range, word, suggestions: analysis.suggestions });
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    BodyChanged(String),
    Send,
    ToggleEmojiPicker,
    ToggleStickerPicker,
    SelectPickerTab(PickerTab),
    EmojiPicked(&'static str),
    CustomEmojiPicked { shortcode: String, mxc_url: String },
    /// A sticker was picked from the sticker tab — sent immediately as an
    /// `m.sticker` (the picker stays open so several can go out in a row).
    StickerPicked { url: String, body: String, width: Option<u32>, height: Option<u32> },
    MentionCandidateClicked(String, String),
    PickAttachment,
    /// A file's bytes arrived (dialog pick or clipboard paste) — staged as
    /// a chip above the input, NOT sent; Enter/Send dispatches it.
    AttachmentPicked(Result<(String, Vec<u8>), String>),
    /// × clicked on a staged-attachment chip.
    RemoveStagedAttachment(usize),

    /// A suggestion-bar button was clicked — replace the flagged word with it.
    SpellSuggestionPicked(String),
    /// "Add to dictionary" was clicked for the flagged word.
    SpellAddToDictionary,

    CancelReply,

    /// Fed back by the root dispatcher once the in-flight command resolves.
    SendSucceeded,
    SendFailed(String),
}

pub enum Effect {
    None,
    Send { body: String, mentioned_user_ids: Vec<String>, reply_to_event_id: Option<String> },
    PickAttachment,
    /// Upload+send one attachment. `caption`/`mentioned_user_ids`/
    /// `reply_to_event_id` ride on the event itself (MSC2530 caption) —
    /// only the first file of an Enter-batch carries them.
    SendAttachment {
        filename: String,
        bytes: Vec<u8>,
        mime: String,
        caption: Option<String>,
        mentioned_user_ids: Vec<String>,
        reply_to_event_id: Option<String>,
    },
    Typing(bool),
    EnsureEmojiFetched(Vec<String>),
    /// The sticker tab was opened/selected — the root dispatcher ensures the
    /// collected stickers' images are fetched (pack images are already
    /// fetched when packs load).
    EnsureStickersFetched,
    /// A sticker was picked — post it as an `m.sticker` event.
    SendSticker { url: String, body: String, width: Option<u32>, height: Option<u32> },
    /// An emoji was used — the root dispatcher bumps the usage history
    /// that feeds the picker's "Frequently used" section. Key: the glyph
    /// for unicode, the `mxc://` URL for custom emoji.
    EmojiUsed(String),
}

pub fn update(
    state: &mut State,
    message: Message,
    spell: &SpellcheckConfig,
) -> (Task<Message>, Effect) {
    match message {
        Message::BodyChanged(body) => {
            let typing = Effect::Typing(!body.trim().is_empty());
            let previous = std::mem::replace(&mut state.body, body);
            // A stale send/attach error shouldn't pin itself above the
            // composer once the user has moved on.
            state.error = None;

            // Backspace immediately after an autocorrect undoes it (restores
            // the original word) instead of just deleting the space.
            if let Some(revert) = state.spell.pending_revert.take() {
                if state.body == revert.undo_trigger {
                    state.body = revert.reverted;
                    state.spell.recompute(&state.body, spell);
                    return (Task::none(), typing);
                }
            }

            // Autocorrect only fires on the "typed a space at the end" edit —
            // the one shape we can locate the finished word in without a
            // cursor position from the text input. When it ran the speller
            // and left the body untouched, its analysis is handed straight
            // to recompute so the word isn't analyzed twice per space.
            let cached = if spell.autocorrect && typed_trailing_boundary(&previous, &state.body) {
                maybe_autocorrect(state)
            } else {
                None
            };

            state.spell.recompute_cached(&state.body, spell, cached);
            (Task::none(), typing)
        }
        Message::Send => {
            // Attachments staged? Enter sends them, and the typed text (if
            // any) rides along as the first file's caption — one event, not
            // an attachment plus a separate text message.
            if !state.staged_attachments.is_empty() {
                // In-flight guard, same shape as the text path's below: the
                // upload slot is single, and a second Enter mid-batch would
                // double-send the front file.
                if state.pending_attachment_request.is_some() {
                    return (Task::none(), Effect::None);
                }
                let carried = CarriedText {
                    body: std::mem::take(&mut state.body),
                    mentioned: std::mem::take(&mut state.mentioned),
                    replying_to: state.replying_to.take(),
                };
                state.error = None;
                state.spell = SpellState::default();
                state.sending_remaining = state.staged_attachments.len();

                let caption = {
                    let trimmed = carried.body.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_string())
                };
                let mentioned_user_ids =
                    carried.mentioned.iter().map(|(id, _)| id.clone()).collect();
                let reply_to_event_id =
                    carried.replying_to.as_ref().map(|r| r.event_id.clone());
                // The entry stays staged (its chip shows "uploading") until
                // the server takes it — the bytes must survive a failure for
                // retry, hence the clone.
                let first = &state.staged_attachments[0];
                let effect = Effect::SendAttachment {
                    filename: first.filename.clone(),
                    bytes: first.bytes.clone(),
                    mime: first.mime.clone(),
                    caption,
                    mentioned_user_ids,
                    reply_to_event_id,
                };
                state.carried = Some(carried);
                return (Task::none(), effect);
            }

            // In-flight guard: a second Enter (or Enter + Send click) before
            // CommandSucceeded round-trips would post the message twice —
            // and overwrite pending_request, orphaning the first response.
            // pending_request is always cleared by SendSucceeded/SendFailed,
            // and the composer resets wholesale on room switch, so this
            // can't wedge.
            if state.pending_request.is_some() {
                return (Task::none(), Effect::None);
            }
            let body = state.body.trim().to_string();
            if body.is_empty() {
                return (Task::none(), Effect::None);
            }
            let mentioned_user_ids = state.mentioned.iter().map(|(id, _)| id.clone()).collect();
            let reply_to_event_id = state.replying_to.as_ref().map(|r| r.event_id.clone());
            (Task::none(), Effect::Send { body, mentioned_user_ids, reply_to_event_id })
        }
        Message::CancelReply => {
            state.replying_to = None;
            (Task::none(), Effect::None)
        }
        Message::ToggleEmojiPicker => {
            // Close if it's already on the emoji tab; otherwise open it (or
            // switch to the emoji tab if the sticker tab was showing).
            if state.show_emoji_picker && state.picker_tab == PickerTab::Emoji {
                state.show_emoji_picker = false;
                return (Task::none(), Effect::None);
            }
            state.show_emoji_picker = true;
            state.picker_tab = PickerTab::Emoji;
            (Task::none(), Effect::EnsureEmojiFetched(crate::emoji_picker::all_unicode_glyphs()))
        }
        Message::ToggleStickerPicker => {
            if state.show_emoji_picker && state.picker_tab == PickerTab::Sticker {
                state.show_emoji_picker = false;
                return (Task::none(), Effect::None);
            }
            state.show_emoji_picker = true;
            state.picker_tab = PickerTab::Sticker;
            (Task::none(), Effect::EnsureStickersFetched)
        }
        Message::SelectPickerTab(tab) => {
            state.picker_tab = tab;
            let effect = match tab {
                PickerTab::Emoji => {
                    Effect::EnsureEmojiFetched(crate::emoji_picker::all_unicode_glyphs())
                }
                PickerTab::Sticker => Effect::EnsureStickersFetched,
            };
            (Task::none(), effect)
        }
        Message::StickerPicked { url, body, width, height } => {
            // Fire-and-forget, like a reaction: the picker stays open so a
            // few stickers can go out in a row.
            (Task::none(), Effect::SendSticker { url, body, width, height })
        }
        Message::EmojiPicked(glyph) => {
            state.body.push_str(glyph);
            // The body changed by insertion, not by the undo-trigger
            // Backspace — a stale revert would misfire on a later deletion
            // and rewrite text the user didn't ask to restore.
            state.spell.pending_revert = None;
            state.spell.recompute(&state.body, spell);
            (Task::none(), Effect::EmojiUsed(glyph.to_string()))
        }
        Message::CustomEmojiPicked { shortcode, mxc_url } => {
            state.body.push_str(&format!(":{shortcode}: "));
            state.spell.pending_revert = None;
            state.spell.recompute(&state.body, spell);
            (Task::none(), Effect::EmojiUsed(mxc_url))
        }
        Message::MentionCandidateClicked(user_id, display_name) => {
            if let Some(at_pos) = state.body.rfind('@') {
                state.body.truncate(at_pos);
            }
            state.body.push('@');
            state.body.push_str(&display_name);
            state.body.push(' ');
            if !state.mentioned.iter().any(|(id, _)| *id == user_id) {
                state.mentioned.push((user_id, display_name));
            }
            state.spell.pending_revert = None;
            // A just-picked mention is never a typo — don't spell-flag the
            // tail of a multi-word display name ("@John Smyth" → "Smyth"
            // would pop "Did you mean: Smith", and clicking it would corrupt
            // the mention text). Any later edit recomputes via BodyChanged.
            state.spell.flagged = None;
            (Task::none(), Effect::None)
        }
        Message::PickAttachment => (Task::none(), Effect::PickAttachment),
        Message::AttachmentPicked(Ok((filename, bytes))) => {
            // Stage it — nothing uploads until Enter/Send. An identical
            // payload already staged is a key-repeat echo of the same Ctrl+V
            // (iced 0.13 exposes no repeat flag to filter on) or a double
            // pick; the visible chip already says it's attached, so skip it
            // rather than stacking duplicates.
            if state
                .staged_attachments
                .iter()
                .any(|staged| staged.filename == filename && staged.bytes == bytes)
            {
                return (Task::none(), Effect::None);
            }
            let mime = mime_guess::from_path(&filename).first_or_octet_stream().to_string();
            let preview = mime
                .starts_with("image/")
                .then(|| iced::widget::image::Handle::from_bytes(bytes.clone()));
            state.staged_attachments.push(StagedAttachment { filename, bytes, mime, preview });
            state.error = None;
            (Task::none(), Effect::None)
        }
        Message::AttachmentPicked(Err(reason)) => {
            state.error = Some(reason);
            (Task::none(), Effect::None)
        }
        Message::RemoveStagedAttachment(index) => {
            // The front chip is the in-flight upload while a batch sends;
            // its × is disabled in `view` (removing it couldn't cancel the
            // upload), so refuse it here too.
            if index < state.staged_attachments.len()
                && !(index == 0 && state.pending_attachment_request.is_some())
            {
                state.staged_attachments.remove(index);
                // If it was part of the batch currently sending, the batch
                // shrinks with it.
                if index < state.sending_remaining {
                    state.sending_remaining -= 1;
                }
            }
            (Task::none(), Effect::None)
        }
        Message::SpellSuggestionPicked(replacement) => {
            if let Some(flagged) = state.spell.flagged.take() {
                // Defensive: only replace if the range still holds the exact
                // word we flagged, so a body edited out from under the bar is
                // never corrupted.
                if state.body.get(flagged.range.clone()) == Some(flagged.word.as_str()) {
                    state.body.replace_range(flagged.range, &replacement);
                }
            }
            state.spell.pending_revert = None;
            state.spell.recompute(&state.body, spell);
            (Task::none(), Effect::None)
        }
        Message::SpellAddToDictionary => {
            if let Some(flagged) = state.spell.flagged.take() {
                crate::spellcheck::add_to_dictionary(&flagged.word);
            }
            // The dictionary just changed — the memoized verdict for this
            // word is stale (it would keep flagging the word just added).
            state.spell.last_checked = None;
            state.spell.recompute(&state.body, spell);
            (Task::none(), Effect::None)
        }
        Message::SendSucceeded => {
            state.body.clear();
            state.mentioned.clear();
            state.replying_to = None;
            state.pending_request = None;
            state.error = None;
            state.spell = SpellState::default();
            (Task::none(), Effect::Typing(false))
        }
        Message::SendFailed(reason) => {
            state.pending_request = None;
            state.error = Some(reason);
            (Task::none(), Effect::None)
        }
    }
}

/// The `@partial` word currently being typed at the end of the composer, if
/// any — drives the mention-autocomplete list. Only looks at the trailing
/// word (simple, correct for top-to-bottom typing; editing a mention
/// mid-message won't retrigger the dropdown, an acceptable trade-off here).
fn active_mention_query(body: &str) -> Option<&str> {
    let last_word = body.rsplit(char::is_whitespace).next()?;
    last_word.strip_prefix('@')
}

/// Applies the speller's high-confidence replacement to the just-finished
/// word, if it offers one, and records how to undo it on the next Backspace.
/// Called only after `typed_trailing_boundary`, so the finished word is the
/// last completed word of `body`.
///
/// Returns the analysis when the speller ran AND the body was left untouched
/// — the caller hands it to `recompute_cached` so the same word isn't
/// analyzed twice per space. Returns `None` when the speller never ran or
/// the body was rewritten (the analysis would describe the old word).
fn maybe_autocorrect(state: &mut State) -> Option<crate::spellcheck::Analysis> {
    let (range, word, raw) = last_completed_word(&state.body)?;
    // Don't silently rewrite mentions/URLs/code, and leave leading-capital
    // words (names, sentence starts) alone — the suggestion bar still offers
    // those, but autocorrect shouldn't touch them.
    if !is_checkable(&raw) || !starts_lowercase(&word) {
        return None;
    }
    let analysis = crate::spellcheck::analyze(&word);
    let Some(replacement) = analysis.replacement.clone() else {
        return Some(analysis);
    };
    if replacement == word {
        return Some(analysis);
    }
    // The last char of the body is the boundary (space) the user just typed;
    // both the undo trigger and the restore target drop it.
    let Some(boundary) = state.body.chars().next_back() else {
        return Some(analysis);
    };
    let boundary_len = boundary.len_utf8();

    let original_body = state.body.clone();
    state.body.replace_range(range, &replacement);

    let undo_trigger = state.body[..state.body.len() - boundary_len].to_string();
    let reverted = original_body[..original_body.len() - boundary_len].to_string();
    state.spell.pending_revert = Some(Revert { undo_trigger, reverted });
    None
}

/// The last whitespace-completed word in `body`: its byte range, the cleaned
/// word (surrounding punctuation stripped), and the raw whitespace-delimited
/// token it came from (used for the skip decisions in [`is_checkable`]).
/// `None` while the user is still typing the final word — i.e. `body` doesn't
/// end in whitespace — so a half-typed word is never flagged or corrected.
fn last_completed_word(body: &str) -> Option<(Range<usize>, String, String)> {
    if !body.chars().next_back()?.is_whitespace() {
        return None;
    }
    // End of the token: just past the last non-whitespace char.
    let token_end = body
        .char_indices()
        .rev()
        .find(|(_, c)| !c.is_whitespace())
        .map(|(i, c)| i + c.len_utf8())?;
    // Start of the token: just past the previous whitespace (or the start).
    let token_start = body[..token_end]
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    let raw = &body[token_start..token_end];

    // Trim to the alphanumeric core so the range we'd replace excludes
    // surrounding punctuation ("helo," → correct just "helo").
    let core_start = raw.char_indices().find(|(_, c)| c.is_alphanumeric()).map(|(i, _)| i)?;
    let core_end = raw
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_alphanumeric())
        .map(|(i, c)| i + c.len_utf8())?;
    let range = (token_start + core_start)..(token_start + core_end);
    let word = body[range.clone()].to_string();
    Some((range, word, raw.to_string()))
}

/// Whether a raw token is ordinary prose worth spell-checking — filters out
/// the things chat is full of that a dictionary would wrongly flag: mentions,
/// emoji shortcodes, URLs/paths, code-ish identifiers, acronyms, and anything
/// carrying a digit.
fn is_checkable(raw: &str) -> bool {
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

/// Autocorrect only rewrites words that start lowercase — a leading capital
/// usually marks a name or sentence-start proper noun we shouldn't touch.
fn starts_lowercase(word: &str) -> bool {
    word.chars().next().is_some_and(|c| c.is_lowercase())
}

/// True when `current` is `previous` with exactly one trailing whitespace
/// char appended — the "typed a space at the very end" edit. Restricting
/// autocorrect to this shape avoids mangling mid-string edits or pastes,
/// which we can't locate without a cursor position.
fn typed_trailing_boundary(previous: &str, current: &str) -> bool {
    match current.strip_prefix(previous) {
        Some(added) => {
            let mut chars = added.chars();
            chars.next().is_some_and(|c| c.is_whitespace()) && chars.next().is_none()
        }
        None => false,
    }
}

/// Stable widget id for the composer's text input — lets the root dispatcher
/// refocus it after staging a pasted/picked attachment, so "paste → type a
/// caption → Enter" flows without an extra click.
pub fn input_id() -> text_input::Id {
    text_input::Id::new("composer-input")
}

/// "412 B" / "3.2 KB" / "8.1 MB" — size tag on a staged-attachment chip.
fn human_size(bytes: usize) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    let b = bytes as f64;
    if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

pub fn view<'a>(
    state: &'a State,
    media: &'a crate::media_cache::State,
    typing: Element<'a, Message>,
    followers: Element<'a, Message>,
) -> Element<'a, Message> {
    // Everything above the input renders inside always-present slots so
    // the input never shifts tree position (which would drop its focus) —
    // see `theme::slot`. The mention list is the critical one: it appears
    // and disappears *while the user is typing*.
    let error_slot = crate::theme::slot(
        state.error.as_ref().map(|error| text(error.clone()).style(text::danger).size(13).into()),
    );

    let reply_slot = crate::theme::slot(state.replying_to.as_ref().map(|reply| {
        let mut banner = row![].spacing(8).align_y(iced::Center);
        banner = banner
            .push(text(crate::theme::icon::REPLY).size(12).font(crate::theme::ICON_FONT).style(text::primary))
            .push(text(format!("Replying to {}", reply.sender)).size(12).style(text::primary));
        if let Some(thumb) = reply
            .image_url
            .as_deref()
            .and_then(|url| crate::media_cache::mxc_visual(media, url, 28, Some(28)))
        {
            banner = banner.push(thumb);
        }
        banner = banner
            .push(text(reply.snippet.clone()).size(12).style(text::secondary).width(Length::Fill))
            .push(
                button(text("×").size(13))
                    .on_press(Message::CancelReply)
                    .style(crate::theme::ghost_button)
                    .padding([0, 6]),
            );
        container(banner).padding([4, 8]).style(crate::theme::panel).into()
    }));

    let mention_slot = crate::theme::slot(active_mention_query(&state.body).and_then(|query| {
        let query_lower = query.to_lowercase();
        let matches: Vec<&RoomMember> = state
            .member_candidates
            .iter()
            .zip(state.member_candidates_lower.iter())
            .filter(|(_, lower)| lower.starts_with(&query_lower))
            .map(|(member, _)| member)
            .take(6)
            .collect();

        if matches.is_empty() {
            return None;
        }
        let mut list = column![].spacing(2);
        for member in matches {
            list = list.push(
                button(text(member.display_name.clone()).size(13))
                    .on_press(Message::MentionCandidateClicked(
                        member.user_id.clone(),
                        member.display_name.clone(),
                    ))
                    .width(Length::Fill)
                    .style(button::text),
            );
        }
        Some(container(list).padding(4).into())
    }));

    // Spell-check suggestions for the word just finished. A slot like the
    // rest, so it never reshapes the tree under the input (which would drop
    // focus mid-typing). Non-destructive: tapping a button rewrites the word,
    // otherwise it's ignorable.
    let spell_slot = crate::theme::slot(state.spell.flagged.as_ref().map(|flagged| {
        let mut bar = row![text("Did you mean").size(12).style(text::secondary)]
            .spacing(6)
            .align_y(iced::Center);
        for suggestion in &flagged.suggestions {
            bar = bar.push(
                button(text(suggestion.clone()).size(13))
                    .on_press(Message::SpellSuggestionPicked(suggestion.clone()))
                    .style(crate::theme::ghost_button)
                    .padding([2, 8]),
            );
        }
        bar = bar.push(
            button(text("Add to dictionary").size(12))
                .on_press(Message::SpellAddToDictionary)
                .style(button::text)
                .padding([2, 8]),
        );
        container(bar).padding([2, 4]).into()
    }));

    // Staged attachments (picked or pasted, not yet sent): one chip per
    // file — thumbnail for images, name, size, × to unstage. While a batch
    // uploads, the front chip is the in-flight file: its label says so and
    // its × is disabled (removal couldn't cancel the upload).
    let staged_slot = crate::theme::slot((!state.staged_attachments.is_empty()).then(|| {
        let uploading = state.pending_attachment_request.is_some();
        let mut chips = row![].spacing(6).align_y(iced::Center);
        for (index, staged) in state.staged_attachments.iter().enumerate() {
            let is_uploading = index == 0 && uploading;
            let mut chip = row![].spacing(6).align_y(iced::Center);
            if let Some(preview) = &staged.preview {
                chip = chip.push(
                    iced::widget::image(preview.clone()).height(Length::Fixed(28.0)),
                );
            } else {
                chip = chip.push(
                    text(crate::theme::icon::ATTACH)
                        .size(12)
                        .font(crate::theme::ICON_FONT)
                        .style(text::primary),
                );
            }
            let label = if is_uploading {
                format!("{} — uploading…", staged.filename)
            } else {
                staged.filename.clone()
            };
            chip = chip
                .push(text(label).size(12))
                .push(text(human_size(staged.bytes.len())).size(11).style(text::secondary));
            let mut remove = button(text("×").size(13))
                .style(crate::theme::ghost_button)
                .padding([0, 6]);
            if !is_uploading {
                remove = remove.on_press(Message::RemoveStagedAttachment(index));
            }
            chip = chip.push(remove);
            chips = chips.push(container(chip).padding([2, 6]).style(crate::theme::panel));
        }
        // Horizontal scroll rather than clipping when many files are staged
        // (the chips row can outgrow the composer width).
        iced::widget::scrollable(chips)
            .direction(iced::widget::scrollable::Direction::Horizontal(
                iced::widget::scrollable::Scrollbar::new().width(3).scroller_width(3),
            ))
            .into()
    }));

    // The emoji/sticker picker panel is NOT part of this column: it floats
    // over the message area as a layer (see the chat stack in
    // `timeline::view`), so opening it doesn't grow the composer and shove
    // the whole timeline upward.
    let mut col =
        column![error_slot, reply_slot, staged_slot, mention_slot, spell_slot].spacing(4);

    // One compact row (Cinny-style): attachment on the left, the input
    // filling the middle, then the emoji/sticker pickers and Send clustered on
    // the right. Icon-only (Windows Fluent glyphs).
    let placeholder = if state.staged_attachments.is_empty() {
        "Message... (@mention, markdown supported)"
    } else {
        "Add a caption… (optional) — Enter sends the attachment"
    };
    let input: Element<'_, Message> = text_input(placeholder, &state.body)
        .id(input_id())
        .on_input(Message::BodyChanged)
        .on_submit(Message::Send)
        .padding(6)
        .width(Length::Fill)
        .into();

    let input_row = row![
        button(crate::theme::icon_text(crate::theme::icon::ATTACH, 15))
            .on_press(Message::PickAttachment)
            .style(crate::theme::ghost_button)
            .padding(6),
        input,
        button(crate::theme::icon_text(crate::theme::icon::REACT, 15))
            .on_press(Message::ToggleEmojiPicker)
            .style(crate::theme::ghost_button)
            .padding(6),
        button(crate::theme::icon_text(crate::theme::icon::STICKER, 15))
            .on_press(Message::ToggleStickerPicker)
            .style(crate::theme::ghost_button)
            .padding(6),
        button(crate::theme::icon_text(crate::theme::icon::SEND, 15))
            .on_press(Message::Send)
            .padding([6, 12]),
    ]
    .spacing(4)
    .align_y(iced::Center);

    // A thin status line under the input: "X is typing…" on the left, the
    // read-receipt follower avatars ("who's caught up") on the right — the
    // Cinny layout. Always present (never a slot) so it can't reshape the tree
    // and drop the input's focus as it fills or empties.
    let status_line = row![container(typing).width(Length::Fill), followers]
        .spacing(6)
        .align_y(iced::Center)
        .padding([0, 4]);

    col = col.push(input_row);
    col = col.push(status_line);

    container(col).padding([6, 8]).width(Length::Fill).into()
}

/// The composer's picker panel: a Sticker | Emoji tab bar over either the
/// sticker grid or the emoji list. Rendered by `timeline::view` as a layer
/// floating over the bottom-right of the chat — not inline in the composer
/// column — so it covers messages instead of pushing them up. (The
/// timeline's reaction picker calls `emoji_picker::view` directly and stays
/// emoji-only — you react with emoji, not stickers.)
pub(super) fn picker_panel<'a>(
    state: &'a State,
    emoji_usage: &'a HashMap<String, u32>,
    media: &'a crate::media_cache::State,
    packs: &'a [EmojiPack],
    stickers: &'a [crate::state::CollectedSticker],
) -> Element<'a, Message> {
    let tab = |label: &'a str, this: PickerTab| {
        let style = if state.picker_tab == this {
            crate::theme::selected_ghost_button
        } else {
            crate::theme::ghost_button
        };
        button(text(label).size(13))
            .on_press(Message::SelectPickerTab(this))
            .style(style)
            .padding([4, 10])
    };
    // Emoji left, Sticker right — same order as the toolbar buttons under
    // the panel, so tab and button don't sit crossed over each other.
    let tabs =
        row![tab("Emoji", PickerTab::Emoji), tab("Sticker", PickerTab::Sticker)].spacing(4);

    let content: Element<'a, Message> = match state.picker_tab {
        PickerTab::Sticker => crate::emoji_picker::sticker_view(
            media,
            packs,
            stickers,
            |url, body, width, height| Message::StickerPicked {
                url: url.to_string(),
                body: body.to_string(),
                width,
                height,
            },
        ),
        PickerTab::Emoji => crate::emoji_picker::view(
            emoji_usage,
            media,
            packs,
            Message::EmojiPicked,
            |emoji| Message::CustomEmojiPicked {
                shortcode: emoji.shortcode.clone(),
                mxc_url: emoji.mxc_url.clone(),
            },
        ),
    };

    column![tabs, content].spacing(4).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_completed_word_while_typing() {
        // No trailing whitespace → the final word is still being typed.
        assert_eq!(last_completed_word("teh"), None);
        assert_eq!(last_completed_word(""), None);
        assert_eq!(last_completed_word("   "), None);
    }

    #[test]
    fn completed_word_is_the_last_before_trailing_space() {
        let (range, word, raw) = last_completed_word("teh ").unwrap();
        assert_eq!((range, word.as_str(), raw.as_str()), (0..3, "teh", "teh"));

        let (range, word, _) = last_completed_word("hello world ").unwrap();
        assert_eq!(&"hello world "[range.clone()], "world");
        assert_eq!((range, word.as_str()), (6..11, "world"));
    }

    #[test]
    fn surrounding_punctuation_is_trimmed_but_kept_in_raw() {
        // The replace range excludes the comma; the raw token keeps it so
        // skip heuristics still see the full token.
        let (range, word, raw) = last_completed_word("wat, ").unwrap();
        assert_eq!((range, word.as_str(), raw.as_str()), (0..3, "wat", "wat,"));
    }

    #[test]
    fn ranges_are_utf8_byte_offsets() {
        // 'é' is two bytes — the range must land on char boundaries.
        let body = "café ";
        let (range, word, _) = last_completed_word(body).unwrap();
        assert_eq!(range, 0..5);
        assert_eq!(word, "café");
        assert_eq!(&body[range], "café");
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

    #[test]
    fn trailing_boundary_is_a_single_appended_space() {
        assert!(typed_trailing_boundary("teh", "teh "));
        assert!(!typed_trailing_boundary("teh", "teh x")); // more than a space
        assert!(!typed_trailing_boundary("teh ", "teh")); // a deletion
        assert!(!typed_trailing_boundary("teh", "teh  ")); // two spaces (paste)
        assert!(!typed_trailing_boundary("teh", "xteh ")); // not an append
    }

    #[test]
    fn autocorrect_skips_leading_capital() {
        assert!(starts_lowercase("teh"));
        assert!(!starts_lowercase("Teh"));
        assert!(!starts_lowercase(""));
    }
}
