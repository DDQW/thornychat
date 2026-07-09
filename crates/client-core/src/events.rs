//! Events flow sync worker -> UI. These are the only types `ui::*` is allowed
//! to depend on for anything that originates from `matrix-sdk` — no
//! `matrix_sdk::*` type should ever cross this boundary directly.

use crate::commands::RequestId;

/// Best-effort display label for a user id when no display name is set.
/// Strips a leading `@` and the trailing `:server`, then an `irc_`
/// bridge-ghost prefix if one is present — matrix-appservice-irc puppets
/// users as `@irc_<nick>:server`, so a bridged sender with no Matrix
/// display name would otherwise render as the raw `@irc_alice:matrix.org`
/// instead of just `alice`.
///
/// This is only ever reached as the *last-resort* fallback (after every
/// cached/profile display name comes back absent), so a genuine, unbridged
/// user whose localpart happens to start with `irc_` and who never set a
/// display name would have it stripped too — accepted as a rare,
/// cosmetic-only false positive (the raw `user_id` is untouched, only the
/// rendered label changes). The `irc_` strip is skipped when nothing would
/// remain (e.g. the pathological id `@irc_:server`), so it never yields an
/// empty string.
pub fn friendly_user_id(user_id: &str) -> &str {
    let localpart = user_id.strip_prefix('@').unwrap_or(user_id);
    let localpart = localpart.split(':').next().unwrap_or(localpart);
    match localpart.strip_prefix("irc_") {
        Some(nick) if !nick.is_empty() => nick,
        _ => localpart,
    }
}

#[derive(Debug, Clone)]
pub enum ClientEvent {
    // --- Phase 0: session / sync lifecycle ---
    SyncStateChanged(SyncState),
    LoggedOut,

    // --- Phase 1: room list / timeline ---
    RoomListUpdated(Vec<RoomSummary>),
    /// Sent once when a room is opened (used to populate @mention
    /// autocomplete). Not kept live-updated yet — good enough for Phase 2;
    /// revisit if member changes need to reflect without reopening the room.
    RoomMembersUpdated { room_id: String, members: Vec<RoomMember> },
    /// A DM room with the requested user is ready (found or freshly
    /// created) — the UI should open it.
    DirectMessageReady { room_id: String },
    /// A room requested via `ClientCommand::CreateRoomWith` was created — the
    /// UI should open it (same handling as `DirectMessageReady`).
    RoomCreated { room_id: String },
    /// MSC3949 power-level tags for a room ("Red team", "Purple team",
    /// ...) — custom member-list groups defined by the room. Sorted by
    /// power level, highest first. Sent when a room is opened.
    PowerLevelTagsUpdated { room_id: String, tags: Vec<PowerLevelTag> },
    /// The full, current set of timeline items for `room_id`. Phase 1 always
    /// sends a full snapshot rather than an incremental diff — simple and
    /// correct for the room sizes a chat client deals with; revisit only if
    /// profiling shows it matters.
    TimelineUpdated { room_id: String, items: Vec<TimelineItem> },
    TypingUpdated { room_id: String, user_ids: Vec<String> },
    ReceiptsUpdated { room_id: String },
    /// Back-pagination hit the very first event of the room — no more
    /// history to load.
    TimelineStartReached { room_id: String },

    // --- Phase 4: custom emoji, media ---
    /// Every custom emoji pack this account can currently use: the
    /// personal pack (`im.ponies.user_emotes`), any packs explicitly
    /// enabled via `im.ponies.emote_rooms`, and the currently-open room's
    /// own default pack (`im.ponies.room_emotes` with an empty state key)
    /// if it has one. Refetched whenever a room is opened.
    CustomEmojiPacksUpdated(Vec<EmojiPack>),
    MediaFetched { request_id: RequestId, bytes: Vec<u8> },
    MediaFetchFailed { request_id: RequestId, reason: String },
    /// OpenGraph data for a URL, resolved via the homeserver's
    /// `preview_url` proxy (the URL is never fetched directly from this
    /// machine).
    UrlPreviewFetched(UrlPreview),
    UrlPreviewFailed { url: String },

    // --- Phase 3: cross-signing bootstrap (runs automatically after login) ---
    /// Bootstrapping needs interactive auth the SDK can't complete for us
    /// (works uniformly for password, SSO, or any other auth stage the
    /// homeserver demands): open `url` in a browser, complete whatever it
    /// asks for, then send `ClientCommand::RetryCrossSigningBootstrap`.
    CrossSigningBootstrapNeedsFallback { url: String },
    CrossSigningBootstrapDone,
    CrossSigningBootstrapFailed { reason: String },

    // --- Phase 3: SAS device verification (one active flow at a time) ---
    VerificationStateChanged(SasState),

    // --- Phase 3: key backup / recovery ---
    /// Secret storage is set up but this device is missing some secrets
    /// (typical for a brand-new device joining encrypted rooms) — prompt
    /// for the recovery key.
    KeyBackupNeedsRecovery,
    /// No recovery/secret-storage set up at all yet — offer to enable it.
    RecoverySetupNeeded,
    RecoveryEnableProgress(RecoveryEnableStage),
    /// The recovery key was just generated; show it to the user exactly
    /// once with an explicit "I've saved it" confirmation before dismissing.
    RecoveryEnabled { recovery_key: String },
    RecoveryEnableFailed { reason: String },
    KeyBackupRestored,
    KeyBackupFailed { reason: String },

    // --- Phase 4: read receipts, notification settings, search ---
    Notification(NotificationEvent),
    RoomNotificationModeChanged { room_id: String, mode: NotificationMode },
    /// The user's per-room override was removed (back to the account
    /// default) — the counterpart to `RoomNotificationModeChanged`.
    RoomNotificationModeCleared { room_id: String },
    /// Full snapshot of every room with a user-defined notification
    /// override. Sent once at startup and again whenever the account's push
    /// rules change (including changes made from another device) — rooms
    /// absent from the list follow the account default.
    RoomNotificationModesUpdated(Vec<(String, NotificationMode)>),
    /// The account-wide default notification mode for direct messages and
    /// group rooms (what a room follows when it has no per-room override).
    /// Sent by the same watcher as `RoomNotificationModesUpdated`, so it
    /// stays current with changes made from other devices too.
    DefaultNotificationModesUpdated { direct_messages: NotificationMode, group_chats: NotificationMode },
    SearchResults { request_id: RequestId, results: Vec<SearchResult> },
    /// Results of a user-directory search (see `ClientCommand::SearchUsers`).
    /// `limited` is the server's flag that it truncated the result set — the
    /// UI can hint that a more specific query would help.
    UserSearchResults { request_id: RequestId, results: Vec<UserSearchResult>, limited: bool },

    // --- Phase 5: calls (MatrixRTC signaling) ---
    /// Live call state for one room. Sent whenever a call membership
    /// changes via sync, when a room is opened, and optimistically right
    /// after a local join/leave. Empty `participants` = no active call
    /// (sent too, so an ending call clears the banner).
    CallStateUpdated(CallState),

    // --- Phase 6: spaces ---
    /// One page of a space's direct children (the space-hierarchy API),
    /// answering `ClientCommand::FetchSpaceHierarchy`. Pages append: the UI
    /// passes this page's `next_batch` back as `from` to get the following
    /// one (`None` = last page).
    SpaceHierarchyFetched {
        request_id: RequestId,
        space_id: String,
        children: Vec<SpaceChildSummary>,
        next_batch: Option<String>,
    },
    /// The full child room-id set of a joined space (joined or not, rooms
    /// and subspaces alike), fetched unprompted at startup and after joins —
    /// the sidebar uses it to nest joined rooms under their parent space.
    /// Replaces that space's previous set wholesale.
    SpaceChildrenFetched { space_id: String, children: Vec<String> },

    // --- correlated command outcomes ---
    CommandFailed { request_id: RequestId, error: String },
    CommandSucceeded { request_id: RequestId },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncState {
    Connecting,
    Syncing,
    Offline,
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoomMember {
    pub user_id: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    /// Matrix power level (0 = default member, 50 = moderator by
    /// convention, 100 = admin) — drives the member list's grouping.
    pub power_level: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoomSummary {
    pub room_id: String,
    pub name: String,
    pub topic: Option<String>,
    pub avatar_url: Option<String>,
    pub unread_count: u64,
    pub is_encrypted: bool,
    pub is_space: bool,
    /// Marked as a direct message via `m.direct` account data.
    pub is_dm: bool,
    pub last_message_preview: Option<String>,
}

/// One room (or subspace) listed under a space by the space-hierarchy API.
/// Unlike `RoomSummary` this can describe rooms the account hasn't joined —
/// that's the point of the space explorer.
#[derive(Debug, Clone, PartialEq)]
pub struct SpaceChildSummary {
    pub room_id: String,
    /// `None` when the room publishes no name — display falls back to the
    /// alias, then the room id.
    pub name: Option<String>,
    pub topic: Option<String>,
    pub canonical_alias: Option<String>,
    pub avatar_url: Option<String>,
    pub num_joined_members: u64,
    /// This child is itself a space (the explorer drills in rather than
    /// opening a timeline).
    pub is_space: bool,
    /// Whether this account already belongs to the room.
    pub joined: bool,
    pub join_rule: SpaceJoinRule,
    /// Candidate servers for joining, from the parent space's
    /// `m.space.child` event — needed when our homeserver isn't already in
    /// the room. Empty is fine for rooms it knows.
    pub via: Vec<String>,
}

/// How a space child can be entered, reduced to the cases the explorer UI
/// distinguishes (the spec's `SpaceRoomJoinRule` is wider).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceJoinRule {
    /// Anyone can join.
    Public,
    /// Members of the parent space can join (`restricted` /
    /// `knock_restricted`).
    Restricted,
    /// Joining requires knocking (asking to be let in).
    Knock,
    /// Invite-only — also the bucket for unknown/custom rules.
    InviteOnly,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimelineItem {
    pub event_id: Option<String>,
    pub sender: String,
    pub sender_display_name: Option<String>,
    pub sender_avatar_url: Option<String>,
    pub timestamp_ms: u64,
    pub content: TimelineItemContent,
    /// Trust indicator for encrypted messages, mirroring `matrix-sdk-ui`'s
    /// `ShieldState` (lax mode — matches what Element shows by default).
    /// `None` means no shield should be shown (unencrypted, local echo, or
    /// verified-and-trusted).
    pub shield: Option<TrustShield>,
    pub reactions: Vec<ReactionGroup>,
    /// Set if this message is a reply within a thread; `None` for the main
    /// timeline. Only the thread root carries `thread_reply_count`.
    pub thread_root: Option<String>,
    pub thread_reply_count: Option<u32>,
    /// User IDs with a read receipt exactly on this event (not "read up to
    /// and including" — matrix-sdk-ui attaches receipts per-event already).
    pub read_by: Vec<String>,
    /// Set when this message is a rich reply — who/what it quotes.
    pub in_reply_to: Option<ReplyPreview>,
}

/// OpenGraph-style link preview, as returned by the homeserver's
/// `/media/preview_url` endpoint. `image_mxc` is an `mxc://` URI (the
/// homeserver re-hosts the remote image), fetchable through the normal
/// media path.
#[derive(Debug, Clone, PartialEq)]
pub struct UrlPreview {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub site_name: Option<String>,
    pub image_mxc: Option<String>,
    /// `og:image:width`/`height` when the homeserver reports them — used to
    /// tell square avatars from attached media in tweet cards.
    pub image_width: Option<u64>,
    pub image_height: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReplyPreview {
    pub event_id: String,
    /// Display name (or user id) of the quoted message's sender; empty when
    /// the SDK hasn't loaded the replied-to event's details.
    pub sender: String,
    pub snippet: String,
    /// Set when the quoted message is an image — quotes render it as a
    /// small thumbnail.
    pub image_url: Option<String>,
}

/// One MSC3949 power-level tag: a named (optionally colored) member group
/// anchored at a power level. Members belong to the tag at their level,
/// falling back to the nearest lower defined tag.
#[derive(Debug, Clone, PartialEq)]
pub struct PowerLevelTag {
    pub level: i64,
    pub name: String,
    /// Hex color like `#ff0000`, when the room defined one.
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReactionGroup {
    /// The raw annotation key: a unicode emoji, or a custom-emoji shortcode
    /// like `:hq_wave:` if that's what the sender's client sent.
    pub key: String,
    pub count: usize,
    pub reacted_by_me: bool,
    /// User IDs of everyone who sent this reaction (hover attribution).
    pub senders: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmojiPack {
    pub name: String,
    pub emojis: Vec<CustomEmoji>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CustomEmoji {
    pub shortcode: String,
    pub mxc_url: String,
    /// MSC2545 usage flags. An image that declares no usage is usable as
    /// *both*, so both default to true — the emoji picker shows the
    /// `is_emoticon` images, the sticker picker shows the `is_sticker` ones.
    pub is_emoticon: bool,
    pub is_sticker: bool,
    /// Intrinsic dimensions from the pack's per-image `info` block, when the
    /// pack declares them — used to render stickers at true aspect and to
    /// fill the `m.sticker` event's `ImageInfo` on send.
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// Signaling-level state of a room's MatrixRTC call (MSC3401 room-scope
/// `m.call` memberships).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CallState {
    pub room_id: String,
    /// Everyone with an unexpired call membership, oldest first. A user
    /// joined from two devices appears twice.
    pub participants: Vec<CallParticipant>,
    /// Whether *this* device has an active membership (not just this user
    /// — the same account joined from a phone doesn't count).
    pub joined: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CallParticipant {
    pub user_id: String,
    pub device_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationMode {
    AllMessages,
    MentionsAndKeywordsOnly,
    Mute,
}

/// Which account-wide default a `ClientCommand::SetDefaultNotificationMode`
/// targets. The SDK models this as a 4-way matrix (encrypted x one-to-one),
/// but the UI only exposes these two buckets — setting either one writes
/// both the encrypted and unencrypted variant together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationScope {
    DirectMessages,
    GroupChats,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TrustShield {
    Red(String),
    Grey(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum TimelineItemContent {
    Text(String),
    /// `width`/`height` are the sender-declared intrinsic dimensions from
    /// the event's `info` block (when present). The UI uses them to reserve
    /// the image's exact display footprint *before* the bytes arrive —
    /// otherwise every image load reflows the timeline and yanks the
    /// scroll position around.
    Image { url: String, caption: Option<String>, width: Option<u32>, height: Option<u32> },
    /// A sticker (`m.sticker`). Rendered like an image but kept distinct so it
    /// can render at sticker size (no caption/bubble) and so the composer can
    /// harvest it into the grow-with-use collection. `body` is the sticker's
    /// alt text / shortcode, reused as the `body` when resending it.
    Sticker { url: String, body: String, width: Option<u32>, height: Option<u32> },
    /// `caption` follows MSC2530: when the event carries a `filename` field,
    /// a differing `body` is a caption, not the filename.
    File { url: String, filename: String, caption: Option<String> },
    Redacted,
    /// A membership change (join/leave/kick/ban/invite/knock…) rendered as a
    /// pre-composed human sentence (e.g. "alice joined the room"). Kept as
    /// its own variant rather than folded into `Text` so the UI can render it
    /// as a compact system line and hide it wholesale via the timeline
    /// setting, without touching real chat messages.
    MembershipChange(String),
    DateDivider(String),
    /// Everything below this point is unread (the SDK's read-marker
    /// position, i.e. the `m.fully_read` marker).
    NewMessagesDivider,
}

#[derive(Debug, Clone)]
pub enum SasState {
    /// Someone else started verifying with us; show an Accept/Decline
    /// prompt naming who.
    RequestReceived { from_user_id: String },
    /// We started verifying (self-verification or another user); waiting
    /// for them to accept.
    RequestSent,
    /// Both sides are ready; about to exchange keys and show emoji.
    Ready,
    /// The 7 (emoji, name) pairs to compare with the other party.
    EmojisReady(Vec<(String, String)>),
    /// We've confirmed our side matches; waiting on the other party.
    WaitingForOtherPartyConfirmation,
    Done,
    Cancelled { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryEnableStage {
    Starting,
    CreatingBackup,
    CreatingRecoveryKey,
    BackingUp,
}

#[derive(Debug, Clone)]
pub struct NotificationEvent {
    pub room_id: String,
    pub sender: String,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub room_id: String,
    pub event_id: String,
    pub sender: String,
    pub snippet: String,
}

/// One hit from a user-directory search — enough to render a pick-a-person
/// row and start a DM with them (even someone you share no room with yet).
#[derive(Debug, Clone)]
pub struct UserSearchResult {
    pub user_id: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}
