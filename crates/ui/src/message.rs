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
// the message by value and require it — root-shell controls (the own-profile
// row) hand this enum to widgets directly.
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
    SpaceExplorer(screens::space_explorer::Message),
    DmSearch(screens::dm_search::Message),

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
    /// `Arc` because `animated_image::Frames` isn't `Clone` but `Message` must be.
    GifDecoded(
        String,
        Result<std::sync::Arc<crate::animated_image::Frames>, iced::widget::image::Handle>,
    ),

    /// Ctrl+V was pressed somewhere in the main shell: probe the clipboard
    /// for files/images to stage as attachments. Plain text is left alone —
    /// the focused text_input pastes it natively (see `clipboard_paste`).
    PasteClipboard,
    /// A file was drag-and-dropped onto the window: stage it as an
    /// attachment chip, same as a pasted file. The OS delivers a multi-file
    /// drop as one of these per file.
    FileDropped(std::path::PathBuf),
    /// Attachment bytes arrived for staging, from the (non-modal) picker
    /// dialog or a clipboard paste. Carries the room that was open when the
    /// read started — the user can switch rooms while a dialog sits open,
    /// and the files must not stage into whichever room happens to be open
    /// when it resolves. `failed` counts entries that couldn't be read
    /// (folders, files gone by read time).
    AttachmentsReadFor { room_id: String, files: Vec<(String, Vec<u8>)>, failed: usize },

    /// Dismiss the fullscreen image lightbox.
    CloseZoom,
    /// Save the open lightbox image to disk (re-fetches the original bytes,
    /// then opens a save dialog).
    DownloadZoomedImage,
    /// The lightbox widget crossed the zoom threshold where upscaling helps —
    /// kick off a super-resolution pass for the open image (once).
    UpscaleZoomedImage,
    /// A super-resolution pass finished: the sharper handle for `url`, or an
    /// error string. `Handle` is `Arc`-backed, so carrying it here is cheap.
    ImageUpscaled { url: String, handle: Result<iced::widget::image::Handle, String> },
    /// Escape pressed anywhere — dismisses the image lightbox (a press on
    /// the image itself pans rather than closing it, so this is the
    /// guaranteed keyboard exit).
    EscapePressed,

    // --- inline video player (native webview glued over the playing
    // card's stage — see `video_player`) ---
    /// Window scale factor, fetched when playback starts; webview creation
    /// waits for it (stage rects are logical, the native window physical).
    InlineVideoScale(f32),
    /// A stage-geometry probe resolved: `(full rect, visible slice)`, or
    /// `None` when no stage container is in the widget tree (see
    /// `video_player::stage_bounds_probe`).
    InlineVideoBounds(Option<(iced::Rectangle, Option<iced::Rectangle>)>),
    /// The native webview finished starting (or failed) on the event-loop
    /// thread.
    InlineVideoOpened(Result<(), String>),
    /// A mouse press reached the app surface while a video plays inline —
    /// i.e. the click landed off the video (its own child HWND swallows its
    /// own clicks). Hands Win32 keyboard focus back to the app window so the
    /// composer types again after the user has clicked into the video (see
    /// `video_player::reclaim_focus`).
    ReclaimAppFocus,
    /// Window resized — the timeline's scroll anchor geometry is stale, and
    /// the new size is buffered for the debounced geometry save.
    WindowResized(iced::Size),
    /// Window moved — buffered so the next launch reopens where the window
    /// was left (see `window_config`).
    WindowMoved(iced::Point),
    /// The geometry-save debounce elapsed: probe the maximized state, which
    /// decides what the buffered values mean.
    PersistWindowGeometry,
    /// Maximized-state answer for the pending geometry save.
    WindowGeometryProbed(bool),
    /// Cursor moved (window-global coords) — tracked so right-click menus can
    /// open at the pointer (see `state::App::cursor_position`).
    CursorMoved(iced::Point),

    // --- middle-click autoscroll (see `timeline::State::autoscroll`) ---
    /// One frame of the autoscroll glide: scroll the timeline toward the
    /// cursor's offset from the anchor. Fired ~60×/s by a timer subscription
    /// that's only live while autoscroll is.
    AutoscrollTick,
    /// End autoscroll — a click, wheel tick, or key press while it's active
    /// (browser-style: any of them stops the glide).
    AutoscrollEnd,

    // --- room leave/forget/rename actions (sidebar right-click) ---
    /// Leave the room, then dismiss the prompt.
    ConfirmLeaveRoom(String),
    /// Leave (if needed) and forget the room, then dismiss the prompt.
    ConfirmForgetRoom(String),
    /// Edited the room-name field in the prompt.
    RoomRenameDraftChanged(String),
    /// Commit the edited name as the room's `m.room.name`, then dismiss.
    ConfirmRoomRename(String),
    /// Dismiss the prompt without doing anything.
    CancelRoomAction,

    // --- root shell controls (not owned by any one screen) ---
    ToggleSettings,
    /// "Sign in again" clicked on the session-expired banner — reuses the
    /// same logout command Settings' Sign Out button sends, which clears
    /// the dead credentials and returns to the login screen.
    SignInAgainClicked,

    // --- Settings panel resize (drag handle, bottom-right corner) ---
    /// Press on the grip: arms a drag, but the panel's size doesn't change
    /// until the first `SettingsResizeDragged` establishes the cursor anchor
    /// (`on_press` has no position to seed it with).
    SettingsResizeStarted,
    SettingsResizeDragged(iced::Point),
    SettingsResizeReleased,

    /// The media-coalescing timer fired: promote everything staged since it was
    /// armed into the visible caches in one shot. Batching a burst's media into
    /// a single reflow keeps iced's redraw settle loop converging — a per-fetch
    /// reflow storm otherwise retriggers the timeline's scroll-anchor
    /// correction cascade past its 3-consecutive-redraw budget. See
    /// [`crate::media_cache::State::staged`].
    FlushStagedMedia,

    /// The connector poll timer fired: kick off an off-thread detection pass
    /// for the game you're currently playing (see `crate::connectors`).
    PollConnectors,
    /// Detection finished: the game running per the enabled connectors, or
    /// `None`. Diffed against the last known game to decide whether to post an
    /// emote into the open room.
    ConnectorsDetected(Option<crate::connectors::ActiveGame>),

    Noop,
}
