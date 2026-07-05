use client_core::{events::SyncState, ClientCommand};
use tokio::sync::mpsc;

use crate::message::{Message, OpaqueClient};
use crate::screens;

/// UI-side state of the embedded video player overlay (YouTube, Vimeo,
/// Dailymotion, Rumble, Kick). `window`/`scale` mirror the real window so
/// the iced-drawn frame and the native webview bounds stay glued together
/// (`video_player::video_rect` maps them both).
pub struct VideoPlayer {
    pub video: crate::video_player::EmbedVideo,
    pub title: Option<String>,
    /// Unknown for the first frame or two (fetched right after opening).
    pub window: Option<iced::Size>,
    pub scale: f32,
    /// The native webview failed to start (e.g. no WebView2 runtime) —
    /// the overlay shows this and offers the browser instead.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// No session yet; showing the login screen.
    Login,
    /// Session established; showing the main room list / timeline shell.
    Main,
}

pub struct App {
    pub route: Route,
    pub profile: String,

    /// Present once a session (restored or freshly logged in) exists. Its
    /// mere presence is what activates `subscriptions::client_events`,
    /// which spawns the sync worker.
    pub client: Option<matrix_sdk::Client>,
    pub cmd_tx: Option<mpsc::UnboundedSender<ClientCommand>>,
    pub sync_state: SyncState,
    /// The logged-in user's Matrix ID, e.g. `@mordred:rpghq.org` — used to
    /// tell which timeline messages are "mine" (editable/deletable).
    pub own_user_id: Option<String>,

    pub login: screens::login::State,
    pub room_list: screens::room_list::State,
    pub timeline: screens::timeline::State,
    pub verification: screens::verification::State,
    pub settings: screens::settings::State,
    pub call: screens::call::State,
    pub media: crate::media_cache::State,
    pub emoji_packs: Vec<client_core::events::EmojiPack>,

    /// Rooms with a user-defined notification override; rooms absent here
    /// follow the account default. Kept fresh by client-core's
    /// notification watcher (including changes made from other devices).
    pub notification_modes:
        std::collections::HashMap<String, client_core::events::NotificationMode>,
    pub keyword_highlights: Vec<String>,
    pub keyword_draft: String,
    pub show_keyword_panel: bool,
    pub dark_mode: bool,
    /// `mxc://` URL of the image currently open in the fullscreen lightbox.
    pub zoomed_image: Option<String>,
    /// Present while the in-app video player overlay is open (the native
    /// webview itself lives on the event-loop thread — see `video_player`).
    pub video_player: Option<VideoPlayer>,
    /// Link-preview cache, keyed by URL. `None` = requested (or failed) —
    /// either way, don't re-request; `Some` = OpenGraph data to render.
    pub url_previews:
        std::collections::HashMap<String, Option<client_core::events::UrlPreview>>,
    /// Rich tweet cards from the FxTwitter API, keyed by the message-body
    /// URL. Same `None` = in-flight-or-failed semantics; failures fall
    /// back to the homeserver OpenGraph card in `url_previews`.
    pub tweet_previews: std::collections::HashMap<String, Option<crate::tweets::TweetData>>,
    /// Rich Steam store cards from the storefront `appdetails` API, keyed
    /// by the message-body URL. Same `None` = in-flight-or-failed
    /// semantics and OpenGraph fallback as `tweet_previews`.
    pub steam_previews: std::collections::HashMap<String, Option<crate::steam::SteamAppData>>,
    /// Emoji usage counts feeding the picker's "Frequently used" section
    /// (glyph for unicode, `mxc://` URL for custom emoji). Loaded from and
    /// persisted to a small per-profile JSON file.
    pub emoji_usage: std::collections::HashMap<String, u32>,
    /// Lowercased shortcode → custom emoji, rebuilt whenever `emoji_packs`
    /// changes — O(1) lookups for the message tokenizer and reaction pills,
    /// which otherwise linear-scanned every pack emoji per `:token:` per
    /// frame.
    pub emoji_shortcode_index:
        std::collections::HashMap<String, client_core::events::CustomEmoji>,
}

/// Where the usage history lives (inside the profile's emoji cache dir —
/// disk state that travels with the profile).
pub fn emoji_usage_path(profile: &str) -> Option<std::path::PathBuf> {
    client_core::store::AppPaths::for_profile(profile)
        .ok()
        .map(|paths| paths.emoji_cache_dir().join("usage.json"))
}

fn load_emoji_usage(profile: &str) -> std::collections::HashMap<String, u32> {
    emoji_usage_path(profile)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|contents| serde_json::from_str(&contents).ok())
        .unwrap_or_default()
}

impl App {
    pub fn new(profile: String) -> Self {
        let emoji_usage = load_emoji_usage(&profile);
        Self {
            route: Route::Login,
            profile,
            client: None,
            cmd_tx: None,
            sync_state: SyncState::Connecting,
            own_user_id: None,
            login: screens::login::State::default(),
            room_list: screens::room_list::State::default(),
            timeline: screens::timeline::State::default(),
            verification: screens::verification::State::default(),
            settings: screens::settings::State,
            call: screens::call::State::default(),
            media: crate::media_cache::State::default(),
            emoji_packs: Vec::new(),
            notification_modes: std::collections::HashMap::new(),
            keyword_highlights: Vec::new(),
            keyword_draft: String::new(),
            show_keyword_panel: false,
            dark_mode: true,
            zoomed_image: None,
            video_player: None,
            url_previews: std::collections::HashMap::new(),
            tweet_previews: std::collections::HashMap::new(),
            steam_previews: std::collections::HashMap::new(),
            emoji_usage,
            emoji_shortcode_index: std::collections::HashMap::new(),
        }
    }

    pub fn adopt_client(&mut self, client: matrix_sdk::Client) {
        self.own_user_id = client.user_id().map(|id| id.to_string());
        self.client = Some(client);
        self.route = Route::Main;
    }
}

/// Boots the app: kicks off an async attempt to restore a previously saved
/// session before the login screen is shown, so a returning user doesn't
/// see a login form flash before landing in their rooms.
pub fn boot(profile: String) -> (App, iced::Task<Message>) {
    let restore_profile = profile.clone();
    let task = iced::Task::perform(
        async move {
            let paths = client_core::store::AppPaths::for_profile(&restore_profile)
                .map_err(|e| e.to_string())?;
            client_core::session::try_restore(&paths)
                .await
                .map(|opt| opt.map(|r| OpaqueClient(r.client)))
                .map_err(|e| e.to_string())
        },
        Message::RestoreResult,
    );
    (App::new(profile), task)
}
