//! Message timeline for the currently open room: rendering (with real
//! inline images and custom emoji), the composer, reactions, inline
//! edit/delete on your own messages, thread reply badges, read-receipt
//! counts, and a typing indicator.

pub mod composer;
pub mod reactions;
pub mod threads;

use std::collections::HashMap;

use client_core::commands::RequestId;
use client_core::events::{
    EmojiPack, NotificationMode, RoomMember, RoomSummary, TimelineItem, TimelineItemContent,
    TrustShield, UrlPreview,
};
use iced::widget::{
    button, column, container, image, row, scrollable, text, text_input, tooltip,
};
use iced::{Element, Length};


/// The room header's notification dropdown options. `Default` means "no
/// per-room override" (follow the account default), which the SDK models as
/// the absence of a rule rather than a fourth mode — hence a UI-side enum
/// rather than reusing `NotificationMode` directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyChoice {
    Default,
    All,
    Mentions,
    Mute,
}

pub const NOTIFY_CHOICES: [NotifyChoice; 4] =
    [NotifyChoice::Default, NotifyChoice::All, NotifyChoice::Mentions, NotifyChoice::Mute];

impl std::fmt::Display for NotifyChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            NotifyChoice::Default => "Default",
            NotifyChoice::All => "All messages",
            NotifyChoice::Mentions => "Mentions only",
            NotifyChoice::Mute => "Mute",
        })
    }
}

impl NotifyChoice {
    fn from_mode(mode: Option<NotificationMode>) -> Self {
        match mode {
            None => NotifyChoice::Default,
            Some(NotificationMode::AllMessages) => NotifyChoice::All,
            Some(NotificationMode::MentionsAndKeywordsOnly) => NotifyChoice::Mentions,
            Some(NotificationMode::Mute) => NotifyChoice::Mute,
        }
    }

    fn to_mode(self) -> Option<NotificationMode> {
        match self {
            NotifyChoice::Default => None,
            NotifyChoice::All => Some(NotificationMode::AllMessages),
            NotifyChoice::Mentions => Some(NotificationMode::MentionsAndKeywordsOnly),
            NotifyChoice::Mute => Some(NotificationMode::Mute),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct State {
    pub room_id: Option<String>,
    pub items: Vec<TimelineItem>,
    pub composer: composer::State,
    /// user_id → index into `composer.member_candidates`, rebuilt alongside
    /// it (see `RoomMembersUpdated`) — O(1) display-name/avatar lookups for
    /// reaction tooltips, the typing line and the follower row, which
    /// otherwise each linear-scanned the whole roster per lookup per frame.
    pub member_index: std::collections::HashMap<String, usize>,
    /// Item indices matching the active search, recomputed on query edits
    /// and timeline snapshots (`recompute_search_matches`) — view() only
    /// tests membership, instead of lowercasing every message body and
    /// sender name per item per frame for the whole life of a search.
    pub search_matches: std::collections::HashSet<usize>,
    /// First URL of each item's text body, index-parallel to `items` and
    /// set in the same place (`TimelineUpdated` in update.rs) — view()
    /// reads it instead of re-running a linkify scan + String allocation
    /// per URL-bearing message per rebuild.
    pub first_urls: Vec<Option<String>>,
    pub editing: Option<EditingState>,
    pub confirm_delete: Option<String>,
    pub pending_edit_request: Option<RequestId>,
    pub pending_redact_request: Option<RequestId>,
    pub typing_users: Vec<String>,
    pub action_error: Option<String>,
    /// Event id of the message whose reaction picker is open, if any.
    pub reacting_to: Option<String>,
    pub search_open: bool,
    pub search_query: String,
    /// Whether the room notification-mode menu (opened from the header bell)
    /// is showing.
    pub notify_menu_open: bool,
    pub reached_start: bool,
    pub loading_older: bool,
    pub pending_paginate_request: Option<RequestId>,
    /// Member panel visibility, inverted so `Default` (false) means shown.
    pub hide_members: bool,
    /// MSC3949 member groups for the open room ("Red team", ...), highest
    /// power level first; empty = fall back to Admin/Mod/Member.
    pub power_tags: Vec<client_core::events::PowerLevelTag>,
    /// Member left-clicked in the roster — highlighted in the list only, no
    /// side effect. Cleared on room switch.
    pub selected_member: Option<String>,
    /// Member whose messages are tinted in the timeline ("highlight in chat",
    /// toggled from the member context menu). Cleared on room switch.
    pub highlighted_member: Option<String>,
    /// User id whose right-click actions menu is open. `None` = closed. The
    /// menu renders as a flyout to the left of the roster, anchored at
    /// `member_menu_anchor_y`.
    pub member_menu: Option<String>,
    /// Y (in roster-local coordinates, which line up with the timeline stack)
    /// where the open member flyout is pinned — frozen from `member_cursor`
    /// at right-click time so the menu doesn't chase the mouse.
    pub member_menu_anchor_y: f32,
    /// Live cursor position while hovering the roster (roster-local), used to
    /// anchor the flyout at the right-clicked row's height.
    pub member_cursor: Option<iced::Point>,
    /// Last roster left-click (user id + when), used to detect a double-click
    /// on the same member — which opens an unencrypted DM instead of just
    /// toggling the highlight.
    pub last_member_click: Option<(String, std::time::Instant)>,
    /// Whether the view is scrolled to (or near) the newest message. This is
    /// the sole gate for read receipts: while it's true the room is marked
    /// read the instant anything arrives (IRC-style, focus irrelevant);
    /// scrolling up to read history is the only thing that holds messages as
    /// unread.
    pub at_bottom: bool,
    /// Message tinted after a quote-jump so the eye lands on it (the jump
    /// scroll is index-estimated, not pixel-exact).
    pub highlighted_event_id: Option<String>,
    /// A quote-jump deferred one frame: jumping while a search was open
    /// must wait for the close-search reflow to publish fresh scroll
    /// geometry (scroll_task multiplies by the last measured content
    /// height, which still describes the filtered list). Consumed by the
    /// next `Scrolled`.
    pub pending_jump: Option<String>,
    /// Message under the mouse — its floating action bar is showing.
    pub hovered_event_id: Option<String>,
    /// Diagnostic: newest event id seen by the last applied snapshot, so
    /// arrival of genuinely new messages can be logged without spamming on
    /// receipt/reaction churn.
    pub last_seen_newest: Option<String>,
    /// Hide the "new messages" divider locally. Set the moment this client
    /// marks the room read; cleared when new messages arrive while away
    /// from the bottom. The SDK only moves/removes its divider item when
    /// the server echoes the fully-read marker back through sync — an echo
    /// that observably doesn't arrive on this homeserver — so the divider
    /// is driven by what this client *knows* instead of waiting for it.
    pub suppress_unread_divider: bool,
    /// Scroll anchor: a message currently on screen and where its top edge
    /// last sat, in window pixels. When the list reflows under a stationary
    /// user, the anchor is re-probed (`container::visible_bounds`) and the
    /// view scrolled by however far the message actually moved — zero when
    /// content changed above the viewport, exact when it changed below.
    /// Measuring displacement instead of inferring it is what previous
    /// attempts (index estimates, from-top pinning) got wrong.
    pub scroll_anchor: Option<ScrollAnchor>,
    /// Content height reported by the most recent `Scrolled`. iced fires
    /// `on_scroll` on any redraw where the offset *or* the content size
    /// changed — a height delta marks the event as a reflow.
    pub last_content_height: f32,
    /// From-bottom offset at the most recent `Scrolled`. The bottom anchor
    /// keeps this exactly constant across reflows, so any change means real
    /// user input (wheel, scrollbar, or one of our own corrections) — those
    /// events re-learn the anchor and are never corrected against, which is
    /// what keeps scrolling itself free of interference.
    pub last_from_bottom: f32,
    /// Viewport height from the most recent `Scrolled`, for converting a
    /// relative jump target into fixed pixels.
    pub last_bounds_height: f32,
    /// Window-space top edge of the timeline viewport, from the most recent
    /// `Scrolled`. Probe rects clipped by the viewport plateau at this y —
    /// such measurements no longer track the message and are skipped.
    pub viewport_top: f32,
    /// Window-space vertical center of the viewport — when several anchor
    /// candidates probe successfully, the one nearest the center wins
    /// (least likely to scroll off before the next measurement).
    pub viewport_center: f32,
    /// Bumped on every user scroll and every issued correction. Probe tasks
    /// carry the generation they were issued under, and results from an
    /// older generation are discarded: they measured a world that has since
    /// moved. Without this, a correction probe resolving after the user's
    /// next wheel tick would "undo" that tick, and two probes queued by
    /// back-to-back reflows would apply the same correction twice.
    pub scroll_generation: u64,
    /// Whether the most recent scroll input moved toward the bottom, and
    /// when it happened. Anchor corrections hold the current post steady —
    /// the right thing while reading, but while the user is *descending*
    /// toward the live edge the same hold shoves them back up by every
    /// arriving message's height ("it jumps up the second I approach the
    /// bottom"). Corrections are suspended while descending and for a
    /// short grace period after.
    pub descending: bool,
    /// Instant of the most recent user scroll input, for the descent grace
    /// period. `None` until the first scroll.
    pub last_scroll_input: Option<std::time::Instant>,
}

/// A message acting as the timeline's scroll anchor: its event id, the
/// window-space y of its top edge when last measured, and the scroll
/// generation that measurement belongs to. Corrections only trust an
/// anchor measured in the *current* generation — a position from before
/// the user's latest input would revert that input.
#[derive(Debug, Clone)]
pub struct ScrollAnchor {
    pub event_id: String,
    pub y: f32,
    pub generation: u64,
}

/// Why an anchor probe was issued — probes after a user scroll re-learn the
/// anchor position, probes after a content reflow measure displacement to
/// undo it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbePurpose {
    Refresh,
    Correct,
}

#[derive(Debug, Clone)]
pub struct EditingState {
    pub event_id: String,
    pub draft: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    Composer(composer::Message),
    StartEdit { event_id: String, current_body: String },
    EditDraftChanged(String),
    ConfirmEdit,
    CancelEdit,
    RequestDelete(String),
    ConfirmDelete(String),
    CancelDelete,
    ToggleReactionPicker(String),
    ReactWithEmoji { event_id: String, key: String },
    ItemHovered(String),
    ItemUnhovered(String),
    ToggleSearch,
    ToggleMembers,
    ToggleNotifyMenu,
    /// Left-click on a roster member: highlight it in the list (toggle), no
    /// DM. Right-click opens the actions menu instead.
    MemberClicked(String),
    MemberRightClicked(String),
    /// Cursor moved over the roster (roster-local coords) — tracked so the
    /// right-click flyout can anchor at the pointed-at row's height.
    MemberCursorMoved(iced::Point),
    MemberMenuDirectMessage(String),
    MemberMenuNewRoom(String),
    MemberMenuInvite(String),
    /// Toggle tinting this member's messages in the open timeline.
    MemberMenuHighlight(String),
    SearchQueryChanged(String),
    NotificationModeSelected(NotifyChoice),
    LoadOlder,
    Scrolled(iced::widget::scrollable::Viewport),
    /// A `visible_bounds` probe of the anchor message resolved (`None` =
    /// not currently laid out / scrolled out of the viewport).
    AnchorProbed {
        event_id: String,
        purpose: ProbePurpose,
        generation: u64,
        bounds: Option<iced::Rectangle>,
    },
    StartReply(client_core::events::ReplyPreview),
    /// A quote block was clicked — scroll to (and highlight) the quoted
    /// message.
    JumpToEvent(String),
    ZoomImage(String),
    OpenUrl(String),
    /// A video card's play button was clicked — open the in-app player
    /// overlay (the root shell owns the native webview lifecycle).
    PlayVideo { video: crate::video_player::EmbedVideo, title: Option<String> },
    /// Join the room's call (or start one — same command either way).
    JoinCallClicked,
    LeaveCallClicked,
    DismissCallError,
}

pub enum Effect {
    None,
    Composer(composer::Effect),
    Edit { event_id: String, new_body: String },
    Redact { event_id: String },
    ToggleReaction { event_id: String, key: String },
    EnsureEmojiFetched(Vec<String>),
    /// `None` clears the per-room override (back to account default).
    SetNotificationMode(Option<NotificationMode>),
    PaginateBackwards,
    ZoomImage(String),
    /// Open (or create) a DM with this user and switch to it. Whether a
    /// newly created DM is encrypted follows the user's encryption setting.
    OpenDirectMessage(String),
    /// Create a fresh private room with this user and switch to it.
    CreateRoomWith(String),
    /// Invite this user to the currently-open room.
    InviteToRoom(String),
    /// Start the embedded player overlay for this video.
    PlayVideo { video: crate::video_player::EmbedVideo, title: Option<String> },
    /// The user scrolled — if that landed them back at the newest message,
    /// the root dispatcher marks the room read (catching up on anything that
    /// arrived while they were scrolled up).
    MaybeMarkRead,
    /// Join (or start) / leave the open room's MatrixRTC call — the root
    /// dispatcher owns the call state and sends the commands.
    JoinCall,
    LeaveCall,
    DismissCallError,
}

/// Widget id of the message-list scrollable, targeted by quote-jump and
/// anchor-correction scroll tasks.
pub fn timeline_scroll_id() -> iced::widget::scrollable::Id {
    iced::widget::scrollable::Id::new("timeline-scroll")
}

/// Widget id of a message row's wrapper container, addressable by
/// `container::visible_bounds` probes.
fn anchor_container_id(event_id: &str) -> iced::widget::container::Id {
    iced::widget::container::Id::new(format!("msg-{event_id}"))
}

/// Display box for an image message: aspect-true from the sender-declared
/// dimensions when the event carries them (capped at 240×360), a standard
/// 240×180 box otherwise. Deterministic before the bytes arrive, so a
/// finishing download can never change the row's height.
fn image_display_size(width: Option<u32>, height: Option<u32>) -> (u16, u16) {
    const MAX_W: f32 = 240.0;
    const MAX_H: f32 = 360.0;
    match (width, height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => {
            let scale = (MAX_W / w as f32).min(MAX_H / h as f32).min(1.0);
            (
                ((w as f32 * scale).round() as u16).max(24),
                ((h as f32 * scale).round() as u16).max(24),
            )
        }
        _ => (240, 180),
    }
}

/// Display box for a sticker: aspect-true from the sender-declared dimensions
/// (capped at 160×160), a 128×128 box when the event carries none. Never
/// upscales — small stickers stay small. Deterministic before the bytes
/// arrive, like [`image_display_size`].
fn sticker_display_size(width: Option<u32>, height: Option<u32>) -> (u16, u16) {
    const MAX: f32 = 160.0;
    match (width, height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => {
            let scale = (MAX / w as f32).min(MAX / h as f32).min(1.0);
            (
                ((w as f32 * scale).round() as u16).max(24),
                ((h as f32 * scale).round() as u16).max(24),
            )
        }
        _ => (128, 128),
    }
}

/// The event id of the item at `index`, or of the nearest item that has one
/// (dividers and local echoes don't) — searching outward in both directions.
fn nearest_event_id(items: &[TimelineItem], index: usize) -> Option<String> {
    if let Some(id) = items.get(index).and_then(|item| item.event_id.clone()) {
        return Some(id);
    }
    for distance in 1..items.len() {
        let below = index.checked_sub(distance).and_then(|i| items.get(i));
        let above = items.get(index + distance);
        if below.is_none() && above.is_none() {
            return None;
        }
        if let Some(id) = below.and_then(|item| item.event_id.clone()) {
            return Some(id);
        }
        if let Some(id) = above.and_then(|item| item.event_id.clone()) {
            return Some(id);
        }
    }
    None
}

/// Probes the on-screen bounds of `event_id`'s row. The result carries the
/// scroll generation it was issued under so stale measurements can be
/// discarded on arrival.
fn probe_task(event_id: String, purpose: ProbePurpose, generation: u64) -> iced::Task<Message> {
    let id = anchor_container_id(&event_id);
    iced::widget::container::visible_bounds(id).map(move |bounds| Message::AnchorProbed {
        event_id: event_id.clone(),
        purpose,
        generation,
        bounds,
    })
}

/// Probe candidates for a fresh anchor: the hovered message first (it is
/// on-screen by definition), then items spread around the uniform-height
/// estimate of the viewport middle — the spread is what makes anchoring
/// reliable when wildly uneven item heights throw the estimate off by
/// whole screens. Candidates only have to land *some* on-screen message;
/// the probe measures its true position, so estimate error never reaches
/// the scroll math. Empty when the whole list fits on screen.
fn anchor_candidates(
    state: &State,
    viewport: &iced::widget::scrollable::Viewport,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |id: Option<String>| {
        if let Some(id) = id {
            if !out.contains(&id) {
                out.push(id);
            }
        }
    };
    push(state.hovered_event_id.clone());

    if state.items.is_empty() {
        return out;
    }
    let last = state.items.len() as isize - 1;

    // The current anchor was on screen one tick ago — after a single wheel
    // tick it (or an immediate list neighbor) almost certainly still is.
    // Tracking its neighborhood keeps anchoring self-sustaining once
    // established instead of re-guessing from scratch every tick, and is
    // far more reliable than any height estimate.
    if let Some(anchor) = &state.scroll_anchor {
        if let Some(index) = state
            .items
            .iter()
            .position(|item| item.event_id.as_deref() == Some(anchor.event_id.as_str()))
        {
            for offset in [0isize, -1, 1, -2, 2, -4, 4, -8, 8] {
                let index = (index as isize + offset).clamp(0, last) as usize;
                push(nearest_event_id(&state.items, index));
            }
            return out;
        }
    }

    let bounds_height = viewport.bounds().height;
    let content_height = viewport.content_bounds().height;
    if content_height <= bounds_height {
        return out;
    }
    let from_top = viewport.absolute_offset_reversed().y;
    let middle = ((from_top + bounds_height * 0.5) / content_height).clamp(0.0, 1.0);
    let base = (middle * (state.items.len() - 1) as f32).round() as isize;
    // With a hovered message in hand (guaranteed on-screen), it plus the
    // middle estimate suffice — the full spread costs ~12 extra widget-tree
    // traversals per scroll tick, and scrolling with the pointer over the
    // list hovers a message almost always. The spread earns its keep only
    // when nothing is hovered (keyboard scroll, pointer off the list).
    let offsets: &[isize] = if state.hovered_event_id.is_none() {
        &[0, -2, 2, -5, 5, -10, 10, -20, 20, -40, 40, -80, 80]
    } else {
        &[0]
    };
    for &offset in offsets {
        let index = (base + offset).clamp(0, last) as usize;
        push(nearest_event_id(&state.items, index));
    }
    out
}

/// Bottom-anchored relative offset (`0.0` = newest/bottom, `1.0` =
/// oldest/top) for the item at `index` out of `len` total items. The
/// scrollable has no notion of item identity, only pixels, so this is an
/// estimate that assumes uniform item height — accurate enough to land
/// the target on screen, not to land it at a precise pixel.
fn relative_offset_for_index(index: usize, len: usize) -> f32 {
    let len = len.max(2);
    (1.0 - (index as f32 / (len - 1) as f32)).clamp(0.0, 1.0)
}

/// Builds a task that scrolls to a bottom-anchored relative position
/// (quote jumps), as a *fixed pixel* offset computed from the tracked
/// viewport geometry. Never `snap_to`: that stores the offset as a
/// percentage of content height, and a percentage-mode offset
/// re-materializes on every layout — each history batch or new message
/// then drags the view proportionally (measured twice in the wild as
/// `from_bottom` moving in lockstep with content height: the "forever
/// scroll", and later the unread divider refusing to clear because the
/// tracked offset never read as "at the bottom" again). The jump target
/// is an estimate anyway (uniform item heights); the highlight marks the
/// exact message once it's on screen.
fn scroll_task(state: &State, relative_y: f32) -> iced::Task<Message> {
    let range = (state.last_content_height - state.last_bounds_height).max(0.0);
    iced::widget::scrollable::scroll_to(
        timeline_scroll_id(),
        iced::widget::scrollable::AbsoluteOffset { x: 0.0, y: relative_y * range },
    )
}

/// Window within which two clicks on the same roster member count as a
/// double-click (opening an unencrypted DM).
const MEMBER_DOUBLE_CLICK: std::time::Duration = std::time::Duration::from_millis(400);

pub fn update(
    state: &mut State,
    message: Message,
    spell: &crate::spellcheck_config::SpellcheckConfig,
) -> (iced::Task<Message>, Effect) {
    match message {
        Message::Composer(msg) => {
            let (task, effect) = composer::update(&mut state.composer, msg, spell);
            (task.map(Message::Composer), Effect::Composer(effect))
        }
        Message::StartEdit { event_id, current_body } => {
            state.editing = Some(EditingState { event_id, draft: current_body });
            (iced::Task::none(), Effect::None)
        }
        Message::EditDraftChanged(draft) => {
            if let Some(editing) = &mut state.editing {
                editing.draft = draft;
            }
            (iced::Task::none(), Effect::None)
        }
        Message::ConfirmEdit => {
            let Some(editing) = &state.editing else {
                return (iced::Task::none(), Effect::None);
            };
            let new_body = editing.draft.trim().to_string();
            if new_body.is_empty() {
                return (iced::Task::none(), Effect::None);
            }
            let event_id = editing.event_id.clone();
            (iced::Task::none(), Effect::Edit { event_id, new_body })
        }
        Message::CancelEdit => {
            state.editing = None;
            (iced::Task::none(), Effect::None)
        }
        Message::RequestDelete(event_id) => {
            state.confirm_delete = Some(event_id);
            (iced::Task::none(), Effect::None)
        }
        Message::ConfirmDelete(event_id) => (iced::Task::none(), Effect::Redact { event_id }),
        Message::CancelDelete => {
            state.confirm_delete = None;
            (iced::Task::none(), Effect::None)
        }
        Message::ToggleReactionPicker(event_id) => {
            let opening = state.reacting_to.as_deref() != Some(event_id.as_str());
            state.reacting_to = if opening { Some(event_id) } else { None };
            let effect = if opening {
                Effect::EnsureEmojiFetched(crate::emoji_picker::all_unicode_glyphs())
            } else {
                Effect::None
            };
            (iced::Task::none(), effect)
        }
        Message::ReactWithEmoji { event_id, key } => {
            state.reacting_to = None;
            (iced::Task::none(), Effect::ToggleReaction { event_id, key })
        }
        Message::ToggleSearch => {
            state.search_open = !state.search_open;
            if !state.search_open {
                state.search_query.clear();
            }
            recompute_search_matches(state);
            // Filtering rebuilds the list; the anchor's stored position no
            // longer means anything. Corrections pause until the next
            // scroll learns a fresh one.
            state.scroll_anchor = None;
            (iced::Task::none(), Effect::None)
        }
        Message::ToggleMembers => {
            state.hide_members = !state.hide_members;
            // Panel show/hide changes the list width → text rewraps → every
            // row's position changes legitimately.
            state.scroll_anchor = None;
            (iced::Task::none(), Effect::None)
        }
        Message::ItemHovered(event_id) => {
            state.hovered_event_id = Some(event_id);
            (iced::Task::none(), Effect::None)
        }
        Message::ItemUnhovered(event_id) => {
            // Only clear if another item's enter didn't already take over.
            if state.hovered_event_id.as_deref() == Some(event_id.as_str()) {
                state.hovered_event_id = None;
            }
            (iced::Task::none(), Effect::None)
        }
        Message::ToggleNotifyMenu => {
            state.notify_menu_open = !state.notify_menu_open;
            (iced::Task::none(), Effect::None)
        }
        Message::MemberClicked(user_id) => {
            state.member_menu = None;
            // Double-click (same member, in quick succession) opens an
            // unencrypted DM. A single click only toggles the roster
            // highlight — it deliberately no longer opens a room.
            let is_double = state
                .last_member_click
                .as_ref()
                .is_some_and(|(prev, at)| {
                    prev == &user_id && at.elapsed() < MEMBER_DOUBLE_CLICK
                });
            if is_double {
                state.last_member_click = None;
                return (iced::Task::none(), Effect::OpenDirectMessage(user_id));
            }
            state.last_member_click = Some((user_id.clone(), std::time::Instant::now()));
            state.selected_member =
                if state.selected_member.as_deref() == Some(user_id.as_str()) {
                    None
                } else {
                    Some(user_id)
                };
            (iced::Task::none(), Effect::None)
        }
        Message::MemberRightClicked(user_id) => {
            // Toggle: right-clicking the member whose menu is already open
            // closes it (the flyout has no dismiss backdrop).
            if state.member_menu.as_deref() == Some(user_id.as_str()) {
                state.member_menu = None;
            } else {
                state.selected_member = Some(user_id.clone());
                state.member_menu = Some(user_id);
                // Pin the flyout at the row we're pointing at (frozen so it
                // doesn't follow the mouse once open).
                state.member_menu_anchor_y = state.member_cursor.map(|p| p.y).unwrap_or(0.0);
            }
            (iced::Task::none(), Effect::None)
        }
        Message::MemberCursorMoved(point) => {
            state.member_cursor = Some(point);
            (iced::Task::none(), Effect::None)
        }
        Message::MemberMenuDirectMessage(user_id) => {
            state.member_menu = None;
            (iced::Task::none(), Effect::OpenDirectMessage(user_id))
        }
        Message::MemberMenuNewRoom(user_id) => {
            state.member_menu = None;
            (iced::Task::none(), Effect::CreateRoomWith(user_id))
        }
        Message::MemberMenuInvite(user_id) => {
            state.member_menu = None;
            (iced::Task::none(), Effect::InviteToRoom(user_id))
        }
        Message::MemberMenuHighlight(user_id) => {
            state.member_menu = None;
            state.highlighted_member =
                if state.highlighted_member.as_deref() == Some(user_id.as_str()) {
                    None
                } else {
                    Some(user_id)
                };
            (iced::Task::none(), Effect::None)
        }
        Message::SearchQueryChanged(query) => {
            state.search_query = query;
            recompute_search_matches(state);
            // Filtering rebuilds the list; drop the now-meaningless anchor.
            state.scroll_anchor = None;
            (iced::Task::none(), Effect::None)
        }
        Message::NotificationModeSelected(choice) => {
            state.notify_menu_open = false;
            (iced::Task::none(), Effect::SetNotificationMode(choice.to_mode()))
        }
        Message::LoadOlder => {
            if state.loading_older || state.reached_start {
                return (iced::Task::none(), Effect::None);
            }
            state.loading_older = true;
            (iced::Task::none(), Effect::PaginateBackwards)
        }
        Message::Scrolled(viewport) => {
            // The list is bottom-anchored: `absolute_offset().y` is the
            // distance scrolled up from the newest message, and the
            // reversed offset is the distance from the top.
            //
            // iced publishes this event both for real scrolls and for
            // redraws where the *content size* changed under a stationary
            // viewport (new message, reaction, receipt, preview card, image
            // — anything that reflows the list). Telling them apart:
            // reflows leave the from-bottom offset untouched (that's what
            // the bottom anchor preserves), so a from-bottom change means
            // real input; a content-height change means a reflow.
            let content_height = viewport.content_bounds().height;
            let from_bottom = viewport.absolute_offset().y;
            let height_delta = content_height - state.last_content_height;
            let reflowed = height_delta.abs() > 0.5;
            let user_scrolled = (from_bottom - state.last_from_bottom).abs() > 0.5;
            let moved_toward_bottom = from_bottom < state.last_from_bottom;
            // Whether real input happened shortly *before* this event —
            // evaluated before `last_scroll_input` is refreshed below.
            let recently_scrolled = state
                .last_scroll_input
                .is_some_and(|at| at.elapsed() < std::time::Duration::from_millis(1500));
            state.last_content_height = content_height;
            state.last_from_bottom = from_bottom;
            state.last_bounds_height = viewport.bounds().height;
            state.viewport_top = viewport.bounds().y;
            state.viewport_center = viewport.bounds().y + viewport.bounds().height * 0.5;

            // A quote-jump deferred by JumpToEvent (search was open, so the
            // geometry was stale): the fields above are fresh now — scroll.
            if let Some(event_id) = state.pending_jump.take() {
                if let Some(index) =
                    state.items.iter().position(|i| i.event_id.as_deref() == Some(&event_id))
                {
                    let target = relative_offset_for_index(index, state.items.len());
                    return (scroll_task(state, target), Effect::None);
                }
            }

            // `at_bottom` is pure geometry and gates read receipts — it
            // must reflect every event, whatever caused it. (It went stale
            // once by only updating on user input: a flip during the
            // room-open burst then latched forever while the user sat at
            // the bottom, receipts never fired, and the unread divider
            // became immortal.)
            let was_at_bottom = state.at_bottom;
            state.at_bottom = from_bottom <= 24.0;
            let arrived_at_bottom = !was_at_bottom && state.at_bottom;

            if user_scrolled {
                state.descending = moved_toward_bottom;
                state.last_scroll_input = Some(std::time::Instant::now());
            }

            if user_scrolled || !reflowed {
                // The offset moved: real input (wheel, scrollbar drag, a
                // quote jump, the settle of one of our own corrections) —
                // or an offset *clamp* from the content shrinking in the
                // same frame. Either way the on-screen world moved: any
                // probe still in flight measured a world that's gone.
                state.scroll_generation += 1;

                // Re-measure the anchor from several candidates at once —
                // the hovered message plus a spread around the estimate —
                // and let the probe results pick the best (closest to the
                // viewport center). Batching many candidates is what keeps
                // an anchor established nearly all the time; a miss means
                // the next reflow can't be corrected at all.
                let probe = if from_bottom > 24.0 {
                    let probes: Vec<_> = anchor_candidates(state, &viewport)
                        .into_iter()
                        .map(|event_id| {
                            probe_task(event_id, ProbePurpose::Refresh, state.scroll_generation)
                        })
                        .collect();
                    if probes.is_empty() {
                        state.scroll_anchor = None;
                        iced::Task::none()
                    } else {
                        iced::Task::batch(probes)
                    }
                } else {
                    // At the bottom the bottom-anchor is already the
                    // correct (and exact) behavior; drop the anchor.
                    state.scroll_anchor = None;
                    iced::Task::none()
                };

                // Offset-tracks-content signature: the offset moved in the
                // same frame as a reflow, with no human input anywhere
                // near. That is not scrolling — it's the offset being
                // *derived* from the content size (a percentage-mode
                // offset, or a clamp), and it drifts forever if left
                // alone. Re-pin the current position as fixed pixels;
                // harmless when the offset is already fixed.
                let task = if user_scrolled && reflowed && !recently_scrolled {
                    tracing::info!(
                        from_bottom,
                        height_delta,
                        "offset moved with content, no recent input — re-pinning as fixed pixels"
                    );
                    iced::Task::batch([
                        probe,
                        iced::widget::scrollable::scroll_to(
                            timeline_scroll_id(),
                            iced::widget::scrollable::AbsoluteOffset { x: 0.0, y: from_bottom },
                        ),
                    ])
                } else {
                    probe
                };

                // Pagination still moves only on pure input — an offset
                // that changed in the same frame as a reflow can be a clamp
                // from a shrunken (reset) timeline, and a clamp is not the
                // user asking for older history. Reaching the bottom is
                // different: however the frame was classified, being at the
                // newest message is what "caught up" means here.
                if reflowed {
                    if arrived_at_bottom {
                        return (task, Effect::MaybeMarkRead);
                    }
                    return (task, Effect::None);
                }

                // Nearing the top auto-loads more history — the explicit
                // "Load older" control stays as a fallback for short
                // timelines that can't scroll yet. (Restored original
                // behavior: while the timeline was accidentally
                // top-anchored, `absolute_offset_reversed` measured
                // distance from the *bottom*, so this trigger fired on
                // every approach to the live edge and looked like runaway
                // history loading. Under the correct anchor it fires only
                // near the real top, and each prepended batch is invisible
                // to a bottom-anchored viewport.)
                let from_top = viewport.absolute_offset_reversed().y;
                if from_top <= 60.0 && !state.loading_older && !state.reached_start {
                    state.loading_older = true;
                    return (task, Effect::PaginateBackwards);
                }
                // Only signal when the user actually returns to the newest
                // message — that's when there may be messages received
                // while scrolled up that now need marking read.
                if arrived_at_bottom {
                    return (task, Effect::MaybeMarkRead);
                }
                return (task, Effect::None);
            }

            // Pure reflow under a stationary user.

            // Live-edge glue (within 150px of the bottom): the bottom
            // anchor keeps the *distance* to the bottom constant, so any
            // reflow leaves that gap sitting open rather than at 0 — a new
            // message grows the list entirely below the window (invisible,
            // nothing sticks), but a *shrink* (e.g. a grouped header
            // collapsing once more history reveals the sender continuation
            // above it — logged in the wild as `height_delta=-184.5`) does
            // the same thing in reverse. Both leave a nonzero gap; snap it
            // closed either way. Skipped only while the user is actively
            // scrolling *up*: they're leaving the live edge on purpose.
            if from_bottom <= 150.0 {
                let leaving = recently_scrolled && !state.descending;
                if from_bottom > 0.5 && !leaving {
                    tracing::debug!(from_bottom, height_delta, "live-edge glue → bottom");
                    return (
                        iced::widget::scrollable::scroll_to(
                            timeline_scroll_id(),
                            iced::widget::scrollable::AbsoluteOffset { x: 0.0, y: 0.0 },
                        ),
                        Effect::None,
                    );
                }
                return (iced::Task::none(), Effect::None);
            }

            // While the user is descending toward the bottom (or paused
            // less than a moment ago), corrections stay suspended
            // entirely: holding the current post steady is right while
            // *reading*, but during a descent it would shove them back up
            // by every arriving message's height. Otherwise, measure how
            // far the anchor message actually moved and undo it — but only
            // against an anchor measured in the current generation: an
            // older measurement predates the user's latest input, and
            // "restoring" it would revert that input.
            if state.descending && recently_scrolled {
                return (iced::Task::none(), Effect::None);
            }
            if !state.at_bottom && from_bottom > 150.0 {
                match &state.scroll_anchor {
                    Some(anchor) if anchor.generation == state.scroll_generation => {
                        return (
                            probe_task(
                                anchor.event_id.clone(),
                                ProbePurpose::Correct,
                                state.scroll_generation,
                            ),
                            Effect::None,
                        );
                    }
                    _ => {
                        // No anchor was ever established — logged in the
                        // wild as this reflow shifting the view by
                        // `height_delta` with nothing to undo it, over and
                        // over for an entire session, because establishing
                        // an anchor previously only happened in response
                        // to the user's *own* scroll input. A user who
                        // never touches the scrollbar (just sitting,
                        // reading, watching a fast room) got no coverage
                        // at all. Probe candidates now regardless, purely
                        // from this reflow, so the *next* one has
                        // something fresh to correct against — this
                        // reflow itself still can't be undone (no "before"
                        // measurement exists for it).
                        tracing::debug!(
                            height_delta,
                            from_bottom,
                            "uncorrected timeline reflow — establishing anchor for next time"
                        );
                        state.scroll_generation += 1;
                        let probes: Vec<_> = anchor_candidates(state, &viewport)
                            .into_iter()
                            .map(|event_id| {
                                probe_task(event_id, ProbePurpose::Refresh, state.scroll_generation)
                            })
                            .collect();
                        if !probes.is_empty() {
                            return (iced::Task::batch(probes), Effect::None);
                        }
                    }
                }
            }
            (iced::Task::none(), Effect::None)
        }
        Message::AnchorProbed { event_id, purpose, generation, bounds } => {
            if generation != state.scroll_generation {
                // Issued before the last user scroll or correction — it
                // measured a world that has since moved. Acting on it
                // would revert real input. If the current anchor is from
                // that older world too, it must go with it: keeping it
                // means the next reflow "restores" a pre-input position.
                if state
                    .scroll_anchor
                    .as_ref()
                    .is_some_and(|anchor| anchor.generation != state.scroll_generation)
                {
                    state.scroll_anchor = None;
                }
                return (iced::Task::none(), Effect::None);
            }
            // Rects clipped by the viewport's top edge plateau there and no
            // longer track the message — treat them as unusable.
            let usable =
                bounds.filter(|rect| rect.y > state.viewport_top + 0.5 && rect.height > 1.0);
            match (purpose, usable) {
                (ProbePurpose::Refresh, Some(rect)) => {
                    // Several candidates race per scroll tick; keep the one
                    // nearest the viewport center (least likely to leave
                    // the screen before the next measurement).
                    let replace = match &state.scroll_anchor {
                        Some(anchor) if anchor.generation == generation => {
                            (rect.y - state.viewport_center).abs()
                                < (anchor.y - state.viewport_center).abs()
                        }
                        _ => true,
                    };
                    if replace {
                        state.scroll_anchor = Some(ScrollAnchor { event_id, y: rect.y, generation });
                    }
                    (iced::Task::none(), Effect::None)
                }
                // This candidate wasn't on screen; siblings from the same
                // batch may still land. A leftover anchor from an older
                // generation is stale by definition — drop it rather than
                // let a later reflow correct against it.
                (ProbePurpose::Refresh, None) => {
                    if state
                        .scroll_anchor
                        .as_ref()
                        .is_some_and(|anchor| anchor.generation != generation)
                    {
                        state.scroll_anchor = None;
                    }
                    (iced::Task::none(), Effect::None)
                }
                (ProbePurpose::Correct, Some(rect)) => {
                    let Some(anchor) = &state.scroll_anchor else {
                        return (iced::Task::none(), Effect::None);
                    };
                    if anchor.event_id != event_id {
                        // Probe of a superseded anchor.
                        return (iced::Task::none(), Effect::None);
                    }
                    let delta = rect.y - anchor.y;
                    if delta.abs() > 1.0 {
                        // The message moved `delta` px on screen; move the
                        // view the same amount so it lands back where it
                        // was. From-bottom offsets grow toward older
                        // content, hence the sign flip. Bump the
                        // generation so a second in-flight probe can't
                        // apply this same correction again.
                        state.scroll_generation += 1;
                        tracing::debug!(delta, event_id, "timeline reflow correction");
                        return (
                            iced::widget::scrollable::scroll_by(
                                timeline_scroll_id(),
                                iced::widget::scrollable::AbsoluteOffset { x: 0.0, y: -delta },
                            ),
                            Effect::None,
                        );
                    }
                    (iced::Task::none(), Effect::None)
                }
                // The anchor's row left the layout entirely (redacted,
                // filtered out, or scrolled far away) — nothing sensible
                // to correct against anymore.
                (ProbePurpose::Correct, None) => {
                    state.scroll_anchor = None;
                    (iced::Task::none(), Effect::None)
                }
            }
        }
        Message::StartReply(preview) => {
            state.composer.replying_to = Some(preview);
            (iced::Task::none(), Effect::None)
        }
        Message::JumpToEvent(event_id) => {
            // The jump math targets the full list, but with a search active
            // the scrollable renders the filtered subset — the snap would
            // land somewhere arbitrary, and a non-matching target wouldn't
            // even be rendered. Close the search first (as ToggleSearch
            // would) so the target is always visible.
            let was_searching = state.search_open;
            if state.search_open {
                state.search_open = false;
                state.search_query.clear();
                state.search_matches.clear();
                state.scroll_anchor = None;
            }
            let index = state.items.iter().position(|i| i.event_id.as_deref() == Some(&event_id));
            match index {
                Some(index) => {
                    // Index-proportional estimate: the scrollable is
                    // bottom-anchored, where relative 0.0 = newest (bottom)
                    // and 1.0 = oldest (top). Variable item heights make
                    // this approximate — the highlight marks the exact
                    // message once it's on screen.
                    state.highlighted_event_id = Some(event_id.clone());
                    if was_searching {
                        // The geometry fields still describe the FILTERED
                        // list; closing the search reflows it and iced
                        // publishes a fresh Scrolled on the next redraw —
                        // defer the jump one frame and scroll there.
                        state.pending_jump = Some(event_id);
                        return (iced::Task::none(), Effect::None);
                    }
                    let from_bottom = relative_offset_for_index(index, state.items.len());
                    (scroll_task(state, from_bottom), Effect::None)
                }
                // The quoted message is older than what's loaded — pull
                // one batch of history; clicking again digs further.
                None if !state.loading_older && !state.reached_start => {
                    state.loading_older = true;
                    (iced::Task::none(), Effect::PaginateBackwards)
                }
                None => (iced::Task::none(), Effect::None),
            }
        }
        Message::ZoomImage(url) => (iced::Task::none(), Effect::ZoomImage(url)),
        Message::OpenUrl(url) => {
            let _ = open::that(url);
            (iced::Task::none(), Effect::None)
        }
        Message::PlayVideo { video, title } => {
            (iced::Task::none(), Effect::PlayVideo { video, title })
        }
        Message::JoinCallClicked => (iced::Task::none(), Effect::JoinCall),
        Message::LeaveCallClicked => (iced::Task::none(), Effect::LeaveCall),
        Message::DismissCallError => (iced::Task::none(), Effect::DismissCallError),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn view<'a>(
    state: &'a State,
    own_user_id: Option<&'a str>,
    room: Option<&'a RoomSummary>,
    notification_mode: Option<NotificationMode>,
    calls: &'a crate::screens::call::State,
    emoji_usage: &'a HashMap<String, u32>,
    media: &'a crate::media_cache::State,
    packs: &'a [EmojiPack],
    stickers: &'a [crate::state::CollectedSticker],
    shortcode_index: &'a HashMap<String, client_core::events::CustomEmoji>,
    url_previews: &'a HashMap<String, Option<UrlPreview>>,
    tweet_previews: &'a HashMap<String, Option<crate::tweets::TweetData>>,
    steam_previews: &'a HashMap<String, Option<crate::steam::SteamAppData>>,
) -> Element<'a, Message> {
    let Some(open_room_id) = state.room_id.as_deref() else {
        return container(text("Select a room to view its messages"))
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into();
    };

    let searching = state.search_open && !state.search_query.trim().is_empty();
    let match_count = state.search_matches.len();

    let mut list = column![].spacing(10).padding(12).width(Length::Fill);

    if !searching {
        let history_control: Element<'a, Message> = if state.reached_start {
            text("— beginning of conversation —").size(12).style(text::secondary).into()
        } else if state.loading_older {
            text("Loading older messages...").size(12).style(text::secondary).into()
        } else {
            button(text("Load older messages").size(12))
                .on_press(Message::LoadOlder)
                .style(crate::theme::ghost_button)
                .padding([4, 8])
                .into()
        };
        list = list.push(container(history_control).width(Length::Fill).center_x(Length::Fill));
    }

    // Consecutive messages from the same sender collapse into one visual
    // group: only the first shows the avatar + name/timestamp header.
    let mut previous_sender: Option<&str> = None;
    for (index, item) in state.items.iter().enumerate() {
        if searching && !state.search_matches.contains(&index) {
            continue;
        }
        // Locally-cleared unread boundary (this client marked the room
        // read; the SDK's own divider only moves on a server echo that
        // observably never arrives here).
        if state.suppress_unread_divider
            && matches!(item.content, TimelineItemContent::NewMessagesDivider)
        {
            continue;
        }
        let show_header = match &item.content {
            TimelineItemContent::DateDivider(_) | TimelineItemContent::NewMessagesDivider => {
                previous_sender = None;
                true
            }
            _ => {
                let grouped = previous_sender == Some(item.sender.as_str())
                    && item.in_reply_to.is_none()
                    && !searching;
                previous_sender = Some(item.sender.as_str());
                !grouped
            }
        };
        let rendered = render_item(
            item,
            state.first_urls.get(index).and_then(|u| u.as_deref()),
            own_user_id,
            show_header,
            state.hovered_event_id.as_deref(),
            &state.editing,
            &state.confirm_delete,
            state.highlighted_event_id.as_deref(),
            state.highlighted_member.as_deref(),
            &state.composer.member_candidates,
            &state.member_index,
            &state.power_tags,
            media,
            shortcode_index,
            url_previews,
            tweet_previews,
            steam_previews,
        );
        // Rows with an event id get an addressable wrapper so scroll-anchor
        // probes (`container::visible_bounds`) can measure where a specific
        // message actually sits on screen.
        list = list.push(match &item.event_id {
            Some(event_id) => container(rendered)
                .id(anchor_container_id(event_id))
                .width(Length::Fill)
                .into(),
            None => rendered,
        });
    }

    // Conditional pieces render inside always-present slots so the widget
    // tree keeps its shape — otherwise the composer input loses focus every
    // time the typing line or an error appears/disappears (see
    // `theme::slot`).
    let error_slot = crate::theme::slot(
        state
            .action_error
            .as_ref()
            .map(|err| text(err.clone()).style(text::danger).size(12).into()),
    );
    // The typing indicator and the mini avatars of everyone "following the
    // conversation" (read receipts at the newest message, Cinny-style) live
    // inside the composer's toolbar next to Send — no extra status row.
    // Always rendered so the widget tree keeps its shape.
    let typing: Element<'a, composer::Message> =
        typing_line(state).unwrap_or_else(|| text("").size(12).into());
    let followers = follower_avatars(
        &state.items,
        own_user_id,
        &state.composer.member_candidates,
        &state.member_index,
        media,
    );
    let bottom = column![
        error_slot,
        composer::view(&state.composer, emoji_usage, media, packs, stickers, typing, followers)
            .map(Message::Composer),
    ]
    .spacing(4);

    let search_slot = crate::theme::slot(state.search_open.then(|| {
        let mut search_row = row![
            text_input("Search messages...", &state.search_query)
                .on_input(Message::SearchQueryChanged)
                .padding(6)
                .size(13),
        ]
        .spacing(8)
        .align_y(iced::Center);
        if searching {
            search_row = search_row.push(
                text(format!("{match_count} match{}", if match_count == 1 { "" } else { "es" }))
                    .size(12)
                    .style(text::secondary),
            );
        }
        container(search_row).padding([4, 12]).into()
    }));

    // Notification-mode menu, opened from the header bell. A slot (like
    // search) so toggling it never reshapes the tree under the composer. The
    // options right-align under the bell that opened them.
    let notify_slot = crate::theme::slot(state.notify_menu_open.then(|| {
        let current = NotifyChoice::from_mode(notification_mode);
        let mut choices =
            row![text("Notify").size(12).style(text::secondary)].spacing(6).align_y(iced::Center);
        for choice in NOTIFY_CHOICES {
            let style = if choice == current {
                crate::theme::selected_ghost_button
            } else {
                crate::theme::ghost_button
            };
            choices = choices.push(
                button(text(choice.to_string()).size(12))
                    .on_press(Message::NotificationModeSelected(choice))
                    .style(style)
                    .padding([4, 8]),
            );
        }
        container(choices).width(Length::Fill).align_x(iced::Right).padding([4, 12]).into()
    }));

    // Below the header, above search/messages. Always a slot so the
    // composer doesn't lose focus when a call starts or ends.
    let call_banner = crate::theme::slot(crate::screens::call::banner(
        calls,
        open_room_id,
        &state.composer.member_candidates,
        &state.member_index,
        media,
        Message::JoinCallClicked,
        Message::LeaveCallClicked,
        Message::DismissCallError,
    ));

    let main: Element<'a, Message> = column![
        header(state, room, notification_mode, calls, open_room_id),
        call_banner,
        notify_slot,
        search_slot,
        scrollable(list)
            .id(timeline_scroll_id())
            .height(Length::Fill)
            .on_scroll(Message::Scrolled)
            // Chat semantics: open at the newest message, stay pinned
            // to the bottom as messages arrive, and don't jump when
            // older history is prepended. The anchor lives *inside* the
            // per-axis `Scrollbar` config, so it must be set on the
            // scrollbar passed to `.direction(...)` — the previous
            // free-standing `.anchor_bottom()` call before
            // `.direction(...)` was silently wiped when `.direction`
            // replaced the whole Direction struct (default anchor: top),
            // leaving the timeline top-anchored at runtime while every
            // piece of scroll logic assumed offset 0 = bottom. That one
            // ordering mistake was the root cause of the entire
            // scroll-jumping saga.
            .direction(iced::widget::scrollable::Direction::Vertical(
                iced::widget::scrollable::Scrollbar::new()
                    .width(6)
                    .scroller_width(6)
                    .anchor(iced::widget::scrollable::Anchor::End),
            ))
            .style(crate::theme::thin_scrollbar),
        bottom,
    ]
    .height(Length::Fill)
    .width(Length::Fill)
    .into();

    let members_shown = !state.hide_members && !state.composer.member_candidates.is_empty();
    let mut layout = row![main];
    if members_shown {
        layout = layout.push(member_panel(
            &state.composer.member_candidates,
            &state.power_tags,
            state.selected_member.as_deref(),
            media,
        ));
    }

    // The reaction picker floats as a centered overlay: expanding it
    // inline under a message near the viewport's bottom edge got the
    // panel clipped mid-row. Always a stack so the tree keeps its shape.
    let picker_overlay = state.reacting_to.as_deref().map(|event_id| {
        let panel = container(reaction_picker(
            event_id,
            emoji_usage,
            media,
            packs,
        ))
        .max_width(430)
        .style(|theme: &iced::Theme| {
            let palette = theme.extended_palette();
            iced::widget::container::Style {
                background: Some(palette.background.weak.color.into()),
                border: iced::Border {
                    color: palette.background.strong.color,
                    width: 1.0,
                    radius: 10.into(),
                },
                ..iced::widget::container::Style::default()
            }
        });
        let close = Message::ToggleReactionPicker(event_id.to_string());
        iced::widget::opaque(
            iced::widget::mouse_area(iced::widget::center(iced::widget::opaque(panel)))
                .on_press(close),
        )
    });
    // The member actions menu is a flyout pinned just to the LEFT of the
    // roster, at the height of the right-clicked row (`member_menu_anchor_y`,
    // frozen at click time). Positioned with a top spacer + right-alignment
    // rather than absolute coords, which iced doesn't expose. The surrounding
    // area is inert (empty padding/space), so clicks off the menu fall through
    // to the timeline and roster below.
    let member_flyout = state.member_menu.as_deref().filter(|_| members_shown).map(|user_id| {
        // Keep the menu from spilling off the bottom: clamp its top so a
        // ~4-row menu still fits in the visible height.
        let max_y = (state.last_bounds_height - 140.0).max(0.0);
        let anchor_y = state.member_menu_anchor_y.clamp(0.0, max_y);
        let menu = member_menu_actions(user_id, state.highlighted_member.as_deref());
        let positioned = column![iced::widget::Space::with_height(Length::Fixed(anchor_y)), menu];
        container(positioned)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced::Right)
            // Right inset = roster width (200) + its 6px padding, so the menu's
            // right edge lands just left of the list.
            .padding(iced::Padding { top: 0.0, right: 206.0, bottom: 0.0, left: 0.0 })
    });
    iced::widget::stack![layout.height(Length::Fill)]
        .push_maybe(picker_overlay)
        .push_maybe(member_flyout)
        .into()
}

/// The member actions menu, rendered as a floating flyout to the left of the
/// roster at the right-clicked row's height (positioned in `view`). A solid
/// bordered panel so it stays legible over the timeline. Dismisses by picking
/// an action, left-clicking any member, or right-clicking the same member
/// again.
fn member_menu_actions<'a>(user_id: &str, highlighted_member: Option<&str>) -> Element<'a, Message> {
    let action = |label: &'static str, message: Message| {
        button(text(label).size(12))
            .on_press(message)
            .style(crate::theme::ghost_button)
            .width(Length::Fill)
            .padding([5, 8])
    };

    let highlight_label =
        if highlighted_member == Some(user_id) { "Clear highlight" } else { "Highlight messages" };

    let owned = user_id.to_string();
    let menu = column![
        action("Direct message", Message::MemberMenuDirectMessage(owned.clone())),
        action("New room with them", Message::MemberMenuNewRoom(owned.clone())),
        action("Invite to this room", Message::MemberMenuInvite(owned.clone())),
        action(highlight_label, Message::MemberMenuHighlight(owned)),
    ]
    .spacing(2);

    container(menu)
        .width(Length::Fixed(190.0))
        .padding(4)
        .style(|theme: &iced::Theme| {
            let palette = theme.extended_palette();
            iced::widget::container::Style {
                background: Some(palette.background.base.color.into()),
                border: iced::Border {
                    color: palette.background.strong.color,
                    width: 1.0,
                    radius: 8.into(),
                },
                ..iced::widget::container::Style::default()
            }
        })
        .into()
}

/// Right-hand member list. Grouped by the room's MSC3949 power-level tags
/// when it defines them ("Red team", "Purple team", ... with their colors,
/// like Cinny/FluffyChat show); otherwise by the conventional
/// Admin/Moderator/Member power-level bands. Left-click highlights a member
/// in the list (`selected_member`); right-click opens the actions flyout
/// (rendered separately, in `view`, to the left of the list). The whole panel
/// is wrapped in a `mouse_area` that reports the cursor position so the flyout
/// can anchor at the pointed-at row.
fn member_panel<'a>(
    members: &'a [RoomMember],
    tags: &'a [client_core::events::PowerLevelTag],
    selected_member: Option<&'a str>,
    media: &'a crate::media_cache::State,
) -> Element<'a, Message> {
    // (label, group color, members). The color tints the section header and
    // its member rows; `None` for the Admin/Mod/Member fallback bands and the
    // below-all-tags catch-all, which have no tag color to draw from.
    let mut groups: Vec<(String, Option<iced::Color>, Vec<&RoomMember>)> = Vec::new();

    if tags.is_empty() {
        groups.push(("Admin".to_string(), None, Vec::new()));
        groups.push(("Moderator".to_string(), None, Vec::new()));
        groups.push(("Member".to_string(), None, Vec::new()));
        for member in members {
            let index = if member.power_level >= 100 {
                0
            } else if member.power_level >= 50 {
                1
            } else {
                2
            };
            groups[index].2.push(member);
        }
    } else {
        for tag in tags {
            groups.push((
                tag.name.clone(),
                tag.color.as_deref().and_then(parse_hex_color),
                Vec::new(),
            ));
        }
        // Catch-all for members below the lowest defined tag (per the MSC,
        // undefined levels fall to the nearest LOWER tag; below every tag
        // there's nothing to fall to).
        groups.push(("Member".to_string(), None, Vec::new()));
        for member in members {
            // `tags` is sorted highest-first: the first tag at or below the
            // member's level is theirs.
            let index =
                tags.iter().position(|tag| member.power_level >= tag.level).unwrap_or(tags.len());
            groups[index].2.push(member);
        }
    }

    let mut col = column![].spacing(2);
    col = col.push(
        container(
            text(format!("{} Members", members.len()))
                .size(12)
                .font(crate::theme::SEMIBOLD_FONT),
        )
        .padding([4, 4]),
    );
    // No per-group sort here: `member_candidates` arrives pre-sorted
    // (case-insensitive, see RoomMembersUpdated in update.rs) and groups are
    // filled in iteration order — sorting per view call allocated a fresh
    // lowercase String per comparison, every frame, for the whole roster.
    for (label, color, group) in groups {
        if group.is_empty() {
            continue;
        }

        let header = text(label).size(11).font(crate::theme::SEMIBOLD_FONT);
        let header: Element<'a, Message> = match color {
            Some(color) => header.style(colored_text(color)).into(),
            None => header.style(text::secondary).into(),
        };
        col = col.push(container(header).padding([6, 4]));

        for member in group {
            let name = text(member.display_name.clone()).size(13);
            let name: Element<'a, Message> = match color {
                Some(color) => name.style(colored_text(color)).into(),
                None => name.into(),
            };
            let is_selected = selected_member == Some(member.user_id.as_str());
            let style = if is_selected {
                crate::theme::selected_ghost_button
            } else {
                crate::theme::ghost_button
            };
            let row_button = button(
                row![
                    crate::media_cache::avatar(
                        media,
                        member.avatar_url.as_deref(),
                        &member.display_name,
                        22,
                    ),
                    name,
                ]
                .spacing(6)
                .align_y(iced::Center),
            )
            .on_press(Message::MemberClicked(member.user_id.clone()))
            .style(style)
            .width(Length::Fill)
            .padding([3, 6]);
            // Wrap in a mouse_area so right-click opens the actions flyout;
            // the inner button still handles the left-click highlight.
            col = col.push(
                iced::widget::mouse_area(row_button)
                    .on_right_press(Message::MemberRightClicked(member.user_id.clone())),
            );
        }
    }

    let panel = container(
        scrollable(col)
            .height(Length::Fill)
            .direction(iced::widget::scrollable::Direction::Vertical(
                iced::widget::scrollable::Scrollbar::new().width(6).scroller_width(6),
            ))
            .style(crate::theme::thin_scrollbar),
    )
    .width(Length::Fixed(200.0))
    .height(Length::Fill)
    .padding(6)
    .style(crate::theme::panel);

    // Report the cursor's roster-local position (the panel's top-left lines up
    // with the timeline stack's, so this Y works directly as the flyout
    // anchor). Wrapping doesn't steal events — the inner buttons/scrollbar
    // still work.
    iced::widget::mouse_area(panel).on_move(Message::MemberCursorMoved).into()
}

/// Local (client-side) search over the already-loaded timeline: matches
/// message text, image/file captions and names, and sender names. Doesn't
/// paginate further back — it filters what's on screen.
/// Recomputes which item indices match the active search — called on query
/// edits, search toggles, and timeline snapshots, NOT per frame:
/// `item_matches` lowercases every body and sender name, far too heavy for
/// view(). Empties the set when no search is active.
pub fn recompute_search_matches(state: &mut State) {
    state.search_matches.clear();
    let query = state.search_query.trim().to_lowercase();
    if !state.search_open || query.is_empty() {
        return;
    }
    for (index, item) in state.items.iter().enumerate() {
        if item_matches(item, &query) {
            state.search_matches.insert(index);
        }
    }
}

fn item_matches(item: &TimelineItem, query_lower: &str) -> bool {
    let content_text = match &item.content {
        TimelineItemContent::Text(body) => Some(body.as_str()),
        TimelineItemContent::Image { caption, .. } => caption.as_deref(),
        TimelineItemContent::Sticker { body, .. } => Some(body.as_str()),
        TimelineItemContent::File { filename, .. } => Some(filename.as_str()),
        TimelineItemContent::Redacted
        | TimelineItemContent::DateDivider(_)
        | TimelineItemContent::NewMessagesDivider => None,
    };
    if content_text.is_some_and(|t| t.to_lowercase().contains(query_lower)) {
        return true;
    }
    item.sender_display_name
        .as_deref()
        .unwrap_or(&item.sender)
        .to_lowercase()
        .contains(query_lower)
}

fn header<'a>(
    state: &'a State,
    room: Option<&'a RoomSummary>,
    notification_mode: Option<NotificationMode>,
    calls: &'a crate::screens::call::State,
    open_room_id: &str,
) -> Element<'a, Message> {
    let mut bar = row![].spacing(10).align_y(iced::Center).width(Length::Fill);

    match room {
        Some(room) => {
            let mut title = row![].spacing(6).align_y(iced::Center);
            title = title.push(text(room.name.clone()).size(15));
            if room.is_encrypted {
                title = title.push(
                    tooltip(
                        text("🔒").size(12).font(crate::theme::EMOJI_FONT),
                        container(text("End-to-end encrypted").size(12)).padding(6),
                        tooltip::Position::Bottom,
                    ),
                );
            }
            bar = bar.push(title);
            if let Some(topic) = &room.topic {
                let short: String = topic.chars().take(80).collect();
                bar = bar.push(
                    text(if short.len() < topic.len() { format!("{short}…") } else { short })
                        .size(12)
                        .style(text::secondary)
                        .width(Length::Fill),
                );
            } else {
                bar = bar.push(iced::widget::horizontal_space());
            }
        }
        None => {
            bar = bar.push(iced::widget::horizontal_space());
        }
    }

    // Start a call from the header only while none exists — once one is
    // active the banner below owns Join/Leave. Always a slot: the condition
    // flips on remote CallStateUpdated events, and a conditional push would
    // shift the positional widget state of everything after it (the open
    // notification dropdown would snap shut when someone starts a call).
    bar = bar.push(crate::theme::slot((!calls.has_active_call(open_room_id)).then(|| {
        tooltip(
            button(text(crate::theme::icon::CALL).font(crate::theme::ICON_FONT).size(12))
                .on_press_maybe(
                    (!calls.pending_for(open_room_id)).then_some(Message::JoinCallClicked),
                )
                .style(crate::theme::ghost_button)
                .padding([4, 8]),
            container(text("Start call").size(12)).padding(6).style(crate::theme::panel),
            tooltip::Position::Bottom,
        )
        .into()
    })));
    // Notification mode: a bell that reflects mute at a glance and opens the
    // mode menu (rendered as a slot below the header) on click.
    let bell = if NotifyChoice::from_mode(notification_mode) == NotifyChoice::Mute {
        crate::theme::icon::NOTIFY_MUTED
    } else {
        crate::theme::icon::NOTIFY
    };
    bar = bar.push(
        button(crate::theme::icon_text(bell, 14))
            .on_press(Message::ToggleNotifyMenu)
            .style(crate::theme::ghost_button)
            .padding([4, 8]),
    );
    let search_icon =
        if state.search_open { crate::theme::icon::CLOSE } else { crate::theme::icon::SEARCH };
    bar = bar.push(
        button(crate::theme::icon_text(search_icon, 14))
            .on_press(Message::ToggleSearch)
            .style(crate::theme::ghost_button)
            .padding([4, 8]),
    );
    bar = bar.push(
        button(crate::theme::icon_text(crate::theme::icon::MEMBERS, 14))
            .on_press(Message::ToggleMembers)
            .style(crate::theme::ghost_button)
            .padding([4, 8]),
    );

    container(bar).padding([8, 12]).style(crate::theme::panel).into()
}

/// O(1) roster lookup via the index map maintained next to
/// `member_candidates`.
fn member_by_id<'a>(
    members: &'a [RoomMember],
    member_index: &HashMap<String, usize>,
    user_id: &str,
) -> Option<&'a RoomMember> {
    member_index.get(user_id).and_then(|&i| members.get(i))
}

fn typing_line(state: &State) -> Option<Element<'_, composer::Message>> {
    if state.typing_users.is_empty() {
        return None;
    }

    let names: Vec<String> = state
        .typing_users
        .iter()
        .map(|user_id| {
            member_by_id(&state.composer.member_candidates, &state.member_index, user_id)
                .map(|m| m.display_name.clone())
                .unwrap_or_else(|| user_id.clone())
        })
        .collect();

    let label = match names.as_slice() {
        [a] => format!("{a} is typing…"),
        [a, b] => format!("{a} and {b} are typing…"),
        _ => "Several people are typing…".to_string(),
    };
    Some(text(label).size(12).into())
}

/// Mini avatars of everyone whose read receipt sits on the newest message
/// — Cinny's "following the conversation" row. Hover a face for the name.
fn follower_avatars<'a>(
    items: &'a [TimelineItem],
    own_user_id: Option<&'a str>,
    members: &'a [RoomMember],
    member_index: &HashMap<String, usize>,
    media: &'a crate::media_cache::State,
) -> Element<'a, composer::Message> {
    const SHOWN: usize = 8;
    let read_by: &[String] = items
        .iter()
        .rev()
        .find(|item| item.event_id.is_some() && !item.read_by.is_empty())
        .map(|item| item.read_by.as_slice())
        .unwrap_or(&[]);

    let mut avatars = row![].spacing(2).align_y(iced::Center);
    let mut shown = 0usize;
    let mut extra = 0usize;
    for user_id in read_by {
        if own_user_id == Some(user_id.as_str()) {
            continue;
        }
        if shown >= SHOWN {
            extra += 1;
            continue;
        }
        let member = member_by_id(members, member_index, user_id);
        let (name, avatar_url) = member
            .map(|m| (m.display_name.as_str(), m.avatar_url.as_deref()))
            .unwrap_or((user_id.as_str(), None));
        avatars = avatars.push(tooltip(
            crate::media_cache::avatar::<composer::Message>(media, avatar_url, name, 16),
            container(text(format!("{name} is following the conversation")).size(11))
                .padding(4)
                .style(crate::theme::panel),
            tooltip::Position::Top,
        ));
        shown += 1;
    }
    if extra > 0 {
        avatars = avatars.push(text(format!("+{extra}")).size(10).style(text::secondary));
    }
    avatars.into()
}

#[allow(clippy::too_many_arguments)]
fn render_item<'a>(
    item: &'a TimelineItem,
    first_url: Option<&'a str>,
    own_user_id: Option<&'a str>,
    show_header: bool,
    hovered: Option<&'a str>,
    editing: &'a Option<EditingState>,
    confirm_delete: &'a Option<String>,
    highlighted: Option<&'a str>,
    highlighted_member: Option<&'a str>,
    members: &'a [RoomMember],
    member_index: &'a HashMap<String, usize>,
    power_tags: &'a [client_core::events::PowerLevelTag],
    media: &'a crate::media_cache::State,
    shortcode_index: &'a HashMap<String, client_core::events::CustomEmoji>,
    url_previews: &'a HashMap<String, Option<UrlPreview>>,
    tweet_previews: &'a HashMap<String, Option<crate::tweets::TweetData>>,
    steam_previews: &'a HashMap<String, Option<crate::steam::SteamAppData>>,
) -> Element<'a, Message> {
    if let TimelineItemContent::DateDivider(date) = &item.content {
        // The SDK places date dividers at *local* day boundaries, but the
        // pre-formatted label from client-core is UTC math — for a non-UTC
        // user, messages between local midnight and UTC midnight would be
        // filed under the previous day's label (two identical consecutive
        // dividers). Format from the divider's timestamp in local time; the
        // UTC string stays as the fallback.
        use chrono::TimeZone;
        let label = match chrono::Local.timestamp_millis_opt(item.timestamp_ms as i64) {
            chrono::LocalResult::Single(dt) | chrono::LocalResult::Ambiguous(dt, _) => {
                dt.format("%Y-%m-%d").to_string()
            }
            chrono::LocalResult::None => date.clone(),
        };
        return container(text(format!("— {label} —")).size(12)).width(Length::Fill).into();
    }
    if matches!(item.content, TimelineItemContent::NewMessagesDivider) {
        return container(text("— new messages —").size(12).style(text::danger))
            .width(Length::Fill)
            .center_x(Length::Fill)
            .into();
    }
    let sender = item.sender_display_name.as_deref().unwrap_or(&item.sender);
    let is_own = own_user_id.is_some_and(|id| id == item.sender);
    // Grouped follow-up messages keep the avatar column's width so bodies
    // stay aligned, without repeating the picture.
    let avatar: Element<'a, Message> = if show_header {
        crate::media_cache::avatar(media, item.sender_avatar_url.as_deref(), sender, 42)
    } else {
        iced::widget::Space::with_width(42).into()
    };

    let body_line: Element<'a, Message> = match &item.content {
        TimelineItemContent::Text(body) => render_text_body(body, shortcode_index, media),
        TimelineItemContent::Image { url, caption, width, height } => {
            // The row's footprint is fixed *before* the bytes arrive —
            // sender-declared dimensions when the event carries them,
            // a standard box otherwise — so a finishing download can
            // never reflow the list and shove the scroll position.
            let (box_w, box_h) = image_display_size(*width, *height);
            let visual: Element<'a, Message> =
                match crate::media_cache::mxc_visual(media, url, box_w, Some(box_h)) {
                    Some(img) => iced::widget::mouse_area(img)
                        .on_press(Message::ZoomImage(url.clone()))
                        .interaction(iced::mouse::Interaction::Pointer)
                        .into(),
                    None => container(text("loading image…").size(12).style(text::secondary))
                        .center_x(Length::Fixed(box_w as f32))
                        .center_y(Length::Fixed(box_h as f32))
                        .style(crate::theme::panel)
                        .into(),
                };
            match caption {
                Some(c) => column![visual, text(c.clone()).size(12)].spacing(2).into(),
                None => visual,
            }
        }
        TimelineItemContent::Sticker { url, width, height, .. } => {
            // Like an inline image, but smaller and with no caption/zoom.
            // Footprint fixed before the bytes arrive so a finishing fetch
            // can't reflow the list (same reasoning as `Image`).
            let (box_w, box_h) = sticker_display_size(*width, *height);
            match crate::media_cache::mxc_visual(media, url, box_w, Some(box_h)) {
                Some(img) => img,
                None => container(text("loading sticker…").size(12).style(text::secondary))
                    .center_x(Length::Fixed(box_w as f32))
                    .center_y(Length::Fixed(box_h as f32))
                    .style(crate::theme::panel)
                    .into(),
            }
        }
        TimelineItemContent::File { filename, .. } => text(format!("[file: {filename}]")).size(15).into(),
        // Rendered through the normal path (not an early return) so the
        // avatar gutter, header, and sender grouping stay correct — the
        // grouping loop counts a redacted item as this sender's, so a bare
        // stub would leave the sender's next message header-less under the
        // previous author.
        TimelineItemContent::Redacted => {
            text("(message removed)").size(14).style(text::secondary).into()
        }
        TimelineItemContent::DateDivider(_) | TimelineItemContent::NewMessagesDivider => {
            unreachable!()
        }
    };

    // Sender names are colored by the sender's power-level group — the server's
    // "Red team", "Purple team", "Founder", … the MSC3949 tag colors Cinny
    // renders — resolved from the roster's power level. Falls back to a
    // stable per-user hash color when the sender has no colored tag (or the
    // room defines none), which is also what Cinny does for untagged users.
    let name_color = member_by_id(members, member_index, &item.sender)
        .and_then(|member| tag_color_for_level(power_tags, member.power_level))
        .unwrap_or_else(|| name_palette_color(&item.sender));
    let styled_name = || {
        text(sender.to_string())
            .size(13)
            .font(crate::theme::SEMIBOLD_FONT)
            .style(colored_text(name_color))
    };
    let sender_name: Element<'a, Message> = match &item.shield {
        Some(shield) => {
            let (glyph, message) = match shield {
                TrustShield::Red(message) => ("⚠", message.clone()),
                TrustShield::Grey(message) => ("ⓘ", message.clone()),
            };
            tooltip(
                row![styled_name(), text(glyph).size(13).font(crate::theme::EMOJI_FONT)].spacing(4),
                container(text(message).size(12)).padding(6),
                tooltip::Position::Bottom,
            )
            .into()
        }
        None => styled_name().into(),
    };
    let mut block = column![].spacing(2);
    if show_header {
        let sender_line: Element<'a, Message> = row![
            sender_name,
            text(format_timestamp(item.timestamp_ms)).size(11).style(text::secondary),
        ]
        .spacing(8)
        .align_y(iced::Center)
        .into();
        block = block.push(sender_line);
    }

    if let Some(reply) = &item.in_reply_to {
        let who = if reply.sender.is_empty() { "…".to_string() } else { reply.sender.clone() };
        let who_line = row![
            text(crate::theme::icon::REPLY).size(11).font(crate::theme::ICON_FONT).style(text::primary),
            text(who).size(11).style(text::primary),
        ]
        .spacing(4)
        .align_y(iced::Center);
        let texts = column![
            who_line,
            text(reply.snippet.clone()).size(12).style(text::secondary),
        ]
        .spacing(1);
        let mut quote = row![].spacing(6).align_y(iced::Center);
        if let Some(thumb) = reply
            .image_url
            .as_deref()
            .and_then(|url| crate::media_cache::mxc_visual(media, url, 36, Some(36)))
        {
            quote = quote.push(thumb);
        }
        quote = quote.push(texts);
        // The whole quote is a button: click to jump to the quoted post.
        block = block.push(
            button(container(quote).padding([3, 8]).style(crate::theme::panel))
                .on_press(Message::JumpToEvent(reply.event_id.clone()))
                .style(crate::theme::ghost_button)
                .padding(0),
        );
    }

    let Some(event_id) = &item.event_id else {
        // Local echo without an event id yet (still sending) — no edit/delete/react affordances.
        return with_avatar(avatar, block.push(body_line).into());
    };

    if editing.as_ref().is_some_and(|e| &e.event_id == event_id) {
        let draft = &editing.as_ref().unwrap().draft;
        block = block.push(
            text_input("Edit message", draft)
                .on_input(Message::EditDraftChanged)
                .on_submit(Message::ConfirmEdit)
                .padding(6),
        );
        block = block.push(
            row![
                button(text("Save").size(13)).on_press(Message::ConfirmEdit).padding(4),
                button(text("Cancel").size(13))
                    .on_press(Message::CancelEdit)
                    .style(crate::theme::ghost_button)
                    .padding(4),
            ]
            .spacing(6),
        );
        return with_avatar(avatar, block.into());
    }

    block = block.push(body_line);

    // Preview card for the message's first link: a playable card for
    // YouTube/Vimeo/Dailymotion/Rumble/Kick videos, a rich FxTwitter card
    // for tweets, otherwise the homeserver's OpenGraph card once it
    // resolves. The URL itself comes precomputed per snapshot
    // (`State::first_urls`) — running linkify here cost a scan + String
    // alloc per URL-bearing message per view rebuild.
    if matches!(item.content, TimelineItemContent::Text(_)) {
        if let Some(url) = first_url {
            if let Some(video) = crate::video_player::video_in(url) {
                let preview = url_previews.get(url).and_then(|p| p.as_ref());
                block = block.push(embed_video_card(video, preview, media));
            } else if let Some(Some(tweet)) = tweet_previews.get(url) {
                block = block.push(real_tweet_card(tweet, url, media));
            } else if let Some(Some(app_data)) = steam_previews.get(url) {
                block = block.push(steam_card(app_data, url, media));
            } else if let Some(Some(preview)) = url_previews.get(url) {
                if preview.title.is_some()
                    || preview.description.is_some()
                    || preview.image_mxc.is_some()
                {
                    block = block.push(preview_card(preview, media));
                }
            }
        }
    }

    if let Some(count) = item.thread_reply_count {
        block = block.push(text(format!("{count} repl{}", if count == 1 { "y" } else { "ies" })).size(12));
    }

    if !item.reactions.is_empty() {
        let mut pills = row![].spacing(4);
        for reaction in &item.reactions {
            let visual = resolve_reaction_visual(&reaction.key, media, shortcode_index);
            let count = text(reaction.count.to_string()).size(12).style(if reaction.reacted_by_me {
                text::primary
            } else {
                text::secondary
            });
            let pill = button(row![visual, count].spacing(4).align_y(iced::Center))
                .on_press(Message::ReactWithEmoji { event_id: event_id.clone(), key: reaction.key.clone() })
                .style(crate::theme::ghost_button)
                .padding([2, 4]);
            pills = pills.push(tooltip(
                pill,
                container(text(reaction_senders_label(&reaction.senders, members, member_index)).size(12))
                    .padding(6)
                    .style(crate::theme::panel),
                tooltip::Position::Bottom,
            ));
        }
        block = block.push(pills);
    }

    if is_own && !item.read_by.is_empty() {
        block = block.push(text(format!("Read by {}", item.read_by.len())).size(11));
    }

    if confirm_delete.as_deref() == Some(event_id.as_str()) {
        block = block.push(
            row![
                text("Delete this message?").size(12),
                button(text("Yes").size(13))
                    .on_press(Message::ConfirmDelete(event_id.clone()))
                    .style(button::danger)
                    .padding(4),
                button(text("No").size(13))
                    .on_press(Message::CancelDelete)
                    .style(crate::theme::ghost_button)
                    .padding(4),
            ]
            .spacing(6)
            .align_y(iced::Center),
        );
    }

    let rendered = with_avatar(avatar, block.into());

    // Cinny-style floating action bar: only while hovered, overlaid at the
    // message's top-right so nothing shifts, outline glyphs not colored
    // emoji.
    let is_hovered = hovered == Some(event_id.as_str());
    let bar_active = is_hovered
        && editing.as_ref().map(|e| e.event_id.as_str()) != Some(event_id.as_str())
        && confirm_delete.as_deref() != Some(event_id.as_str());
    let bar_slot =
        crate::theme::slot(bar_active.then(|| hover_action_bar(item, event_id, sender, is_own)));
    let stacked: Element<'a, Message> = iced::widget::stack![
        rendered,
        container(bar_slot).width(Length::Fill).align_x(iced::Right),
    ]
    .into();
    let interactive: Element<'a, Message> = iced::widget::mouse_area(stacked)
        .on_enter(Message::ItemHovered(event_id.clone()))
        .on_exit(Message::ItemUnhovered(event_id.clone()))
        .into();

    if highlighted == Some(event_id.as_str()) {
        return container(interactive)
            .padding(4)
            .style(|theme: &iced::Theme| {
                let palette = theme.extended_palette();
                iced::widget::container::Style {
                    background: Some(palette.primary.weak.color.scale_alpha(0.35).into()),
                    border: iced::border::rounded(6),
                    ..iced::widget::container::Style::default()
                }
            })
            .into();
    }
    // Persistent highlight for a member "highlighted in chat" from the roster
    // menu: a clearly visible tinted card with an accent border (the earlier
    // near-transparent `secondary` tint was invisible in most themes). The
    // transient quote-jump flash above wins when both apply (it returns
    // first).
    if highlighted_member == Some(item.sender.as_str()) {
        return container(interactive)
            .padding(6)
            .width(Length::Fill)
            .style(|theme: &iced::Theme| {
                let palette = theme.extended_palette();
                iced::widget::container::Style {
                    background: Some(palette.primary.weak.color.scale_alpha(0.22).into()),
                    border: iced::Border {
                        color: palette.primary.strong.color,
                        width: 2.0,
                        radius: 6.into(),
                    },
                    ..iced::widget::container::Style::default()
                }
            })
            .into();
    }
    interactive
}

/// The floating actions themselves: react / reply (+ edit / delete on own
/// messages), monochrome outline glyphs with tooltips, in a small bordered
/// pill.
fn hover_action_bar<'a>(
    item: &'a TimelineItem,
    event_id: &str,
    sender: &str,
    is_own: bool,
) -> Element<'a, Message> {
    // Kept no taller than a single text line so hovering a grouped
    // (headerless) message doesn't grow the row and shift the layout.
    let icon = |glyph: &'static str, label: &'static str, message: Message| -> Element<'a, Message> {
        tooltip(
            button(text(glyph).size(11).font(crate::theme::ICON_FONT).style(text::secondary))
                .on_press(message)
                .style(crate::theme::ghost_button)
                .padding([1, 6]),
            container(text(label).size(11)).padding(4).style(crate::theme::panel),
            tooltip::Position::Top,
        )
        .into()
    };

    let mut bar = row![].spacing(1).align_y(iced::Center);
    bar = bar.push(icon(crate::theme::icon::REACT, "React", Message::ToggleReactionPicker(event_id.to_owned())));
    bar = bar.push(icon(
        crate::theme::icon::REPLY,
        "Reply",
        Message::StartReply(client_core::events::ReplyPreview {
            event_id: event_id.to_owned(),
            sender: sender.to_string(),
            snippet: ui_snippet(&item.content),
            image_url: match &item.content {
                TimelineItemContent::Image { url, .. }
                | TimelineItemContent::Sticker { url, .. } => Some(url.clone()),
                _ => None,
            },
        }),
    ));
    if is_own {
        if let TimelineItemContent::Text(body) = &item.content {
            bar = bar.push(icon(
                crate::theme::icon::EDIT,
                "Edit",
                Message::StartEdit { event_id: event_id.to_owned(), current_body: body.clone() },
            ));
        }
        bar = bar.push(icon(crate::theme::icon::DELETE, "Delete", Message::RequestDelete(event_id.to_owned())));
    }

    container(bar)
        .padding(1)
        .style(|theme: &iced::Theme| {
            let palette = theme.extended_palette();
            iced::widget::container::Style {
                background: Some(palette.background.weak.color.into()),
                border: iced::Border {
                    color: palette.background.strong.color,
                    width: 1.0,
                    radius: 8.into(),
                },
                ..iced::widget::container::Style::default()
            }
        })
        .into()
}

/// Stable per-user display-name color from an 8-color palette keyed by a
/// hash of the user id — the scheme Cinny and Element use, tuned to stay
/// readable on both themes.
fn name_palette_color(user_id: &str) -> iced::Color {
    const PALETTE: [(u8, u8, u8); 8] = [
        (0xE5, 0x73, 0x73), // red
        (0xFF, 0xB7, 0x4D), // orange
        (0xFF, 0xD5, 0x4F), // amber
        (0xAE, 0xD5, 0x81), // green
        (0x4D, 0xD0, 0xE1), // cyan
        (0x64, 0xB5, 0xF6), // blue
        (0xBA, 0x68, 0xC8), // purple
        (0xF0, 0x62, 0x92), // pink
    ];
    let hash: u32 = user_id.bytes().fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    let (r, g, b) = PALETTE[(hash % PALETTE.len() as u32) as usize];
    iced::Color::from_rgb8(r, g, b)
}

/// A `text` style function that pins a fixed color — used to tint sender
/// names, member rows and group headers in their power-level group color.
fn colored_text(color: iced::Color) -> impl Fn(&iced::Theme) -> text::Style {
    move |_: &iced::Theme| text::Style { color: Some(color) }
}

/// Parse a CSS-style hex color (`#rgb`, `#rrggbb`, or `#rrggbbaa`) into an
/// iced [`Color`](iced::Color). Returns `None` for anything malformed so
/// callers fall back to the hash palette rather than rendering a wrong or
/// invisible color. the server's tags use `#rrggbb`, but the others are cheap to
/// accept and match what any web/Cinny-authored value could carry.
fn parse_hex_color(hex: &str) -> Option<iced::Color> {
    let hex = hex.trim().strip_prefix('#')?;
    // Guard char-boundary slicing below: a stray multibyte char would panic.
    if !hex.is_ascii() {
        return None;
    }
    let pair = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    match hex.len() {
        // `#rgb` shorthand: each nibble is doubled (`f` → `0xff`).
        3 => {
            let nib = |i: usize| u8::from_str_radix(&hex[i..=i], 16).ok().map(|n| n * 17);
            Some(iced::Color::from_rgb8(nib(0)?, nib(1)?, nib(2)?))
        }
        6 => Some(iced::Color::from_rgb8(pair(0)?, pair(2)?, pair(4)?)),
        8 => Some(iced::Color::from_rgba8(pair(0)?, pair(2)?, pair(4)?, pair(6)? as f32 / 255.0)),
        _ => None,
    }
}

/// The group color for a member at `power_level`, per the room's MSC3949
/// power-level tags (sorted highest-level first): the color of the first tag
/// at or below the member's level — the same tag the member panel files them
/// under. `None` when no tag applies or the matching tag defined no color, in
/// which case callers fall back to the per-user hash palette.
fn tag_color_for_level(
    tags: &[client_core::events::PowerLevelTag],
    power_level: i64,
) -> Option<iced::Color> {
    tags.iter()
        .find(|tag| power_level >= tag.level)
        .and_then(|tag| tag.color.as_deref())
        .and_then(parse_hex_color)
}

/// Link preview card. Tweet links get a dedicated layout (author line,
/// tweet text as the main content, media below — the shape Discord/Element
/// use); everything else gets the generic thumbnail + title + description
/// card. Clicking either opens the URL.
fn preview_card<'a>(
    preview: &'a UrlPreview,
    media: &'a crate::media_cache::State,
) -> Element<'a, Message> {
    let inner: Element<'a, Message> = match tweet_from_preview(preview) {
        Some(tweet) => tweet_card(tweet, preview, media),
        None => generic_preview(preview, media),
    };

    // The Discord-style accent strip on the left edge is an accent-colored
    // underlay showing through 3px of left padding — a `Length::Fill`-tall
    // bar widget would panic inside the vertical scrollable ("scrollable
    // content must not fill its vertical scrolling axis").
    button(
        container(container(inner).padding(8).style(crate::theme::panel))
            .padding(iced::Padding { top: 0.0, right: 0.0, bottom: 0.0, left: 3.0 })
            .max_width(480)
            .style(|theme: &iced::Theme| iced::widget::container::Style {
                background: Some(theme.extended_palette().primary.strong.color.into()),
                border: iced::border::rounded(3),
                ..iced::widget::container::Style::default()
            }),
    )
    .on_press(Message::OpenUrl(preview.url.clone()))
    .style(crate::theme::ghost_button)
    .padding(0)
    .into()
}

/// Video-platform link card: OG title/thumbnail (from the homeserver's
/// preview proxy, once resolved) under a centered play badge, with a
/// platform-tinted accent strip. Clicking starts the in-app embedded player
/// overlay — the link never opens a browser. Renders even before/without OG
/// data since the video id alone is enough to play.
fn embed_video_card<'a>(
    video: crate::video_player::EmbedVideo,
    preview: Option<&'a UrlPreview>,
    media: &'a crate::media_cache::State,
) -> Element<'a, Message> {
    let title = preview.and_then(|p| p.title.clone());
    let platform = video.platform;

    let mut details =
        column![text(platform.label()).size(11).style(text::secondary)].spacing(2);
    if let Some(title) = title.clone() {
        details = details.push(text(title).size(13).font(crate::theme::SEMIBOLD_FONT));
    }

    let thumb: Element<'a, Message> = preview
        .and_then(|p| p.image_mxc.as_deref())
        .and_then(|mxc| crate::media_cache::mxc_visual(media, mxc, 360, None))
        .unwrap_or_else(|| {
            // No thumbnail (yet) — a dark 16:9 stage keeps the card's shape.
            container(iced::widget::Space::new(360, 202))
                .style(|_theme: &iced::Theme| iced::widget::container::Style {
                    background: Some(iced::Color::from_rgb8(0x10, 0x10, 0x10).into()),
                    border: iced::border::rounded(4),
                    ..iced::widget::container::Style::default()
                })
                .into()
        });

    let play_badge = container(
        text(crate::theme::icon::PLAY)
            .font(crate::theme::ICON_FONT)
            .size(22)
            .color(iced::Color::WHITE),
    )
    .padding([10, 14])
    .style(|_theme: &iced::Theme| iced::widget::container::Style {
        background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.65).into()),
        border: iced::border::rounded(999),
        ..iced::widget::container::Style::default()
    });

    let visual = iced::widget::stack![thumb, iced::widget::center(play_badge)];
    let card = column![details, visual].spacing(6);

    button(
        container(container(card).padding(8).style(crate::theme::panel))
            .padding(iced::Padding { top: 0.0, right: 0.0, bottom: 0.0, left: 3.0 })
            .max_width(480)
            .style(move |_theme: &iced::Theme| iced::widget::container::Style {
                // Platform-tinted accent strip (same construction as `preview_card`).
                background: Some(platform.accent().into()),
                border: iced::border::rounded(3),
                ..iced::widget::container::Style::default()
            }),
    )
    .on_press(Message::PlayVideo { video, title })
    .style(crate::theme::ghost_button)
    .padding(0)
    .into()
}

fn generic_preview<'a>(
    preview: &'a UrlPreview,
    media: &'a crate::media_cache::State,
) -> Element<'a, Message> {
    let mut details = column![].spacing(2);
    if let Some(site) = &preview.site_name {
        details = details.push(text(site.clone()).size(11).style(text::secondary));
    }
    if let Some(title) = &preview.title {
        details = details.push(text(title.clone()).size(13).font(crate::theme::SEMIBOLD_FONT));
    }
    if let Some(description) = &preview.description {
        let flat = description.replace('\n', " ");
        let shown = if flat.chars().count() > 180 {
            format!("{}…", flat.chars().take(180).collect::<String>())
        } else {
            flat
        };
        details = details.push(text(shown).size(12).style(text::secondary));
    }

    let mut card = row![].spacing(8).align_y(iced::Center);
    if let Some(thumb) = preview
        .image_mxc
        .as_deref()
        .and_then(|mxc| crate::media_cache::mxc_visual(media, mxc, 72, None))
    {
        card = card.push(thumb);
    }
    card = card.push(details);
    card.into()
}

/// A tweet reconstructed from X's OpenGraph fields: `og:title` is the
/// author's display name and `og:description` is
/// `tweet text — Display Name (@handle) Month Day, Year` (with the exact
/// dash and spacing varying by scrape — X pads with non-breaking and
/// narrow spaces).
struct TweetPreview {
    author: String,
    handle_line: Option<String>,
    text: String,
}

fn tweet_from_preview(preview: &UrlPreview) -> Option<TweetPreview> {
    let host = preview
        .url
        .split_once("://")
        .map(|(_, rest)| rest)?
        .split(['/', '?'])
        .next()?
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    let tweet_host = matches!(
        host.as_str(),
        "x.com" | "twitter.com" | "fxtwitter.com" | "vxtwitter.com" | "fixupx.com"
    );
    if !tweet_host {
        return None;
    }

    // Normalize exotic whitespace before parsing.
    let description: String = preview
        .description
        .as_deref()?
        .chars()
        .map(|c| match c {
            '\u{a0}' | '\u{202f}' | '\u{2009}' | '\u{2007}' => ' ',
            c => c,
        })
        .collect();

    // Anchor on the "(@handle)" pattern rather than the dash — the most
    // stable part of the suffix.
    let parsed = (|| {
        let at = description.rfind("(@")?;
        let close = at + description[at..].find(')')?;
        let handle = description[at + 2..close].trim().to_string();
        let date = description[close + 1..].trim().to_string();

        let head = description[..at].trim_end();
        // The author's display name sits between the last dash separator
        // and "(@"; names may contain hyphens, so require the separator to
        // be a spaced-out dash. Multi-line tweets put the attribution on
        // its own line, so a newline also counts as the leading space.
        let separators = [
            " — ", " – ", " ― ", " - ", "\n— ", "\n– ", "\n― ", "\n- ",
        ];
        let (text_end, author_start) = separators
            .iter()
            .filter_map(|sep| head.rfind(sep).map(|i| (i, i + sep.len())))
            .max_by_key(|(i, _)| *i)?;
        let author = head[author_start..].trim().to_string();
        let text = strip_trailing_tco(head[..text_end].trim()).to_string();
        let handle_line = if date.is_empty() {
            format!("@{handle}")
        } else {
            format!("@{handle} · {date}")
        };
        Some(TweetPreview { author, handle_line: Some(handle_line), text })
    })();

    // Suffix didn't parse — still render tweet-shaped with what's certain:
    // og:title is the author, the description is the tweet text.
    Some(parsed.unwrap_or_else(|| TweetPreview {
        author: preview.title.clone().unwrap_or_else(|| "X".to_string()),
        handle_line: None,
        text: strip_trailing_tco(description.trim()).to_string(),
    }))
}

/// X appends the media's shortlink(s) to the tweet text — `t.co` or
/// `pic.twitter.com`/`pic.x.com`, scheme optional depending on the scrape.
/// Noise once the media renders as an actual image.
fn strip_trailing_tco(text: &str) -> &str {
    fn is_media_shortlink(word: &str) -> bool {
        let word = word.strip_prefix("https://").unwrap_or(word);
        word.starts_with("t.co/")
            || word.starts_with("pic.twitter.com/")
            || word.starts_with("pic.x.com/")
    }
    let mut current = text.trim_end();
    loop {
        match current.rsplit_once(char::is_whitespace) {
            Some((rest, last)) if is_media_shortlink(last) => {
                current = rest.trim_end();
            }
            None if is_media_shortlink(current) => return "",
            _ => return current,
        }
    }
}

fn tweet_card<'a>(
    tweet: TweetPreview,
    preview: &'a UrlPreview,
    media: &'a crate::media_cache::State,
) -> Element<'a, Message> {
    // X's og:image is the author's avatar for text-only tweets (square,
    // modest size) but the attached photo for media tweets — dimensions
    // tell them apart. Unknown dimensions are treated as an avatar so a
    // profile picture never renders huge.
    let is_avatar = match (preview.image_width, preview.image_height) {
        (Some(w), Some(h)) => w == h && w <= 500,
        _ => true,
    };

    let mut author_row = row![].spacing(8).align_y(iced::Center);
    if is_avatar {
        if let Some(avatar) = preview
            .image_mxc
            .as_deref()
            .and_then(|mxc| crate::media_cache::mxc_visual(media, mxc, 36, Some(36)))
        {
            author_row = author_row.push(avatar);
        }
    }
    let mut author_col =
        column![text(tweet.author).size(14).font(crate::theme::SEMIBOLD_FONT)].spacing(1);
    if let Some(handle_line) = tweet.handle_line {
        author_col = author_col.push(text(handle_line).size(12).style(text::secondary));
    }
    author_row = author_row.push(author_col);

    let mut card = column![author_row].spacing(6);
    if !tweet.text.is_empty() {
        card = card.push(text(tweet.text).size(14));
    }
    if !is_avatar {
        if let Some(media) = preview
            .image_mxc
            .as_deref()
            .and_then(|mxc| crate::media_cache::mxc_visual(media, mxc, 360, None))
        {
            card = card.push(media);
        }
    }
    card = card.push(text("X").size(11).style(text::secondary));
    card.into()
}

/// Discord-quality tweet card from FxTwitter data: avatar, author (with
/// verified mark), handle, tweet text, photo grid, quoted tweet,
/// timestamp plus views, and engagement counts — the closest iced gets
/// to the real thing. Clicking anywhere opens the tweet.
fn real_tweet_card<'a>(
    tweet: &'a crate::tweets::TweetData,
    url: &str,
    media: &'a crate::media_cache::State,
) -> Element<'a, Message> {
    let card = column![tweet_body(tweet, media, false)].spacing(6);

    button(
        container(container(card).padding(10).style(crate::theme::panel))
            .padding(iced::Padding { top: 0.0, right: 0.0, bottom: 0.0, left: 3.0 })
            .max_width(500)
            .style(|theme: &iced::Theme| iced::widget::container::Style {
                background: Some(theme.extended_palette().primary.strong.color.into()),
                border: iced::border::rounded(3),
                ..iced::widget::container::Style::default()
            }),
    )
    .on_press(Message::OpenUrl(url.to_string()))
    .style(crate::theme::ghost_button)
    .padding(0)
    .into()
}

/// The inner content of a tweet (also used, smaller, for quoted tweets).
fn tweet_body<'a>(
    tweet: &'a crate::tweets::TweetData,
    media: &'a crate::media_cache::State,
    is_quote: bool,
) -> Element<'a, Message> {
    let web_img = |url: &str, width: u16, height: Option<u16>| -> Option<Element<'a, Message>> {
        media.web_images.get(url).map(|handle| {
            let mut widget = image(handle.clone()).width(width);
            if let Some(height) = height {
                widget = widget.height(height);
            }
            widget.into()
        })
    };
    let (name_size, text_size, avatar_size, photo_width) =
        if is_quote { (13, 13, 24, 160) } else { (14, 14, 42, 215) };

    let mut author_row = row![].spacing(8).align_y(iced::Center);
    if let Some(avatar) = tweet
        .author
        .avatar_url
        .as_deref()
        .and_then(|u| web_img(u, avatar_size, Some(avatar_size)))
    {
        author_row = author_row.push(avatar);
    }
    let mut name_row = row![
        text(tweet.author.name.as_str()).size(name_size).font(crate::theme::SEMIBOLD_FONT)
    ]
    .spacing(4)
    .align_y(iced::Center);
    if tweet.author.is_verified() {
        name_row = name_row.push(text("✓").size(name_size - 1).style(text::primary));
    }
    let handle = text(format!("@{}", tweet.author.screen_name)).size(12).style(text::secondary);
    if is_quote {
        // Quotes keep the author on one compact line.
        author_row = author_row.push(row![name_row, handle].spacing(6).align_y(iced::Center));
    } else {
        author_row = author_row.push(column![name_row, handle].spacing(1));
    }

    let mut body = column![author_row].spacing(6);

    if !tweet.text.is_empty() {
        body = body.push(text(tweet.text.as_str()).size(text_size));
    }

    // Photos two-up like X's grid; video stills get a play marker.
    let photos = tweet.photo_urls();
    for pair in photos.chunks(2) {
        let mut media_row = row![].spacing(4);
        let width = if pair.len() == 1 && photos.len() == 1 { photo_width * 2 + 4 } else { photo_width };
        for photo in pair {
            if let Some(img) = web_img(photo, width, None) {
                media_row = media_row.push(img);
            }
        }
        body = body.push(media_row);
    }
    for thumbnail in tweet.video_thumbnail_urls() {
        if let Some(still) = web_img(thumbnail, photo_width * 2 + 4, None) {
            body = body.push(still);
            body = body.push(text("▶ video").size(11).style(text::secondary));
        }
    }

    if let Some(quote) = &tweet.quote {
        body = body.push(
            container(tweet_body(quote, media, true))
                .padding(8)
                .style(|theme: &iced::Theme| {
                    let palette = theme.extended_palette();
                    iced::widget::container::Style {
                        background: Some(palette.background.base.color.into()),
                        border: iced::border::rounded(8),
                        ..iced::widget::container::Style::default()
                    }
                }),
        );
    }

    if !is_quote {
        let mut footer = String::new();
        if let Some(ts) = tweet.created_timestamp {
            footer.push_str(&tweet_time(ts));
        }
        if let Some(views) = tweet.views {
            if !footer.is_empty() {
                footer.push_str(" · ");
            }
            footer.push_str(&format!("{} Views", compact_count(views)));
        }
        if !footer.is_empty() {
            body = body.push(text(footer).size(12).style(text::secondary));
        }

        let mut stats = Vec::new();
        if let Some(n) = tweet.replies {
            stats.push(format!("💬 {}", compact_count(n)));
        }
        if let Some(n) = tweet.retweets {
            stats.push(format!("🔁 {}", compact_count(n)));
        }
        if let Some(n) = tweet.likes {
            stats.push(format!("❤️ {}", compact_count(n)));
        }
        if !stats.is_empty() {
            body = body.push(
                text(stats.join("   "))
                    .size(12)
                    .font(crate::theme::EMOJI_FONT)
                    .style(text::secondary),
            );
        }
    }

    body.into()
}

/// Steam store card modeled on the store's own purchase widget: capsule
/// art on the left, short description, platform list, and live pricing
/// with the green discount badge and struck-through original price.
/// Rendered in Steam's fixed brand palette rather than the app theme —
/// the card is meant to read as a lifted piece of the store. Clicking
/// anywhere (including the "Buy on Steam" button, which is visual-only)
/// opens the store page.
fn steam_card<'a>(
    app_data: &'a crate::steam::SteamAppData,
    url: &str,
    media: &'a crate::media_cache::State,
) -> Element<'a, Message> {
    const CARD_BG: iced::Color = iced::Color::from_rgb(0x16 as f32 / 255.0, 0x20 as f32 / 255.0, 0x2D as f32 / 255.0);
    const TITLE: iced::Color = iced::Color::WHITE;
    const DESCRIPTION: iced::Color = iced::Color::from_rgb(0xC6 as f32 / 255.0, 0xD4 as f32 / 255.0, 0xDF as f32 / 255.0);
    const MUTED: iced::Color = iced::Color::from_rgb(0x7C as f32 / 255.0, 0x8B as f32 / 255.0, 0x94 as f32 / 255.0);
    const BADGE_BG: iced::Color = iced::Color::from_rgb(0x4C as f32 / 255.0, 0x6B as f32 / 255.0, 0x22 as f32 / 255.0);
    const BADGE_TEXT: iced::Color = iced::Color::from_rgb(0xBE as f32 / 255.0, 0xEE as f32 / 255.0, 0x11 as f32 / 255.0);
    const BUY_BG: iced::Color = iced::Color::from_rgb(0x5C as f32 / 255.0, 0x9E as f32 / 255.0, 0x2E as f32 / 255.0);

    let colored = |color: iced::Color| {
        move |_theme: &iced::Theme| iced::widget::text::Style { color: Some(color) }
    };

    let header_row = row![
        text(format!("Buy {}", app_data.name))
            .size(15)
            .font(crate::theme::SEMIBOLD_FONT)
            .style(colored(TITLE)),
        iced::widget::Space::with_width(Length::Fill),
        text("STEAM").size(11).font(crate::theme::SEMIBOLD_FONT).style(colored(MUTED)),
    ]
    .spacing(8)
    .align_y(iced::Center);

    // Capsule art keeps Steam's 460:215 header ratio at thumbnail size.
    let mut body_row = row![].spacing(10);
    if let Some(handle) =
        app_data.header_image.as_deref().and_then(|u| media.web_images.get(u))
    {
        body_row = body_row.push(image(handle.clone()).width(165));
    }
    if let Some(description) = &app_data.short_description {
        let flat = description.replace('\n', " ");
        let shown = if flat.chars().count() > 220 {
            format!("{}…", flat.chars().take(220).collect::<String>())
        } else {
            flat
        };
        body_row = body_row.push(
            column![text(shown).size(12).style(colored(DESCRIPTION))].width(Length::Fill),
        );
    }

    // Bottom strip: platforms on the left, discount badge + prices + the
    // buy button flush right — the store widget's purchase row.
    let mut purchase_row = row![].spacing(8).align_y(iced::Center);
    if let Some(platforms) = &app_data.platforms {
        let mut names = Vec::new();
        if platforms.windows {
            names.push("Windows");
        }
        if platforms.mac {
            names.push("macOS");
        }
        if platforms.linux {
            names.push("Linux");
        }
        if !names.is_empty() {
            purchase_row =
                purchase_row.push(text(names.join(" · ")).size(11).style(colored(MUTED)));
        }
    }
    purchase_row = purchase_row.push(iced::widget::Space::with_width(Length::Fill));

    let mut buy_label = "View on Steam";
    if let Some(price) = &app_data.price_overview {
        buy_label = "Buy on Steam";
        if price.discount_percent > 0 {
            purchase_row = purchase_row.push(
                container(
                    text(format!("-{}%", price.discount_percent))
                        .size(14)
                        .font(crate::theme::SEMIBOLD_FONT)
                        .style(colored(BADGE_TEXT)),
                )
                .padding([2, 6])
                .style(move |_theme: &iced::Theme| iced::widget::container::Style {
                    background: Some(BADGE_BG.into()),
                    border: iced::border::rounded(2),
                    ..iced::widget::container::Style::default()
                }),
            );
            if !price.initial_formatted.is_empty() {
                // Spans typed with `Message` as the link type — `Rich`
                // emits its link type as the message, so it must match
                // the tree even though these spans carry no links.
                let struck: [iced::widget::text::Span<'static, Message>; 1] = [
                    iced::widget::span(price.initial_formatted.clone())
                        .size(12)
                        .color(MUTED)
                        .strikethrough(true),
                ];
                purchase_row = purchase_row.push(iced::widget::rich_text(struck));
            }
        }
        purchase_row = purchase_row.push(
            text(price.final_formatted.clone())
                .size(14)
                .font(crate::theme::SEMIBOLD_FONT)
                .style(colored(TITLE)),
        );
    } else if app_data.is_free {
        buy_label = "Play on Steam";
        purchase_row = purchase_row
            .push(text("Free To Play").size(13).style(colored(TITLE)));
    } else if let Some(release) = &app_data.release_date {
        if release.coming_soon && !release.date.is_empty() {
            purchase_row = purchase_row
                .push(text(format!("Coming {}", release.date)).size(12).style(colored(MUTED)));
        }
    }
    purchase_row = purchase_row.push(
        container(
            text(buy_label).size(13).font(crate::theme::SEMIBOLD_FONT).style(colored(TITLE)),
        )
        .padding([5, 12])
        .style(move |_theme: &iced::Theme| iced::widget::container::Style {
            background: Some(BUY_BG.into()),
            border: iced::border::rounded(2),
            ..iced::widget::container::Style::default()
        }),
    );

    let card = column![header_row, body_row, purchase_row].spacing(8);

    button(
        container(card)
            .padding(12)
            .max_width(480)
            .style(move |_theme: &iced::Theme| iced::widget::container::Style {
                background: Some(CARD_BG.into()),
                border: iced::border::rounded(6),
                ..iced::widget::container::Style::default()
            }),
    )
    .on_press(Message::OpenUrl(url.to_string()))
    .style(crate::theme::ghost_button)
    .padding(0)
    .into()
}

fn compact_count(n: u64) -> String {
    if n >= 1_000_000 {
        let v = n as f64 / 1_000_000.0;
        if v >= 10.0 { format!("{v:.0}M") } else { format!("{v:.1}M") }
    } else if n >= 1_000 {
        let v = n as f64 / 1_000.0;
        if v >= 10.0 { format!("{v:.0}K") } else { format!("{v:.1}K") }
    } else {
        n.to_string()
    }
}

fn tweet_time(timestamp_secs: i64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_opt(timestamp_secs, 0) {
        chrono::LocalResult::Single(dt) | chrono::LocalResult::Ambiguous(dt, _) => {
            dt.format("%H:%M · %b %e, %Y").to_string()
        }
        chrono::LocalResult::None => String::new(),
    }
}

fn format_timestamp(timestamp_ms: u64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_millis_opt(timestamp_ms as i64) {
        chrono::LocalResult::Single(dt) | chrono::LocalResult::Ambiguous(dt, _) => {
            dt.format("%H:%M").to_string()
        }
        chrono::LocalResult::None => String::new(),
    }
}

/// One-line summary of a message, used when quoting it in the composer's
/// reply banner.
fn ui_snippet(content: &TimelineItemContent) -> String {
    const LIMIT: usize = 90;
    match content {
        TimelineItemContent::Text(body) => {
            let flat = body.replace('\n', " ");
            if flat.chars().count() > LIMIT {
                format!("{}…", flat.chars().take(LIMIT).collect::<String>())
            } else {
                flat
            }
        }
        TimelineItemContent::Image { caption, .. } => match caption {
            Some(c) => format!("[image: {c}]"),
            None => "[image]".to_string(),
        },
        TimelineItemContent::Sticker { .. } => "[sticker]".to_string(),
        TimelineItemContent::File { filename, .. } => format!("[file: {filename}]"),
        TimelineItemContent::Redacted => "(message removed)".to_string(),
        TimelineItemContent::DateDivider(_) | TimelineItemContent::NewMessagesDivider => {
            String::new()
        }
    }
}

/// Message layout: avatar gutter on the left, everything else (sender line,
/// body, reactions, action row) stacked to its right.
fn with_avatar<'a>(
    avatar: Element<'a, Message>,
    content: Element<'a, Message>,
) -> Element<'a, Message> {
    row![container(avatar).padding([2, 0]), container(content).width(Length::Fill)]
        .spacing(8)
        .into()
}

/// A slice of a message body: literal text, a `:shortcode:` that resolved
/// against a known custom emoji pack, a real unicode emoji typed inline, or
/// a URL.
enum BodySegment<'a> {
    Text(&'a str),
    Emoji(&'a client_core::events::CustomEmoji),
    /// The emoji's own canonical, fully-qualified string (from the `emojis`
    /// crate's static table), not the raw slice matched in the body — those
    /// can differ (e.g. a sender's client omitting the FE0F variation
    /// selector), and the Twemoji disk cache / picker prefetch are both
    /// keyed by the canonical form.
    UnicodeEmoji(&'static str),
    Link(&'a str),
}

/// First URL in a piece of text, if any — the one a preview card is
/// requested for.
pub fn first_url_in(text: &str) -> Option<String> {
    // Cheap pre-filter: linkify's Url kind literally requires "://" after
    // the scheme (linkify url.rs: `starts_with("://")`), so its absence
    // means no URL — this runs per text item per view call, and skips the
    // scan for timestamp-style bodies ("meeting at 12:30") that a bare
    // colon check would let through.
    if !text.contains("://") {
        return None;
    }
    let mut finder = linkify::LinkFinder::new();
    finder.kinds(&[linkify::LinkKind::Url]);
    finder.links(text).next().map(|link| link.as_str().to_string())
}

/// Splits one line into text / custom-emoji / link segments. Links are
/// carved out first so a `:` inside a URL is never mistaken for a
/// shortcode delimiter.
fn tokenize_line<'a>(
    line: &'a str,
    shortcode_index: &'a HashMap<String, client_core::events::CustomEmoji>,
) -> Vec<BodySegment<'a>> {
    let mut finder = linkify::LinkFinder::new();
    finder.kinds(&[linkify::LinkKind::Url]);

    let mut segments = Vec::new();
    let mut cursor = 0;
    for link in finder.links(line) {
        if link.start() > cursor {
            tokenize_custom_emoji_into(&line[cursor..link.start()], shortcode_index, &mut segments);
        }
        segments.push(BodySegment::Link(link.as_str()));
        cursor = link.end();
    }
    if cursor < line.len() {
        tokenize_custom_emoji_into(&line[cursor..], shortcode_index, &mut segments);
    }
    segments
}

/// Splits `text` on `:shortcode:` tokens that match a pack emoji
/// (case-insensitive), appending segments to `out`.
fn tokenize_custom_emoji_into<'a>(
    text: &'a str,
    shortcode_index: &'a HashMap<String, client_core::events::CustomEmoji>,
    out: &mut Vec<BodySegment<'a>>,
) {
    fn is_shortcode_char(c: char) -> bool {
        c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+')
    }
    // Hash lookup (index is keyed lowercase, matching the old
    // eq_ignore_ascii_case scan) — the previous per-candidate linear scan
    // over every pack emoji ran per ':token:' per message per view call,
    // and even "meeting at 12:30" produces a candidate ("30").
    let lookup = |code: &str| shortcode_index.get(&code.to_ascii_lowercase());

    let mut plain_start = 0;
    let mut search_from = 0;

    while let Some(open_rel) = text[search_from..].find(':') {
        let open = search_from + open_rel;
        let inner_start = open + 1;
        let Some(close_rel) = text[inner_start..].find(':') else { break };
        let close = inner_start + close_rel;
        let code = &text[inner_start..close];

        if !code.is_empty() && code.len() <= 64 && code.chars().all(is_shortcode_char) {
            if let Some(emoji) = lookup(code) {
                if open > plain_start {
                    tokenize_unicode_emoji_into(&text[plain_start..open], out);
                }
                out.push(BodySegment::Emoji(emoji));
                plain_start = close + 1;
                search_from = close + 1;
                continue;
            }
        }
        // No match — the closing colon may itself open the next code
        // (e.g. "well:sad:" → retry from the ':' before "sad").
        search_from = close;
    }

    if plain_start < text.len() {
        tokenize_unicode_emoji_into(&text[plain_start..], out);
    }
}

/// The longest real emoji sequence at the start of `s` (checked longest
/// first, up to the ~8-codepoint length of the longest real ZWJ sequences —
/// e.g. the couple-kissing family ones — so a sequence's first codepoint
/// alone is never mistaken for the whole thing). Returns the byte length
/// matched in `s` and the emoji's canonical `'static` string.
fn emoji_prefix(s: &str) -> Option<(usize, &'static str)> {
    const MAX_EMOJI_CHARS: usize = 8;
    // Fixed stack buffer, not a Vec — this runs once per char position of
    // every non-ASCII text segment, per view rebuild (the hottest loop).
    let mut ends = [0usize; MAX_EMOJI_CHARS];
    let mut n = 0;
    let mut end = 0;
    for c in s.chars().take(MAX_EMOJI_CHARS) {
        end += c.len_utf8();
        ends[n] = end;
        n += 1;
    }
    ends[..n].iter().rev().find_map(|&end| emojis::get(&s[..end]).map(|e| (end, e.as_str())))
}

/// Splits `text` on real unicode emoji (plain-ASCII text, the common case,
/// is returned as a single segment untouched — no emoji is representable in
/// pure ASCII).
fn tokenize_unicode_emoji_into<'a>(text: &'a str, out: &mut Vec<BodySegment<'a>>) {
    if text.is_ascii() {
        out.push(BodySegment::Text(text));
        return;
    }
    let mut plain_start = 0;
    let mut cursor = 0;
    while cursor < text.len() {
        if let Some((len, canonical)) = emoji_prefix(&text[cursor..]) {
            if cursor > plain_start {
                out.push(BodySegment::Text(&text[plain_start..cursor]));
            }
            out.push(BodySegment::UnicodeEmoji(canonical));
            cursor += len;
            plain_start = cursor;
        } else {
            let step = text[cursor..].chars().next().map_or(1, char::len_utf8);
            cursor += step;
        }
    }
    if plain_start < text.len() {
        out.push(BodySegment::Text(&text[plain_start..]));
    }
}

/// Every real unicode emoji embedded in `text` (canonical, fully-qualified
/// forms) — the same detector `render_text_body` renders with, exposed so
/// the fetch-triggering pass in `update.rs` can warm the Twemoji cache for
/// inline message emoji the same way it already does for reactions (keeping
/// what's fetched and what's looked up on the same key).
pub fn unicode_emojis_in(text: &str) -> Vec<&'static str> {
    if text.is_ascii() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut cursor = 0;
    while cursor < text.len() {
        match emoji_prefix(&text[cursor..]) {
            Some((len, canonical)) => {
                out.push(canonical);
                cursor += len;
            }
            None => cursor += text[cursor..].chars().next().map_or(1, char::len_utf8),
        }
    }
    out
}

/// Message text with inline custom emoji and clickable links. Emoji-only
/// messages render big (Element-style). Lines containing emoji or links
/// render as a segment row (no wrapping — iced has no flow container that
/// interleaves widgets into a wrapped paragraph); plain lines keep normal
/// wrapping text.
fn render_text_body<'a>(
    body: &'a str,
    shortcode_index: &'a HashMap<String, client_core::events::CustomEmoji>,
    media: &'a crate::media_cache::State,
) -> Element<'a, Message> {
    // Links need their scheme's colon and shortcodes are ':code:', so a
    // body that's both colon-free and plain ASCII (the common case — no
    // unicode emoji is representable in ASCII) skips the whole linkify +
    // shortcode + emoji scan. This runs per text item per view call, the
    // hottest loop in the app.
    if body.is_ascii() && !body.contains(':') {
        return text(body).size(15).into();
    }
    let lines: Vec<Vec<BodySegment<'a>>> =
        body.split('\n').map(|line| tokenize_line(line, shortcode_index)).collect();

    let has_special = lines
        .iter()
        .flatten()
        .any(|segment| !matches!(segment, BodySegment::Text(_)));
    if !has_special {
        return text(body).size(15).into();
    }

    let emoji_widget = |emoji: &'a client_core::events::CustomEmoji, size: u16| -> Element<'a, Message> {
        crate::media_cache::mxc_visual(media, &emoji.mxc_url, size, Some(size))
            .unwrap_or_else(|| text(format!(":{}:", emoji.shortcode)).size(15).into())
    };
    let link_widget = |url: &str| -> Element<'a, Message> {
        button(text(url.to_string()).size(15))
            .on_press(Message::OpenUrl(url.to_string()))
            .style(crate::theme::link_button)
            .padding(0)
            .into()
    };

    let emoji_only = lines.iter().flatten().all(|segment| match segment {
        BodySegment::Emoji(_) | BodySegment::UnicodeEmoji(_) => true,
        BodySegment::Text(t) => t.trim().is_empty(),
        BodySegment::Link(_) => false,
    });
    if emoji_only {
        let mut out = column![].spacing(2);
        for chunk in lines
            .iter()
            .flatten()
            .filter(|s| matches!(s, BodySegment::Emoji(_) | BodySegment::UnicodeEmoji(_)))
            .collect::<Vec<_>>()
            .chunks(12)
        {
            let mut line = row![].spacing(2);
            for segment in chunk {
                match segment {
                    BodySegment::Emoji(emoji) => line = line.push(emoji_widget(emoji, 28)),
                    BodySegment::UnicodeEmoji(emoji) => {
                        line = line.push(crate::emoji_picker::emoji_visual(media, emoji, 28));
                    }
                    _ => {}
                }
            }
            out = out.push(line);
        }
        return out.into();
    }

    let mut out = column![].spacing(2);
    for segments in lines {
        let plain_only = segments.iter().all(|s| matches!(s, BodySegment::Text(_)));
        if plain_only {
            // Re-join (a line without emoji/links is a single Text segment,
            // or empty) — plain text wraps properly.
            let content: String =
                segments
                    .iter()
                    .map(|s| match s {
                        BodySegment::Text(t) => *t,
                        _ => unreachable!(),
                    })
                    .collect();
            out = out.push(text(content).size(15));
            continue;
        }
        let mut line = row![].spacing(2).align_y(iced::Center);
        for segment in segments {
            match segment {
                BodySegment::Text(t) => line = line.push(text(t).size(15)),
                BodySegment::Emoji(emoji) => line = line.push(emoji_widget(emoji, 20)),
                BodySegment::UnicodeEmoji(emoji) => {
                    line = line.push(crate::emoji_picker::emoji_visual(media, emoji, 20));
                }
                BodySegment::Link(url) => line = line.push(link_widget(url)),
            }
        }
        out = out.push(line);
    }
    out.into()
}

/// "Alice, Bob and 3 more" for a reaction's hover tooltip — display names
/// resolved from the room's member list, falling back to the user id's
/// localpart.
fn reaction_senders_label(
    senders: &[String],
    members: &[RoomMember],
    member_index: &HashMap<String, usize>,
) -> String {
    const SHOWN: usize = 12;
    let mut names: Vec<String> = senders
        .iter()
        .map(|user_id| {
            member_by_id(members, member_index, user_id)
                .map(|m| m.display_name.clone())
                .unwrap_or_else(|| {
                    user_id.trim_start_matches('@').split(':').next().unwrap_or(user_id).to_string()
                })
        })
        .collect();
    if names.len() > SHOWN {
        let extra = names.len() - SHOWN;
        names.truncate(SHOWN);
        format!("{} +{extra} more", names.join(", "))
    } else {
        names.join(", ")
    }
}

/// Reaction keys aren't parsed by matrix-sdk (confirmed: fully opaque
/// strings) — custom-emoji clients commonly use the raw `mxc://` URL as the
/// key directly; some use a `:shortcode:` form instead. Try both before
/// falling back to rendering the key as plain text (covers real unicode
/// emoji too).
fn resolve_reaction_visual<'a>(
    key: &'a str,
    media: &'a crate::media_cache::State,
    shortcode_index: &'a HashMap<String, client_core::events::CustomEmoji>,
) -> Element<'a, Message> {
    if let Some(visual) = crate::media_cache::mxc_visual(media, key, 18, Some(18)) {
        return visual;
    }
    // Hash lookup instead of a full pack scan — this runs per reaction pill
    // per view rebuild, and unicode keys (the common case) never matched
    // anyway. Now case-insensitive like the message tokenizer.
    if !shortcode_index.is_empty() {
        let shortcode = key.trim_matches(':');
        if let Some(emoji) = shortcode_index.get(&shortcode.to_ascii_lowercase()) {
            if let Some(visual) = crate::media_cache::mxc_visual(media, &emoji.mxc_url, 18, Some(18))
            {
                return visual;
            }
        }
    }
    crate::emoji_picker::emoji_visual(media, key, 20)
}

/// The full sectioned picker (frequently used + custom packs + every
/// unicode group, vertically scrolled) — no hardcoded quick strip; the
/// "Frequently used" section is the personal equivalent.
fn reaction_picker<'a>(
    event_id: &'a str,
    emoji_usage: &'a HashMap<String, u32>,
    media: &'a crate::media_cache::State,
    packs: &'a [EmojiPack],
) -> Element<'a, Message> {
    let event_id_unicode = event_id.to_string();
    let event_id_custom = event_id.to_string();
    crate::emoji_picker::view(
        emoji_usage,
        media,
        packs,
        move |glyph| Message::ReactWithEmoji {
            event_id: event_id_unicode.clone(),
            key: glyph.to_string(),
        },
        move |emoji| Message::ReactWithEmoji {
            event_id: event_id_custom.clone(),
            key: emoji.mxc_url.clone(),
        },
    )
}
