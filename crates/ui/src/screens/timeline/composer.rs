//! Message composer: @mention autocomplete, emoji picker (unicode + custom
//! emoji packs), and attachment staging — picked/pasted files wait as chips
//! above the input until Enter/Send, with any typed text riding out as the
//! first file's caption (MSC2530). This
//! module never talks to `client_core::sync`/`mpsc` directly — it only
//! produces `Effect`s, which the root dispatcher (`ui::update`) turns into
//! actual `ClientCommand` sends, generating and tracking the `request_id`
//! needed to correlate the eventual
//! `ClientEvent::CommandSucceeded`/`CommandFailed`.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use client_core::commands::RequestId;
use client_core::events::{EmojiPack, ReplyPreview, RoomMember};
use iced::advanced::text::editor::{Cursor, Position};
use iced::widget::text_editor::{self, Action, Binding, Edit, KeyPress};
use iced::widget::{button, column, container, row, text};
use iced::{Element, Length, Task};

use crate::spellcheck_config::SpellcheckConfig;
use crate::spellcheck_highlight::{is_checkable, words, Word};

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
    /// The draft as the editor holds it — the widget's own buffer, and the
    /// only thing with a caret.
    pub content: text_editor::Content,
    /// Mirror of `content.text()`, refreshed after every edit. Everything
    /// downstream (send, captions, the mention filter) reads the draft as a
    /// plain string, and rebuilding it once per edit is far cheaper than
    /// walking the editor's lines at each of those call sites.
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
    /// When the input's right-click edit menu (Cut/Copy/Paste/Select All) is
    /// showing, the window-global cursor point it opened at — the menu anchors
    /// there. `None` when closed. Rendered by `timeline::view` as a floating
    /// layer so it can sit at the pointer without resizing the composer.
    pub context_menu: Option<iced::Point>,
}

impl State {
    /// Puts a draft back in the composer, caret at the end — for when a send
    /// fails and the carried text has to be restored. Goes through the same
    /// rebuild every programmatic edit does, so the editor and the `body`
    /// mirror can't drift apart.
    pub fn restore_draft(&mut self, body: String) {
        set_body(self, body, None);
    }
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

/// How many word verdicts to remember before dropping the lot. The cache is
/// a latency trick, not a store — a long session in one composer shouldn't
/// grow it without bound.
const VERDICT_CACHE_CAP: usize = 512;

/// Spell-check state for the composer, recomputed on every edit. Holds only
/// the speller's plain-data verdicts so `view` never has to talk to COM.
#[derive(Debug, Clone, Default)]
pub struct SpellState {
    /// The flagged word the suggestion bar targets, or `None`.
    pub flagged: Option<Flagged>,
    /// Which words to draw in the danger colour inside the editor, handed
    /// straight to the highlighter. See [`crate::spellcheck_highlight`].
    pub highlight: crate::spellcheck_highlight::Settings,
    /// Set for exactly one edit after an autocorrect: if the next edit is the
    /// Backspace that would delete the space we just added, we restore the
    /// original word instead ("undo autocorrect", like a phone keyboard).
    pending_revert: Option<Revert>,
    /// Per-word memo of [`crate::spellcheck::is_misspelled`]. Every keystroke
    /// re-checks the whole draft, and the speller is a synchronous COM call —
    /// without this, a long draft would pay for all of it on every key.
    /// Keyed by the word (not its range — edits earlier in the body shift the
    /// range without changing the word).
    verdicts: HashMap<String, bool>,
    /// Memo of the bar's suggestion list, so parking the caret next to a typo
    /// doesn't re-run the speller's expensive `Suggest` on every keystroke.
    flag_memo: Option<(String, Vec<String>)>,
    /// Words the user has un-corrected with the Backspace that follows an
    /// autocorrect. Autocorrect leaves these alone for the rest of the draft.
    ///
    /// Without this, undoing achieves nothing: the word is still misspelled,
    /// so finishing it again re-applies the same fix, and the spelling the
    /// user actually wants can never survive a space. The suggestion bar is
    /// deliberately *not* gated on this — the word stays flagged and the
    /// correction stays one click away, it just stops happening by itself.
    rejected: HashSet<String>,
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
    /// Byte range the correction occupies in the body autocorrect left behind.
    range: Range<usize>,
    /// The word autocorrect put there — checked before undoing, so a body
    /// edited out from under us is never corrupted.
    corrected: String,
    /// The word the user actually typed, to put back.
    original: String,
}

impl Revert {
    /// The body and caret to restore when the Backspace that just ran was the
    /// "undo the autocorrect" one: the caret must have landed exactly where
    /// the boundary character used to be, with the correction still intact.
    fn undo(&self, body: &str, cursor: usize) -> Option<(String, usize)> {
        if cursor != self.range.end
            || body.get(self.range.clone()) != Some(self.corrected.as_str())
        {
            return None;
        }
        let mut restored = body.to_string();
        restored.replace_range(self.range.clone(), &self.original);
        Some((restored, self.range.start + self.original.len()))
    }
}

impl SpellState {
    /// Re-checks every word in `body` and refreshes both the in-editor marks
    /// and the suggestion bar. `cursor` is the caret's byte offset into
    /// `body`. Clears everything when spell check is turned off.
    fn recompute(&mut self, body: &str, cursor: usize, cfg: &SpellcheckConfig) {
        self.flagged = None;
        if !cfg.enabled {
            self.set_misspelled(HashSet::new());
            return;
        }

        // The word under the caret is still being typed; flagging it would
        // paint it red halfway through and unpaint it at the end. Words are
        // matched by text, so this necessarily spares an identical word
        // elsewhere in the draft too — until the next space brings it back.
        let in_progress = word_being_typed(body, cursor).map(|word| word.core.to_string());

        let mut misspelled = HashSet::new();
        for word in words(body) {
            if !is_checkable(word.raw)
                || in_progress.as_deref() == Some(word.core)
                || misspelled.contains(word.core)
            {
                continue;
            }
            if self.verdict(word.core) {
                misspelled.insert(word.core.to_string());
            }
        }
        self.set_misspelled(misspelled);

        // The bar targets the flagged word the caret is in or has just left,
        // so clicking into a red word offers its fixes.
        let Some(target) = flag_target(body, cursor) else {
            return;
        };
        if !self.highlight.misspelled.contains(target.core) {
            return;
        }
        let suggestions = match &self.flag_memo {
            Some((word, suggestions)) if word == target.core => suggestions.clone(),
            _ => {
                let suggestions = crate::spellcheck::analyze(target.core).suggestions;
                self.flag_memo = Some((target.core.to_string(), suggestions.clone()));
                suggestions
            }
        };
        if !suggestions.is_empty() {
            self.flagged = Some(Flagged {
                range: target.range,
                word: target.core.to_string(),
                suggestions,
            });
        }
    }

    /// Cached [`crate::spellcheck::is_misspelled`].
    fn verdict(&mut self, word: &str) -> bool {
        if let Some(known) = self.verdicts.get(word) {
            return *known;
        }
        if self.verdicts.len() >= VERDICT_CACHE_CAP {
            self.verdicts.clear();
        }
        let verdict = crate::spellcheck::is_misspelled(word);
        self.verdicts.insert(word.to_string(), verdict);
        verdict
    }

    /// Swaps in a new set of flagged words, bumping the revision only when it
    /// actually changed — that revision is the whole of the highlighter's
    /// equality check, and bumping it re-runs the highlighter over every line.
    fn set_misspelled(&mut self, misspelled: HashSet<String>) {
        if *self.highlight.misspelled == misspelled {
            return;
        }
        self.highlight.revision = self.highlight.revision.wrapping_add(1);
        self.highlight.misspelled = Arc::new(misspelled);
    }

    /// Drops every memoized verdict — the personal dictionary just changed,
    /// which can flip the answer for any word, not just the one added.
    fn forget_verdicts(&mut self) {
        self.verdicts.clear();
        self.flag_memo = None;
    }

    /// Clears everything for a fresh draft. The highlighter revision survives:
    /// it has to stay monotonic, or the widget could mistake the new empty
    /// state for the one it is already showing.
    fn reset(&mut self) {
        let revision = self.highlight.revision;
        *self = Self::default();
        self.highlight.revision = revision;
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    /// The editor performed an edit or a caret move. Carries the widget's own
    /// action rather than the resulting string: the caret is what lets
    /// autocorrect find the word you just finished anywhere in the draft.
    Action(Action),
    Send,
    ToggleEmojiPicker,
    ToggleStickerPicker,
    /// Dismiss the emoji/sticker picker (a click outside the floating panel).
    ClosePicker,
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

    /// Right-clicked the input — show the Cut/Copy/Paste/Select All menu.
    OpenContextMenu,
    /// Dismiss that menu (clicked off it, picked an item, or sent).
    CloseContextMenu,
    /// Edit-menu actions. Cut/Copy drive the input's own native handlers
    /// (see [`crate::synthetic_input`]); Paste and Select All are handled
    /// here / app-side (see the `update` arms).
    ContextCut,
    ContextCopy,
    ContextPaste,
    ContextSelectAll,
    /// Append text to the draft — used by the right-click Paste path, which
    /// reads the clipboard app-side (see [`Effect::PasteFromClipboard`])
    /// rather than leaning on the focused widget the way Ctrl+V does.
    InsertText(String),

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
    /// for unicode, the `mxc://` URL for custom emoji (matching how custom
    /// reactions are keyed).
    EmojiUsed(String),
    /// Right-clicked the input: the root dispatcher snapshots the window-
    /// global cursor into `State::context_menu` so the menu opens at the
    /// pointer (the composer can't see that coordinate itself).
    OpenContextMenu,
    /// Right-click Cut/Copy: drive the focused input's native clipboard
    /// handler by synthesizing the Ctrl chord (see [`crate::synthetic_input`]).
    ClipboardEdit(crate::synthetic_input::Edit),
    /// Right-click Paste: the root dispatcher reads the clipboard and either
    /// feeds the text back as [`Message::InsertText`] or stages its
    /// files/image as attachments (matching Ctrl+V).
    PasteFromClipboard,
}

pub fn update(
    state: &mut State,
    message: Message,
    spell: &SpellcheckConfig,
) -> (Task<Message>, Effect) {
    match message {
        Message::Action(action) => {
            // The editor reports caret moves, clicks, drags and scrolls
            // through here too — only a real edit should clear the error or
            // re-announce that we're typing.
            let is_edit = matches!(action, Action::Edit(_));
            let was_backspace = matches!(action, Action::Edit(Edit::Backspace));
            let finished_word = ends_word(&action);
            if is_edit {
                // A stale send/attach error shouldn't pin itself above the
                // composer once the user has moved on.
                state.error = None;
            }
            // Typing dismisses the edit menu (its backdrop only swallows mouse
            // events, so the focused editor still receives keystrokes).
            state.context_menu = None;

            // A revert is good for exactly one edit — whatever that edit
            // turns out to be.
            let revert = state.spell.pending_revert.take();
            state.content.perform(action);
            state.body = state.content.text();
            let typing = if is_edit {
                Effect::Typing(!state.body.trim().is_empty())
            } else {
                Effect::None
            };

            // Backspace immediately after an autocorrect undoes it (restores
            // the original word) instead of just deleting the space.
            if was_backspace {
                if let Some(revert) = revert {
                    let undone = revert.undo(&state.body, cursor_offset(&state.content));
                    if let Some((body, caret)) = undone {
                        set_body(state, body, Some(caret));
                        // This Backspace is the user saying no to the fix, so
                        // stop offering to make it for them.
                        state.spell.rejected.insert(revert.original);
                        recompute_spell(state, spell);
                        return (Task::none(), typing);
                    }
                }
            }

            // Autocorrect fires on the edit that finishes a word — a space or
            // a newline. The caret says which word that was, so it works
            // mid-line and not only at the end of the draft.
            if spell.autocorrect && finished_word {
                maybe_autocorrect(state);
            }

            recompute_spell(state, spell);
            (Task::none(), typing)
        }
        Message::Send => {
            // Enter can fire with the edit menu still up (its backdrop blocks
            // only mouse); don't leave it floating over a sent message.
            state.context_menu = None;
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
        Message::ClosePicker => {
            state.show_emoji_picker = false;
            (Task::none(), Effect::None)
        }
        Message::StickerPicked { url, body, width, height } => {
            // Fire-and-forget, like a reaction: the picker stays open so a
            // few stickers can go out in a row.
            (Task::none(), Effect::SendSticker { url, body, width, height })
        }
        Message::EmojiPicked(glyph) => {
            insert_at_caret(state, glyph);
            // The body changed by insertion, not by the undo-trigger
            // Backspace — a stale revert would misfire on a later deletion
            // and rewrite text the user didn't ask to restore.
            state.spell.pending_revert = None;
            recompute_spell(state, spell);
            (Task::none(), Effect::EmojiUsed(glyph.to_string()))
        }
        Message::CustomEmojiPicked { shortcode, mxc_url } => {
            insert_at_caret(state, &format!(":{shortcode}: "));
            state.spell.pending_revert = None;
            recompute_spell(state, spell);
            // Record usage by the mxc URL — the same key custom reactions use,
            // so an emoji's frequency is one tally across both and the
            // "Frequently used" row shows it once.
            (Task::none(), Effect::EmojiUsed(mxc_url))
        }
        Message::MentionCandidateClicked(user_id, display_name) => {
            // Rebuilt through `set_body` rather than poked into `state.body`:
            // the editor owns the text now, and a body it doesn't know about
            // would be overwritten by the next keystroke.
            let mut body = state.body.clone();
            if let Some(at_pos) = body.rfind('@') {
                body.truncate(at_pos);
            }
            body.push('@');
            body.push_str(&display_name);
            body.push(' ');
            // Caret to the end — completion only ever rewrites the trailing
            // word (see `active_mention_query`).
            set_body(state, body, None);
            if !state.mentioned.iter().any(|(id, _)| *id == user_id) {
                state.mentioned.push((user_id, display_name));
            }
            state.spell.pending_revert = None;
            // A just-picked mention is never a typo — don't spell-flag the
            // tail of a multi-word display name ("@John Smyth" → "Smyth"
            // would pop "Did you mean: Smith", and clicking it would corrupt
            // the mention text). Any later edit recomputes via `Action`.
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
                    let caret = flagged.range.start + replacement.len();
                    let mut body = state.body.clone();
                    body.replace_range(flagged.range, &replacement);
                    set_body(state, body, Some(caret));
                }
            }
            state.spell.pending_revert = None;
            recompute_spell(state, spell);
            (Task::none(), Effect::None)
        }
        Message::SpellAddToDictionary => {
            if let Some(flagged) = state.spell.flagged.take() {
                crate::spellcheck::add_to_dictionary(&flagged.word);
            }
            // The dictionary just changed — the memoized verdict for this
            // word is stale (it would keep flagging the word just added).
            state.spell.forget_verdicts();
            recompute_spell(state, spell);
            (Task::none(), Effect::None)
        }
        Message::OpenContextMenu => {
            // The window-global cursor lives in `App`; the root dispatcher
            // fills `state.context_menu` with it (see `Effect::OpenContextMenu`).
            (Task::none(), Effect::OpenContextMenu)
        }
        Message::CloseContextMenu => {
            state.context_menu = None;
            (Task::none(), Effect::None)
        }
        Message::ContextCopy => {
            state.context_menu = None;
            (Task::none(), Effect::ClipboardEdit(crate::synthetic_input::Edit::Copy))
        }
        Message::ContextCut => {
            state.context_menu = None;
            (Task::none(), Effect::ClipboardEdit(crate::synthetic_input::Edit::Cut))
        }
        Message::ContextPaste => {
            state.context_menu = None;
            (Task::none(), Effect::PasteFromClipboard)
        }
        Message::ContextSelectAll => {
            state.context_menu = None;
            // Focus *then* select: `focus()` snaps the caret to the end, so
            // selecting must come second or it'd be collapsed. Focusing first
            // also makes the selection visible and gives a follow-up Copy a
            // focused target even if the input wasn't focused before.
            (
                iced::widget::operation::focus(input_id())
                    .chain(iced::widget::operation::select_all(input_id())),
                Effect::None,
            )
        }
        Message::InsertText(text) => {
            insert_at_caret(state, &text);
            // A paste isn't the autocorrect-undo Backspace — drop any pending
            // revert so a later deletion doesn't misfire (as with emoji).
            state.spell.pending_revert = None;
            state.error = None;
            recompute_spell(state, spell);
            let typing = Effect::Typing(!state.body.trim().is_empty());
            // Focus so the caret lands after the pasted text, ready to keep
            // typing without an extra click.
            (iced::widget::operation::focus(input_id()), typing)
        }
        Message::SendSucceeded => {
            set_body(state, String::new(), None);
            state.mentioned.clear();
            state.replying_to = None;
            state.pending_request = None;
            state.error = None;
            state.spell.reset();
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

/// The caret as an absolute byte offset into `State::body`.
///
/// `Content` reports the caret as a line/column pair, but everything the
/// spell checker does is expressed in byte ranges over the flat draft, so
/// the two have to be converted at every boundary.
fn cursor_offset(content: &text_editor::Content) -> usize {
    let cursor = content.cursor();
    let mut offset = 0;
    for (index, line) in content.lines().enumerate() {
        if index == cursor.position.line {
            return offset + cursor.position.column.min(line.text.len());
        }
        offset += line.text.len() + line_ending_len(line.ending);
    }
    offset
}

/// The inverse of [`cursor_offset`]: an absolute byte offset expressed as the
/// line/column the editor can be moved to. Offsets past the end clamp to it.
fn position_at(content: &text_editor::Content, offset: usize) -> Cursor {
    let mut consumed = 0;
    let mut last = Position { line: 0, column: 0 };
    for (index, line) in content.lines().enumerate() {
        let end = consumed + line.text.len();
        if offset <= end {
            return Cursor {
                position: Position { line: index, column: offset - consumed },
                selection: None,
            };
        }
        consumed = end + line_ending_len(line.ending);
        last = Position { line: index, column: line.text.len() };
    }
    Cursor { position: last, selection: None }
}

/// How many bytes `Content::text()` writes for a line ending. `None` means the
/// line has no ending of its own — `text()` falls back to the platform default
/// there, so the accounting has to as well.
fn line_ending_len(ending: text_editor::LineEnding) -> usize {
    if ending == text_editor::LineEnding::None {
        text_editor::LineEnding::default().as_str().len()
    } else {
        ending.as_str().len()
    }
}

/// Replaces the draft wholesale and puts the caret at `caret` (or the end).
///
/// This is the path every *programmatic* edit takes — emoji, mentions, paste,
/// autocorrect, clearing on send. `Content` has no "replace this range"
/// operation, so it is rebuilt and the caret restored by hand; that's O(draft),
/// which for a chat message is nothing.
fn set_body(state: &mut State, body: String, caret: Option<usize>) {
    state.body = body;
    state.content = text_editor::Content::with_text(&state.body);
    let caret = caret.unwrap_or(state.body.len());
    let position = position_at(&state.content, caret);
    state.content.move_to(position);
}

/// Inserts `text` at the caret, leaving the caret just after it.
fn insert_at_caret(state: &mut State, text: &str) {
    let at = cursor_offset(&state.content);
    let mut body = state.body.clone();
    body.insert_str(at, text);
    set_body(state, body, Some(at + text.len()));
}

/// Re-runs the spell check over the whole draft against the current caret.
fn recompute_spell(state: &mut State, cfg: &SpellcheckConfig) {
    let cursor = cursor_offset(&state.content);
    state.spell.recompute(&state.body, cursor, cfg);
}

/// Whether an action finishes a word — the moment autocorrect gets to act.
/// A paste isn't one of them: it can drop in any amount of text, and silently
/// rewriting part of what someone pasted is not a fix anyone asked for.
fn ends_word(action: &Action) -> bool {
    match action {
        Action::Edit(Edit::Insert(c)) => c.is_whitespace(),
        Action::Edit(Edit::Enter) => true,
        _ => false,
    }
}

/// The word the caret is at the trailing edge of — the one being typed right
/// now. Deliberately *not* "the word the caret is inside": clicking into the
/// middle of a finished typo has to leave it marked, or the suggestion bar
/// would empty out at the exact moment you reached for it.
fn word_being_typed(body: &str, cursor: usize) -> Option<Word<'_>> {
    words(body).find(|word| word.raw_range.end == cursor)
}

/// The word the caret has just finished: the last one that ends *before* it.
/// A caret at a word's trailing edge means it is still being typed, so nothing
/// is finished there.
fn word_before_cursor(body: &str, cursor: usize) -> Option<Word<'_>> {
    words(body).take_while(|word| word.raw_range.end < cursor).last()
}

/// The word the suggestion bar should offer fixes for: the one the caret is
/// in, or — when the caret sits on whitespace — the one it just left.
fn flag_target(body: &str, cursor: usize) -> Option<Word<'_>> {
    words(body).take_while(|word| word.raw_range.start <= cursor).last()
}

/// Corrects the word the caret just finished, if the speller offers a
/// plausible fix, and records how to undo it on the next Backspace.
///
/// Called on the edit that ends a word, so the target is the token immediately
/// left of the caret — which is what makes this work mid-line rather than only
/// at the very end of the draft.
fn maybe_autocorrect(state: &mut State) {
    let cursor = cursor_offset(&state.content);
    // Everything the correction needs is copied out here: applying it borrows
    // `state` mutably, which ends the borrow the word itself holds on `body`.
    let Some((range, original)) = word_before_cursor(&state.body, cursor).and_then(|word| {
        // Don't silently rewrite mentions/URLs/code, and leave leading-capital
        // words (names, sentence starts) alone — the suggestion bar still
        // offers those, but autocorrect shouldn't touch them.
        (is_checkable(word.raw) && starts_lowercase(word.core))
            .then(|| (word.range.clone(), word.core.to_string()))
    }) else {
        return;
    };
    // Already rejected once in this draft — leave it alone (see
    // `SpellState::rejected`). Checked before the speller, so a word the user
    // has settled doesn't pay for a COM call on every space either.
    if state.spell.rejected.contains(&original) {
        return;
    }
    let Some(correction) = crate::spellcheck::top_correction(&original) else {
        return;
    };

    // Only the word is swapped — the boundary character the user just typed,
    // and anything after it, stays put, so the caret keeps its distance from
    // the end of the word.
    let caret = cursor - original.len() + correction.len();
    let corrected = range.start..(range.start + correction.len());
    let mut body = state.body.clone();
    body.replace_range(range, &correction);
    set_body(state, body, Some(caret));
    state.spell.pending_revert =
        Some(Revert { range: corrected, corrected: correction, original });
}

/// Autocorrect only rewrites words that start lowercase — a leading capital
/// usually marks a name or sentence-start proper noun we shouldn't touch.
fn starts_lowercase(word: &str) -> bool {
    word.chars().next().is_some_and(|c| c.is_lowercase())
}


/// Padding inside the composer's editor, matching what the old single-line
/// input used.
const INPUT_PADDING: u16 = 6;

/// How tall the editor is allowed to grow before it starts scrolling. Five
/// lines is enough for a paragraph without the composer eating the timeline.
const MAX_INPUT_LINES: f32 = 5.0;

/// One line of composer text, in pixels: iced's default 16px text at its
/// default 1.3 line height.
const INPUT_LINE_HEIGHT: f32 = 20.8;

/// The editor's total height for `lines` lines of text — `min_height` and
/// `max_height` bound the whole widget, padding included.
fn input_height(lines: f32) -> f32 {
    INPUT_LINE_HEIGHT * lines + f32::from(INPUT_PADDING) * 2.0
}

/// Enter sends; Shift+Enter breaks the line. Everything else keeps iced's
/// defaults — including the Ctrl+C/X/V/A that the right-click edit menu
/// synthesises as real key chords.
fn send_on_enter(press: KeyPress) -> Option<Binding<Message>> {
    let enter = iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter);
    if press.key == enter && !press.modifiers.shift() {
        return Some(Binding::Custom(Message::Send));
    }
    Binding::from_key_press(press)
}

/// Stable widget id for the composer's text input — lets the root dispatcher
/// refocus it after staging a pasted/picked attachment, so "paste → type a
/// caption → Enter" flows without an extra click.
pub fn input_id() -> iced::widget::Id {
    iced::widget::Id::from("composer-input")
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
            .push(crate::theme::remote_text(format!("Replying to {}", reply.sender)).size(12).style(text::primary));
        if let Some(thumb) = reply
            .image_url
            .as_deref()
            .and_then(|url| crate::media_cache::mxc_visual(media, url, 28, Some(28)))
        {
            banner = banner.push(thumb);
        }
        banner = banner
            .push(crate::theme::remote_text(reply.snippet.clone()).size(12).style(text::secondary).width(Length::Fill))
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
                button(crate::theme::remote_text(member.display_name.clone()).size(13))
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
                button(crate::theme::remote_text(suggestion.clone()).size(13))
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
                .push(crate::theme::remote_text(label).size(12))
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
    // Wrapped in a mouse_area only to catch the right-click that opens the
    // edit menu. mouse_area delegates to its child first and bails when the
    // child captures, so the editor's own left-click caret placement and
    // drag-select are untouched (it ignores right-clicks, which is exactly
    // what lets them fall through to `on_right_press`).
    let input: Element<'_, Message> = iced::widget::mouse_area(
        iced::widget::text_editor(&state.content)
            .id(input_id())
            .placeholder(placeholder)
            .on_action(Message::Action)
            .padding(INPUT_PADDING)
            // Width is already `Length::Fill` by default, and the builder only
            // accepts a fixed pixel width — so it's left alone here.
            .min_height(input_height(1.0))
            .max_height(input_height(MAX_INPUT_LINES))
            .wrapping(text::Wrapping::Word)
            .key_binding(send_on_enter)
            // Always attached — it changes the widget's type, so it can't be
            // added conditionally. Spell check being off just means the set of
            // words to mark is empty (see `SpellState::recompute`).
            .highlight_with::<crate::spellcheck_highlight::Highlighter>(
                state.spell.highlight.clone(),
                crate::spellcheck_highlight::format,
            ),
    )
    .on_right_press(Message::OpenContextMenu)
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

/// The input's right-click edit menu (Cut / Copy / Paste / Select all), opened
/// at `anchor` — the window-global cursor point captured on right-click.
///
/// `timeline::view` renders this as a layer in the *outer* stack — it has to
/// live there, above the whole shell, because only a `stack` short-circuits
/// event dispatch on capture. That's what stops the click that picks an item
/// (or dismisses the menu) from also reaching and unfocusing the input; an
/// unfocused input would drop the synthesized Cut/Copy chord on the floor.
///
/// It opens *upward* from the pointer (the menu's bottom edge sits at the
/// cursor): the composer is pinned to the window's bottom edge, so a
/// downward menu would spill off-screen. `MENU_HEIGHT` is the panel's
/// measured height, used only to place that bottom edge at the cursor.
pub fn context_menu<'a>(anchor: iced::Point) -> Element<'a, Message> {
    // 4 items (~28px each) + inter-item spacing + panel padding/border.
    const MENU_HEIGHT: f32 = 130.0;
    const MENU_WIDTH: f32 = 150.0;

    let item = |label: &'a str, message: Message| {
        button(text(label).size(13))
            .on_press(message)
            .style(crate::theme::ghost_button)
            .width(Length::Fill)
            .padding([5, 10])
    };
    let menu = container(
        column![
            item("Cut", Message::ContextCut),
            item("Copy", Message::ContextCopy),
            item("Paste", Message::ContextPaste),
            item("Select all", Message::ContextSelectAll),
        ]
        .spacing(2),
    )
    .width(Length::Fixed(MENU_WIDTH))
    .padding(4)
    .style(crate::theme::floating_panel);

    // Anchor at the pointer. iced has no absolute positioning, so a full-size
    // container places the menu with top/left padding: `top` lifts it so its
    // bottom lands on the cursor (open upward), `left` puts its left edge
    // there. Clamped to the top-left so it never pushes off those edges.
    let top = (anchor.y - MENU_HEIGHT).max(0.0);
    let left = anchor.x.max(0.0);
    let positioned = container(iced::widget::opaque(menu))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced::Left)
        .align_y(iced::Top)
        .padding(iced::Padding { top, right: 0.0, bottom: 0.0, left });

    // Backdrop: a click off the (opaque) menu dismisses it, and wrapping the
    // whole layer in `opaque` keeps every click on it from falling through to
    // unfocus the input below — which is what lets Cut/Copy still see the live
    // selection. Same shape as the emoji picker's dismiss backdrop.
    iced::widget::opaque(
        iced::widget::mouse_area(positioned).on_press(Message::CloseContextMenu),
    )
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
    fn autocorrect_skips_leading_capital() {
        assert!(starts_lowercase("teh"));
        assert!(!starts_lowercase("Teh"));
        assert!(!starts_lowercase(""));
    }

    #[test]
    fn the_finished_word_is_the_one_left_of_the_caret() {
        // "teh| cat" — the space was just typed at offset 3.
        let word = word_before_cursor("teh cat", 4).unwrap();
        assert_eq!((word.range, word.core), (0..3, "teh"));

        // Nothing is finished until the caret has passed a boundary.
        assert!(word_before_cursor("teh", 3).is_none());
        assert!(word_before_cursor("", 0).is_none());
    }

    #[test]
    fn a_word_finished_mid_line_is_found_too() {
        // "one teh| two" — this is what the old end-of-draft-only trigger
        // could never see.
        let word = word_before_cursor("one teh two", 8).unwrap();
        assert_eq!((word.range, word.core), (4..7, "teh"));
    }

    #[test]
    fn only_the_caret_s_trailing_edge_counts_as_still_typing() {
        // Typing left to right, the caret sits at the end of the word.
        assert_eq!(word_being_typed("recieve", 7).unwrap().core, "recieve");
        // Clicked into the middle of it — that word is finished, and stays
        // marked so the bar has something to offer.
        assert!(word_being_typed("recieve", 3).is_none());
        // On whitespace, nothing is in progress.
        assert!(word_being_typed("recieve ", 8).is_none());
    }

    #[test]
    fn the_bar_targets_the_word_the_caret_just_left() {
        // Caret inside a word: that word.
        assert_eq!(flag_target("one teh two", 6).unwrap().core, "teh");
        // Caret on the space after it (offset 7): still that word.
        assert_eq!(flag_target("one teh two", 7).unwrap().core, "teh");
        // Caret at the leading edge of the next word: that word instead.
        assert_eq!(flag_target("one teh two", 8).unwrap().core, "two");
        assert!(flag_target("   ", 3).is_none());
    }

    #[test]
    fn undo_restores_the_typed_word_only_at_the_right_caret() {
        let revert = Revert {
            range: 0..3,
            corrected: "the".to_string(),
            original: "teh".to_string(),
        };
        // Backspace ate the space, leaving the caret at the word's end.
        assert_eq!(revert.undo("the", 3), Some(("teh".to_string(), 3)));
        // Caret somewhere else — this Backspace wasn't the undo.
        assert_eq!(revert.undo("the cat", 7), None);
        // The correction is gone, so there is nothing to put back.
        assert_eq!(revert.undo("th", 3), None);
    }

    #[test]
    fn only_a_boundary_keystroke_triggers_autocorrect() {
        assert!(ends_word(&Action::Edit(Edit::Insert(' '))));
        assert!(ends_word(&Action::Edit(Edit::Enter)));
        assert!(!ends_word(&Action::Edit(Edit::Insert('x'))));
        assert!(!ends_word(&Action::Edit(Edit::Backspace)));
        // A paste can drop in any amount of text — never a correction cue.
        assert!(!ends_word(&Action::Edit(Edit::Paste(Arc::new(
            "teh ".to_string()
        )))));
        assert!(!ends_word(&Action::SelectAll));
    }

    #[test]
    fn the_caret_survives_a_programmatic_rewrite() {
        let mut state = State::default();
        set_body(&mut state, "hello world".to_string(), Some(5));
        assert_eq!(cursor_offset(&state.content), 5);

        // Past the end clamps rather than panicking.
        set_body(&mut state, "hi".to_string(), Some(99));
        assert_eq!(cursor_offset(&state.content), 2);

        // No caret given means "put it at the end", which is what every
        // append-shaped edit wants.
        set_body(&mut state, "abc".to_string(), None);
        assert_eq!(cursor_offset(&state.content), 3);
    }

    #[test]
    fn the_caret_offset_accounts_for_earlier_lines() {
        let mut state = State::default();
        // Offset 8 is the 'i' of "line2": 5 + 1 newline + 2.
        set_body(&mut state, "line1
line2".to_string(), Some(8));
        assert_eq!(cursor_offset(&state.content), 8);

        // ...and the round trip back through `position_at` lands there.
        let body = state.body.clone();
        set_body(&mut state, body, Some(8));
        assert_eq!(cursor_offset(&state.content), 8);
    }

    /// Spell check is exercised through `SpellState` directly: body and caret
    /// go in as plain data, so these run without a live speller — the seeded
    /// `verdicts`/`flag_memo` stand in for one.
    const ON: SpellcheckConfig = SpellcheckConfig { enabled: true, autocorrect: false };
    const OFF: SpellcheckConfig = SpellcheckConfig { enabled: false, autocorrect: false };

    /// A `SpellState` that already "knows" `teh` is wrong and what to offer
    /// for it, so `recompute` never reaches the COM speller.
    fn seeded() -> SpellState {
        let mut spell = SpellState::default();
        spell.verdicts.insert("teh".to_string(), true);
        spell.flag_memo =
            Some(("teh".to_string(), vec!["the".to_string(), "ten".to_string()]));
        spell
    }

    #[test]
    fn turning_spell_check_off_clears_every_mark() {
        let mut spell = seeded();
        spell.recompute("teh ", 4, &ON);
        assert!(spell.highlight.misspelled.contains("teh"));

        spell.recompute("teh ", 4, &OFF);
        assert!(spell.highlight.misspelled.is_empty());
        assert!(spell.flagged.is_none());
    }

    #[test]
    fn chat_tokens_are_never_checked() {
        // These would all be flagged by a dictionary; none of them is a typo.
        let mut spell = SpellState::default();
        for body in [
            "@alice:example.org ",
            "https://example.org/page ",
            ":shrug: ",
            "ACRONYM ",
        ] {
            spell.recompute(body, body.len(), &ON);
            assert!(spell.highlight.misspelled.is_empty(), "{body:?} was checked");
        }
    }

    #[test]
    fn the_word_still_being_typed_is_not_marked_yet() {
        let mut spell = seeded();
        // Caret at the trailing edge: "teh" is mid-word, so it stays unpainted
        // rather than flashing red halfway through.
        spell.recompute("teh", 3, &ON);
        assert!(spell.highlight.misspelled.is_empty());
        assert!(spell.flagged.is_none());

        // The space finishes it, and now it is marked.
        spell.recompute("teh ", 4, &ON);
        assert!(spell.highlight.misspelled.contains("teh"));
    }

    #[test]
    fn the_bar_targets_the_flagged_word_at_the_caret() {
        let mut spell = seeded();
        spell.recompute("teh ", 4, &ON);
        let flagged = spell.flagged.clone().expect("the bar should target \"teh\"");
        assert_eq!((flagged.range, flagged.word.as_str()), (0..3, "teh"));
        assert_eq!(flagged.suggestions, ["the", "ten"]);
    }

    #[test]
    fn the_highlighter_revision_only_moves_when_the_marks_do() {
        let mut spell = seeded();
        spell.recompute("teh ", 4, &ON);
        let revision = spell.highlight.revision;
        // Same marks, so the highlighter must not be asked to re-run.
        spell.recompute("teh ", 4, &ON);
        assert_eq!(spell.highlight.revision, revision);

        // A real change bumps it.
        spell.recompute("", 0, &ON);
        assert_ne!(spell.highlight.revision, revision);
    }

    #[test]
    fn a_reset_draft_keeps_the_revision_monotonic() {
        let mut spell = seeded();
        spell.recompute("teh ", 4, &ON);
        let revision = spell.highlight.revision;
        spell.reset();
        assert!(spell.flagged.is_none());
        assert!(spell.highlight.misspelled.is_empty());
        // A rewound revision could be mistaken for state already on screen.
        assert_eq!(spell.highlight.revision, revision);
    }

    #[test]
    fn a_space_autocorrects_the_word_just_finished_and_backspace_undoes_it() {
        const AUTO: SpellcheckConfig =
            SpellcheckConfig { enabled: true, autocorrect: true };
        // Drives the real update path, so it needs the OS speller; where there
        // is none, `top_correction` yields nothing and there is nothing to
        // assert (see [`crate::spellcheck`]).
        let Some(expected) = crate::spellcheck::top_correction("teh") else {
            return;
        };

        let mut state = State::default();
        for c in "teh".chars() {
            let _ = update(&mut state, Message::Action(Action::Edit(Edit::Insert(c))), &AUTO);
        }
        // Mid-word: nothing has been rewritten yet.
        assert_eq!(state.body, "teh");

        let _ = update(&mut state, Message::Action(Action::Edit(Edit::Insert(' '))), &AUTO);
        assert_eq!(state.body, format!("{expected} "));
        // The boundary character stays put and the caret stays after it.
        assert_eq!(cursor_offset(&state.content), expected.len() + 1);

        // The very next Backspace undoes the correction instead of just
        // eating the space.
        let _ = update(&mut state, Message::Action(Action::Edit(Edit::Backspace)), &AUTO);
        assert_eq!(state.body, "teh");
    }

    /// Types `body` one character at a time through the real update path and
    /// returns the draft it leaves behind.
    fn typed(body: &str, cfg: &SpellcheckConfig) -> String {
        let mut state = State::default();
        for c in body.chars() {
            let _ = update(&mut state, Message::Action(Action::Edit(Edit::Insert(c))), cfg);
        }
        state.body
    }

    #[test]
    fn a_typo_the_speller_only_guesses_at_is_still_corrected() {
        const AUTO: SpellcheckConfig =
            SpellcheckConfig { enabled: true, autocorrect: true };
        // "disappointet" gets no CORRECTIVE_ACTION_REPLACE — just two guesses
        // one edit away, "disappointed" and "disappointer". Needs the OS
        // speller, so it degrades to a no-op where there isn't one.
        let Some(expected) = crate::spellcheck::top_correction("disappointet") else {
            return;
        };
        assert_eq!(expected, "disappointed");
        assert_eq!(typed("disappointet ", &AUTO), "disappointed ");
    }

    #[test]
    fn chat_slang_is_left_alone_by_both_marking_and_autocorrect() {
        const AUTO: SpellcheckConfig =
            SpellcheckConfig { enabled: true, autocorrect: true };
        // The speller offers "goanna" for "gonna" and "urn" for "ur"; neither
        // word should be rewritten or marked.
        assert_eq!(typed("gonna ", &AUTO), "gonna ");
        assert_eq!(typed("ur ", &AUTO), "ur ");

        let mut state = State::default();
        for c in "gonna ".chars() {
            let _ = update(&mut state, Message::Action(Action::Edit(Edit::Insert(c))), &AUTO);
        }
        assert!(state.spell.highlight.misspelled.is_empty());
    }

    #[test]
    fn an_undone_correction_is_not_made_again() {
        const AUTO: SpellcheckConfig =
            SpellcheckConfig { enabled: true, autocorrect: true };
        let Some(expected) = crate::spellcheck::top_correction("teh") else {
            return;
        };

        let mut state = State::default();
        let mut key = |state: &mut State, action: Action| {
            let _ = update(state, Message::Action(action), &AUTO);
        };
        for c in "teh ".chars() {
            key(&mut state, Action::Edit(Edit::Insert(c)));
        }
        assert_eq!(state.body, format!("{expected} "));

        // Backspace takes the correction back...
        key(&mut state, Action::Edit(Edit::Backspace));
        assert_eq!(state.body, "teh");

        // ...and finishing the word again must leave it alone. Before this,
        // the space re-applied the same fix and the typed spelling could
        // never be kept.
        key(&mut state, Action::Edit(Edit::Insert(' ')));
        assert_eq!(state.body, "teh ");

        // Still flagged, though — the bar keeps offering what autocorrect has
        // stopped doing on its own.
        assert!(state.spell.highlight.misspelled.contains("teh"));

        // And it stays rejected for the rest of the draft, not just once.
        for c in "teh ".chars() {
            key(&mut state, Action::Edit(Edit::Insert(c)));
        }
        assert_eq!(state.body, "teh teh ");
    }

    #[test]
    fn a_rejection_does_not_outlive_the_draft() {
        const AUTO: SpellcheckConfig =
            SpellcheckConfig { enabled: true, autocorrect: true };
        let Some(expected) = crate::spellcheck::top_correction("teh") else {
            return;
        };

        let mut state = State::default();
        for c in "teh ".chars() {
            let _ = update(&mut state, Message::Action(Action::Edit(Edit::Insert(c))), &AUTO);
        }
        let _ = update(&mut state, Message::Action(Action::Edit(Edit::Backspace)), &AUTO);
        assert_eq!(state.body, "teh");

        // Sending clears the draft, and with it the rejection: the next
        // message starts from the same defaults as the first.
        let _ = update(&mut state, Message::SendSucceeded, &AUTO);
        for c in "teh ".chars() {
            let _ = update(&mut state, Message::Action(Action::Edit(Edit::Insert(c))), &AUTO);
        }
        assert_eq!(state.body, format!("{expected} "));
    }

    #[test]
    fn with_autocorrect_off_a_space_only_flags() {
        let mut state = State::default();
        for c in "teh".chars() {
            let _ = update(&mut state, Message::Action(Action::Edit(Edit::Insert(c))), &ON);
        }
        let _ = update(&mut state, Message::Action(Action::Edit(Edit::Insert(' '))), &ON);
        // Checking is on, so the word is marked — but never rewritten.
        assert_eq!(state.body, "teh ");
    }
}
