//! Root `Message` enum. Per-feature screens own their own `Message` enums
//! (see `screens::*`); this composes them rather than flattening everything
//! into one enum, per the project plan's state-management guideline.

use client_core::{ClientCommand, ClientEvent};
use tokio::sync::mpsc;

use crate::screens;

/// `matrix_sdk::Client` isn't guaranteed `Debug`, and `Message` must be.
/// Wrapping it (and the command sender) keeps that upstream detail from
/// leaking into every match arm that touches login/session state.
pub struct OpaqueClient(pub matrix_sdk::Client);

impl Clone for OpaqueClient {
    fn clone(&self) -> Self {
        OpaqueClient(self.0.clone())
    }
}

impl std::fmt::Debug for OpaqueClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpaqueClient").finish_non_exhaustive()
    }
}

pub struct OpaqueCmdSender(pub mpsc::UnboundedSender<ClientCommand>);

impl Clone for OpaqueCmdSender {
    fn clone(&self) -> Self {
        OpaqueCmdSender(self.0.clone())
    }
}

impl std::fmt::Debug for OpaqueCmdSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpaqueCmdSender").finish_non_exhaustive()
    }
}

// `Clone` because iced's `button::on_press`/`text_input::on_submit` take
// the message by value and require it — root-shell controls (top bar,
// keyword panel) hand this enum to widgets directly.
#[derive(Debug, Clone)]
pub enum Message {
    /// Every event the sync worker emits, forwarded verbatim through the
    /// `Subscription` set up in `subscriptions.rs`.
    Client(ClientEvent),

    /// Emitted once by the subscription's producer right after it spawns
    /// the sync worker, handing the UI the command sender.
    WorkerStarted(OpaqueCmdSender),

    /// Result of the startup attempt to restore a previously saved session.
    RestoreResult(Result<Option<OpaqueClient>, String>),

    /// Result of an interactive login attempt (password or SSO).
    LoginResult(Result<OpaqueClient, String>),

    Login(screens::login::Message),
    RoomList(screens::room_list::Message),
    Timeline(screens::timeline::Message),
    Verification(screens::verification::Message),
    Settings(screens::settings::Message),

    /// Result of fetching a Twemoji SVG for a unicode emoji grapheme
    /// cluster (see `twemoji.rs`) — a pure UI-side fetch, not routed
    /// through `client_core` since it isn't Matrix data.
    EmojiSvgFetched(String, Result<Vec<u8>, String>),

    /// Result of resolving a tweet URL via the FxTwitter API (keyed by the
    /// URL as it appeared in the message body).
    TweetFetched(String, Result<crate::tweets::TweetData, String>),
    /// Result of resolving a Steam store link via the storefront
    /// `appdetails` API (keyed by the URL as it appeared in the body).
    SteamFetched(String, Result<crate::steam::SteamAppData, String>),
    /// Result of fetching a plain-HTTPS image (tweet avatar/photo,
    /// Steam capsule art).
    WebImageFetched(String, Result<Vec<u8>, String>),
    /// A fetched GIF finished frame-decoding off-thread (the decode is
    /// CPU-heavy — full RGBA per frame — and would freeze the UI if done in
    /// `update()`). `Err` carries the raster fallback for corrupt GIFs.
    /// `Arc` because `iced_gif::Frames` isn't `Clone` but `Message` must be.
    GifDecoded(String, Result<std::sync::Arc<iced_gif::Frames>, iced::widget::image::Handle>),

    /// A file was picked from the (non-modal) attachment dialog. Carries
    /// the room that was open when the dialog was launched — the user can
    /// switch rooms while it sits open, and the file must not be sent to
    /// whichever room happens to be open when the dialog resolves.
    AttachmentPickedFor { room_id: String, filename: String, bytes: Vec<u8> },

    /// Dismiss the fullscreen image lightbox.
    CloseZoom,

    // --- embedded YouTube player overlay (native webview, see `video_player`) ---
    /// The native webview finished starting (or failed) on the event-loop
    /// thread; carries the window geometry captured at open time.
    VideoPlayerOpened { window: iced::Size, scale: f32, result: Result<(), String> },
    /// Window resized — keep the native video surface glued to the overlay.
    WindowResized(iced::Size),
    /// Dismiss the player overlay (backdrop click or ✕).
    CloseVideoPlayer,
    /// "Watch on YouTube": open the watch page externally and dismiss.
    OpenVideoInBrowser,

    // --- top-bar controls (owned by the root shell, not any one screen) ---
    ToggleSettings,
    ToggleKeywordPanel,
    KeywordDraftChanged(String),
    AddKeywordClicked,
    RemoveKeywordClicked(String),

    Noop,
}
