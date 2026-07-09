use client_core::{events::SyncState, ClientCommand};
use tokio::sync::mpsc;

use crate::message::{Message, OpaqueClient};
use crate::screens;

/// An open "leave or forget this room?" confirmation prompt, raised by
/// right-clicking a room in the sidebar. Carries the display name so the
/// modal can name the room without re-looking it up.
#[derive(Debug, Clone)]
pub struct RoomActionPrompt {
    pub room_id: String,
    pub room_name: String,
    /// True for a direct message (the modal wording differs slightly).
    pub is_dm: bool,
    /// Editable room-name field in the modal, seeded with `room_name`; the
    /// Rename button commits it as the room's `m.room.name`.
    pub rename_draft: String,
}

/// Default and minimum size of the Settings panel — resizable by the user
/// via the bottom-right grip (see `view::settings_overlay`). Not persisted:
/// resets to the default each launch, same scope as the rest of the panel's
/// transient UI state.
pub const DEFAULT_SETTINGS_SIZE: iced::Size = iced::Size { width: 760.0, height: 640.0 };
pub const MIN_SETTINGS_WIDTH: f32 = 480.0;
pub const MIN_SETTINGS_HEIGHT: f32 = 360.0;

/// An in-progress drag of the Settings panel's resize grip.
#[derive(Debug, Clone, Copy)]
pub struct ResizeDrag {
    pub size_at_start: iced::Size,
    /// Cursor position the drag delta is measured from. `None` until the
    /// first `CursorMoved` after the press — `mouse_area::on_press` doesn't
    /// hand back a position, so the first move establishes the anchor
    /// instead of jumping the panel size on press.
    pub anchor: Option<iced::Point>,
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
    /// The logged-in user's Matrix ID, e.g. `@alice:matrix.org` — used to
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
    /// Account-wide defaults (direct messages, group chats) that a room
    /// follows when it has no entry in `notification_modes`. Kept fresh by
    /// the same client-core watcher, including changes from other devices.
    pub default_notification_modes:
        (client_core::events::NotificationMode, client_core::events::NotificationMode),
    /// Active color/typography/density theme — persisted globally (across
    /// all profiles) and editable from the Appearance settings tab.
    pub theme: crate::theme_config::ThemeConfig,
    /// `theme` pre-built into an `iced::Theme`, rebuilt only when `theme`
    /// changes (in the `Settings` update arm). iced's `.theme()` closure is
    /// called on every update cycle with no dirty-check, and
    /// `to_iced_theme()` regenerates the whole extended palette + allocates —
    /// far too heavy to run per composer keystroke / sync tick. Cloning this
    /// is a cheap `Arc` refcount bump (`Theme::Custom` is `Arc`-backed).
    pub built_theme: iced::Theme,
    /// Privacy preferences (read receipts, typing, link previews) — persisted
    /// globally and editable from the Privacy settings tab. Gates the
    /// activity-sharing commands dispatched in `update.rs`. Defaults to the
    /// most private stance on a fresh install.
    pub privacy: crate::privacy_config::PrivacyConfig,
    /// Whether new DMs/rooms this client creates turn on encryption — both
    /// default off. Persisted globally; read when building create commands.
    pub encryption: crate::encryption_config::EncryptionConfig,
    /// Composer spell-check / autocorrect preferences — persisted globally and
    /// editable from the General settings tab. Read by the composer on every
    /// keystroke (the Windows speller itself lives in `crate::spellcheck`).
    pub spellcheck: crate::spellcheck_config::SpellcheckConfig,
    /// Timeline display preferences (currently just whether membership changes
    /// are shown) — persisted globally, editable from the General settings
    /// tab. Read by `view.rs` when rendering the timeline.
    pub chat: crate::chat_config::ChatConfig,
    pub show_settings: bool,
    /// Latest window-global cursor position, updated on every mouse move (see
    /// `subscriptions::window_events`). Snapshotted when a right-click menu
    /// opens so it can anchor at the pointer.
    pub cursor_position: iced::Point,
    /// Current size of the Settings panel, user-resizable via its
    /// bottom-right grip.
    pub settings_panel_size: iced::Size,
    /// Present while the user is actively dragging the resize grip — also
    /// what gates the mouse-tracking subscription in `subscriptions.rs`.
    pub settings_resize_drag: Option<ResizeDrag>,
    /// The `mxc://` URL of the image currently open in the fullscreen
    /// lightbox, if any.
    pub zoomed_image: Option<String>,
    /// Present while the leave/forget confirmation modal is open (raised by
    /// right-clicking a room in the sidebar).
    pub pending_room_action: Option<RoomActionPrompt>,
    /// Present while the space explorer overlay is open (browsing a
    /// space's rooms via the hierarchy API; opened by clicking a space in
    /// the sidebar).
    pub space_explorer: Option<screens::space_explorer::State>,
    /// Present while the "new direct message" user-search overlay is open
    /// (opened by the "+" on the sidebar's Direct messages header).
    pub dm_search: Option<screens::dm_search::State>,
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
    /// Stickers seen in rooms (and ones the user picks) — the grow-with-use
    /// collection shown in the sticker picker. Most-recent first, deduped by
    /// `url`, capped at [`MAX_COLLECTED_STICKERS`]. Loaded from and persisted
    /// to a small per-profile JSON file, exactly like `emoji_usage`.
    pub sticker_collection: Vec<CollectedSticker>,
    /// Safe-state: the room to reopen on this launch, loaded at startup from
    /// the per-profile `last-room` file. Held pending until the sync brings it
    /// into the room list (sliding sync streams rooms in gradually), then
    /// opened once and cleared. Also cleared the moment the user opens any
    /// room manually, so a restored session never fights a deliberate click.
    pub pending_restore_room: Option<String>,
}

/// One sticker remembered for reuse — harvested from an `m.sticker` event
/// seen in a room, or picked from a pack. Persisted per-profile so the
/// collection survives restarts and grows with use. Deduped by `url`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CollectedSticker {
    /// `mxc://` URL of the sticker image.
    pub url: String,
    /// Alt text / shortcode, reused as the event `body` when resending.
    pub body: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
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

/// Cap on the grow-with-use sticker collection: most-recent kept, oldest
/// dropped past this. Big enough to be useful, bounded so the picker grid and
/// the JSON file stay small.
pub const MAX_COLLECTED_STICKERS: usize = 120;

/// Where the harvested sticker collection lives (next to the emoji usage
/// history, in the profile's emoji cache dir).
pub fn sticker_collection_path(profile: &str) -> Option<std::path::PathBuf> {
    client_core::store::AppPaths::for_profile(profile)
        .ok()
        .map(|paths| paths.emoji_cache_dir().join("stickers.json"))
}

fn load_sticker_collection(profile: &str) -> Vec<CollectedSticker> {
    sticker_collection_path(profile)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|contents| serde_json::from_str(&contents).ok())
        .unwrap_or_default()
}

/// Where the "last open room" pointer lives — a plain file in the profile
/// root holding just the room id. It sits alongside the SDK store rather than
/// in a cache dir because it's per-account session state, not a cache. Drives
/// the safe-state feature: on relaunch the app reopens whatever room was last
/// on screen for this profile.
pub fn last_room_path(profile: &str) -> Option<std::path::PathBuf> {
    client_core::store::AppPaths::for_profile(profile).ok().map(|paths| paths.root.join("last-room"))
}

/// The room id remembered from the previous session, if any. A missing or
/// blank file — never opened a room, or the last one was left — yields `None`.
fn load_last_room(profile: &str) -> Option<String> {
    let contents = std::fs::read_to_string(last_room_path(profile)?).ok()?;
    let room_id = contents.trim();
    (!room_id.is_empty()).then(|| room_id.to_string())
}

impl App {
    pub fn new(profile: String, theme: crate::theme_config::ThemeConfig) -> Self {
        let emoji_usage = load_emoji_usage(&profile);
        let sticker_collection = load_sticker_collection(&profile);
        let pending_restore_room = load_last_room(&profile);
        let built_theme = theme.to_iced_theme();
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
            settings: screens::settings::State::new(&theme),
            call: screens::call::State::default(),
            media: crate::media_cache::State::default(),
            emoji_packs: Vec::new(),
            notification_modes: std::collections::HashMap::new(),
            default_notification_modes: (
                client_core::events::NotificationMode::AllMessages,
                client_core::events::NotificationMode::AllMessages,
            ),
            theme,
            built_theme,
            privacy: {
                let privacy = crate::privacy_config::PrivacyConfig::load_or_default();
                // Features silently gated by privacy (link previews above
                // all) are a recurring "why doesn't X work" — record the
                // posture once per run so the log answers it.
                tracing::info!(
                    read_receipts = privacy.send_read_receipts,
                    typing = privacy.send_typing_notifications,
                    link_previews = privacy.enable_link_previews,
                    "privacy config loaded"
                );
                privacy
            },
            encryption: crate::encryption_config::EncryptionConfig::load_or_default(),
            spellcheck: crate::spellcheck_config::SpellcheckConfig::load_or_default(),
            chat: crate::chat_config::ChatConfig::load_or_default(),
            show_settings: false,
            cursor_position: iced::Point::ORIGIN,
            settings_panel_size: DEFAULT_SETTINGS_SIZE,
            settings_resize_drag: None,
            zoomed_image: None,
            pending_room_action: None,
            space_explorer: None,
            dm_search: None,
            url_previews: std::collections::HashMap::new(),
            tweet_previews: std::collections::HashMap::new(),
            steam_previews: std::collections::HashMap::new(),
            emoji_usage,
            emoji_shortcode_index: std::collections::HashMap::new(),
            sticker_collection,
            pending_restore_room,
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
/// see a login form flash before landing in their rooms. `theme` is loaded
/// synchronously by the caller (`main.rs`) before this runs, since it also
/// feeds the `iced::application` builder's static `.default_font()`.
/// `start_minimized` comes from the `--minimized` launch flag (autostart);
/// minimizing is queued independently of the restore task so it doesn't wait
/// on a slow network round trip before hiding the window.
pub fn boot(
    profile: String,
    theme: crate::theme_config::ThemeConfig,
    start_minimized: bool,
) -> (App, iced::Task<Message>) {
    let restore_profile = profile.clone();
    let restore_task = iced::Task::perform(
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

    let mut tasks = vec![restore_task];
    if start_minimized {
        tasks.push(iced::window::latest().and_then(|id| iced::window::minimize(id, true)));
    }

    (App::new(profile, theme), iced::Task::batch(tasks))
}
