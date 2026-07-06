//! Commands flow UI -> sync worker. Every variant that expects a correlated
//! response carries a `request_id` so the UI can match `ClientEvent::CommandFailed`
//! / `CommandSucceeded` back to the action that triggered it.
//!
//! Only login/session/room-open variants are consumed by the worker today
//! (Phase 0). The rest of the shape is sketched now so later phases extend
//! this enum instead of redesigning the channel contract.

use uuid::Uuid;

pub type RequestId = Uuid;

#[derive(Debug, Clone)]
pub enum ClientCommand {
    // --- Phase 0: session / sync lifecycle ---
    Logout,

    // --- Phase 1: room list / timeline navigation ---
    OpenRoom { room_id: String },
    CloseRoom { room_id: String },
    LoadOlderTimelineEvents { room_id: String, request_id: RequestId },

    // --- Phase 2: sending ---
    SendMessage {
        room_id: String,
        body: String,
        mentioned_user_ids: Vec<String>,
        /// When set, sends as a rich reply quoting this event.
        reply_to_event_id: Option<String>,
        request_id: RequestId,
    },
    EditMessage { room_id: String, event_id: String, new_body: String, request_id: RequestId },
    RedactEvent { room_id: String, event_id: String, reason: Option<String>, request_id: RequestId },
    SendAttachment {
        room_id: String,
        filename: String,
        bytes: Vec<u8>,
        mime: String,
        /// Optional caption (MSC2530): rides in the media event's `body`,
        /// with the real name in `filename`. Markdown is rendered the same
        /// way a text message's would be.
        caption: Option<String>,
        /// Users the caption mentions (attached as `m.mentions`).
        mentioned_user_ids: Vec<String>,
        /// When set, sends the attachment as a rich reply quoting this event.
        reply_to_event_id: Option<String>,
        request_id: RequestId,
    },
    /// Post an `m.sticker` event pointing at an already-hosted image. Stickers
    /// picked from a pack or the collection carry an `mxc://` URL already, so
    /// unlike `SendAttachment` there is nothing to upload first.
    SendSticker {
        room_id: String,
        /// `mxc://` URL of the sticker image.
        url: String,
        /// Alt text / shortcode, stored as the event `body`.
        body: String,
        width: Option<u32>,
        height: Option<u32>,
        mimetype: Option<String>,
        request_id: RequestId,
    },
    ToggleReaction { room_id: String, event_id: String, key: String, request_id: RequestId },
    SetTyping { room_id: String, typing: bool },
    /// Marks the room as read up to its latest event. `public_receipt`
    /// chooses the receipt kind: `true` sends the federated public read
    /// receipt (`m.read`, others can see it); `false` (privacy mode) sends
    /// the unfederated private one (`m.read.private`) instead, so the read
    /// position still advances server-side without telling anyone else.
    MarkRoomRead { room_id: String, public_receipt: bool },
    FetchMedia { mxc_url: String, request_id: RequestId },

    // --- Phase 3: cross-signing bootstrap ---
    /// Retry after the user completed the UIAA fallback web page prompted
    /// by `ClientEvent::CrossSigningBootstrapNeedsFallback`.
    RetryCrossSigningBootstrap,

    // --- Phase 3: SAS device verification (acts on the single active flow) ---
    /// `user_id` is the account being verified: pass your own user ID to
    /// self-verify this device via one of your other devices, or someone
    /// else's to verify them.
    StartVerification { user_id: String },
    AcceptVerificationRequest,
    ConfirmSasMatch,
    RejectSasMatch,
    VerificationCancel,

    // --- Phase 3: key backup / recovery ---
    EnableRecovery { passphrase: Option<zeroize::Zeroizing<String>>, request_id: RequestId },
    RestoreFromBackup { recovery_key: zeroize::Zeroizing<String>, request_id: RequestId },

    /// Open (or create) a direct-message room with the user; answered by
    /// `ClientEvent::DirectMessageReady`. `encrypted` chooses whether a
    /// *newly* created DM turns on end-to-end encryption (an existing DM is
    /// reused as-is regardless).
    OpenDirectMessage { user_id: String, encrypted: bool, request_id: RequestId },

    /// Create a fresh private (invite-only) room and invite this user to it;
    /// answered by `ClientEvent::RoomCreated` so the UI can open it.
    /// `encrypted` turns on end-to-end encryption for the new room.
    CreateRoomWith { user_id: String, encrypted: bool, request_id: RequestId },

    /// Leave a room (it drops out of the joined-room list on the next sync).
    LeaveRoom { room_id: String, request_id: RequestId },
    /// Leave (if still joined) *and* forget a room, dropping it from the
    /// local store entirely so it won't reappear across restarts.
    ForgetRoom { room_id: String, request_id: RequestId },

    /// Load ~50 older events into the open room's timeline (the timeline
    /// stream delivers them; `TimelineStartReached` fires if the room's
    /// first event was reached).
    PaginateBackwards { room_id: String, request_id: RequestId },

    /// Resolve an OpenGraph preview for a URL via the homeserver proxy.
    /// Keyed by URL (no request id): the answer arrives as
    /// `UrlPreviewFetched`/`UrlPreviewFailed` carrying the URL back.
    FetchUrlPreview { url: String },

    // --- Phase 4: notification settings, search ---
    SetRoomNotificationMode { room_id: String, mode: crate::events::NotificationMode, request_id: RequestId },
    /// Remove the per-room override entirely, restoring the account default.
    ClearRoomNotificationMode { room_id: String, request_id: RequestId },
    /// Set the account-wide default for direct messages or group rooms
    /// (rooms with no per-room override follow whichever of these applies).
    SetDefaultNotificationMode {
        scope: crate::events::NotificationScope,
        mode: crate::events::NotificationMode,
        request_id: RequestId,
    },
    AddKeywordHighlight { keyword: String, request_id: RequestId },
    RemoveKeywordHighlight { keyword: String, request_id: RequestId },
    Search { query: String, request_id: RequestId },

    // --- Phase 5: calls (MatrixRTC signaling) ---
    /// Join the room's active call, or start one if none exists. Publishes
    /// this device's `m.call.member` state event; answered by
    /// `CallStateUpdated` plus the usual correlated outcome.
    JoinCall { room_id: String, request_id: RequestId },
    LeaveCall { room_id: String, request_id: RequestId },

    // --- Phase 7: room/space admin ---
    InviteUser { room_id: String, user_id: String, request_id: RequestId },
    /// Join a room by id or `#alias:server`. `via` lists candidate servers
    /// for the join handshake (a space-hierarchy listing provides them) —
    /// needed for rooms our homeserver isn't in yet, harmless otherwise.
    JoinRoom { room_id_or_alias: String, via: Vec<String>, request_id: RequestId },

    // --- Phase 6: spaces ---
    /// Fetch one page of a space's *direct* children via the
    /// space-hierarchy API (depth 1 — subspaces are explored by issuing
    /// this again for them). `from` is the previous page's `next_batch`
    /// token (`None` = first page). Answered by
    /// `ClientEvent::SpaceHierarchyFetched`, or `CommandFailed`.
    FetchSpaceHierarchy { space_id: String, from: Option<String>, request_id: RequestId },
}
