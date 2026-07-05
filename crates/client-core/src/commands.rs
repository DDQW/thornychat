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
        request_id: RequestId,
    },
    ToggleReaction { room_id: String, event_id: String, key: String, request_id: RequestId },
    SetTyping { room_id: String, typing: bool },
    /// Marks the room as read up to its latest event.
    MarkRoomRead { room_id: String },
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
    /// `ClientEvent::DirectMessageReady`.
    OpenDirectMessage { user_id: String, request_id: RequestId },

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
    JoinRoom { room_id_or_alias: String, request_id: RequestId },
}
