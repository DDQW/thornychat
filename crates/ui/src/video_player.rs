//! In-app video playback for links to YouTube, Vimeo, Dailymotion, Rumble,
//! and Kick. Clicking a video card starts the platform's own iframe player
//! *inline in the chat*, in place of the card's thumbnail — no overlay, no
//! browser window.
//!
//! The player is a native WebView2 child window (wry), which always
//! composites above the wgpu surface and knows nothing about iced's
//! scrollable. Two pieces glue it into the timeline:
//!
//! * [`stage_bounds_probe`] — a widget operation that reports where the
//!   playing card's stage container currently sits: its full rect (the
//!   size the video must keep) plus the part visible through the timeline
//!   viewport. `update.rs` re-runs it after every message, so scrolls,
//!   reflows and resizes all converge on [`sync_bounds`].
//! * a "clip host" — a plain black Win32 child window sized to the
//!   *visible* slice, with the wry webview parented inside it at the full
//!   rect's offset. Children can't draw (or hit-test) outside their
//!   parent, so ordinary Win32 clipping crops the video at the viewport
//!   edges — WebView2 itself has no clip API.
//!
//! `wry::WebView` is not `Send`, and WebView2 requires the thread that owns
//! the parent window's message pump — so the instance lives in a
//! thread-local on the winit event-loop thread, and every operation
//! (open/reposition/close) is funneled through
//! `iced::window::run_with_handle`, whose closure runs on exactly that
//! thread.

use std::cell::RefCell;

use raw_window_handle::HasWindowHandle;

/// Logical size of the inline player stage (16:9), fixed so the message
/// row's height never depends on playback state beyond the card→player
/// swap itself.
pub const STAGE_WIDTH: f32 = 448.0;
pub const STAGE_HEIGHT: f32 = 252.0;

/// Widget id of the inline player's stage container. There is at most one
/// playing video, so a single well-known id links the timeline's stage
/// element to the bounds probe without threading ids around.
pub fn stage_id() -> iced::widget::container::Id {
    iced::widget::container::Id::new("inline-video-stage")
}

/// Queries the current geometry of the stage container: `(full, visible)`
/// where `full` is its scroll-translated rect and `visible` is the part
/// showing through every ancestor scrollable's viewport (`None` = scrolled
/// completely out of view). Outer `None` = no stage in the widget tree at
/// all (no video playing, message redacted/filtered away, or the probe ran
/// before the placeholder's first layout).
///
/// Modeled on `iced::widget::container::visible_bounds`, which only
/// returns the clipped rect — positioning a fixed-size webview needs the
/// unclipped rect too, hence this custom operation.
pub fn stage_bounds_probe() -> iced::Task<Option<(iced::Rectangle, Option<iced::Rectangle>)>> {
    use iced::advanced::widget::{self, operation, Operation};
    use iced::{Point, Rectangle, Size, Vector};

    type StageResult = Option<(Rectangle, Option<Rectangle>)>;

    struct StageBounds {
        target: widget::Id,
        depth: usize,
        scrollables: Vec<(Vector, Rectangle, usize)>,
        result: StageResult,
    }

    impl Operation<StageResult> for StageBounds {
        fn scrollable(
            &mut self,
            _state: &mut dyn operation::Scrollable,
            _id: Option<&widget::Id>,
            bounds: Rectangle,
            _content_bounds: Rectangle,
            translation: Vector,
        ) {
            match self.scrollables.last() {
                Some((last_translation, last_viewport, _depth)) => {
                    let viewport = last_viewport
                        .intersection(&(bounds - *last_translation))
                        .unwrap_or(Rectangle::new(Point::ORIGIN, Size::ZERO));
                    self.scrollables.push((
                        translation + *last_translation,
                        viewport,
                        self.depth,
                    ));
                }
                None => self.scrollables.push((translation, bounds, self.depth)),
            }
        }

        fn container(
            &mut self,
            id: Option<&widget::Id>,
            bounds: Rectangle,
            operate_on_children: &mut dyn FnMut(&mut dyn Operation<StageResult>),
        ) {
            if self.result.is_some() {
                return;
            }
            if id == Some(&self.target) {
                self.result = Some(match self.scrollables.last() {
                    Some((translation, viewport, _)) => {
                        let full = bounds - *translation;
                        (full, viewport.intersection(&full))
                    }
                    None => (bounds, Some(bounds)),
                });
                return;
            }

            self.depth += 1;
            operate_on_children(self);
            self.depth -= 1;

            match self.scrollables.last() {
                Some((_, _, depth)) if self.depth == *depth => {
                    let _ = self.scrollables.pop();
                }
                _ => {}
            }
        }

        fn finish(&self) -> operation::Outcome<StageResult> {
            operation::Outcome::Some(self.result)
        }
    }

    iced::advanced::widget::operate(StageBounds {
        target: stage_id().into(),
        depth: 0,
        scrollables: Vec::new(),
        result: None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    YouTube,
    Vimeo,
    Dailymotion,
    Rumble,
    /// Live-channel embed only — Kick has no officially documented way to
    /// embed a specific VOD or clip by id, so those links fall through to
    /// the regular OpenGraph preview instead of a misleading player.
    Kick,
}

impl Platform {
    pub fn label(&self) -> &'static str {
        match self {
            Platform::YouTube => "YouTube",
            Platform::Vimeo => "Vimeo",
            Platform::Dailymotion => "Dailymotion",
            Platform::Rumble => "Rumble",
            Platform::Kick => "Kick",
        }
    }

    /// Brand-ish accent for the card's left edge strip — approximate,
    /// cosmetic only.
    pub fn accent(&self) -> iced::Color {
        match self {
            Platform::YouTube => iced::Color::from_rgb8(0xFF, 0x00, 0x00),
            Platform::Vimeo => iced::Color::from_rgb8(0x1A, 0xB7, 0xEA),
            Platform::Dailymotion => iced::Color::from_rgb8(0x00, 0xAA, 0xFF),
            Platform::Rumble => iced::Color::from_rgb8(0x85, 0xC7, 0x42),
            Platform::Kick => iced::Color::from_rgb8(0x53, 0xFC, 0x18),
        }
    }
}

/// A video reference parsed out of a message-body URL, plus enough to build
/// the platform's embed URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedVideo {
    pub platform: Platform,
    pub id: String,
    pub start_seconds: u32,
    /// Vimeo's unlisted-video privacy hash (`?h=...`); unused elsewhere.
    pub vimeo_hash: Option<String>,
    /// The exact URL as it appeared in the message body — used verbatim as
    /// the "watch externally" link. Reconstructing a canonical watch URL
    /// per platform would risk drifting from whatever the platform actually
    /// wants; the original link is always correct by construction.
    pub source_url: String,
}

impl EmbedVideo {
    /// The iframe-player URL loaded into the webview.
    pub fn embed_url(&self) -> String {
        match self.platform {
            // YouTube is NOT navigated to directly — see `wrapper_url` and
            // the module docs on the referer requirement. This URL becomes
            // the iframe src inside the wrapper page.
            Platform::YouTube => youtube_embed_src(&self.id, self.start_seconds),
            // Confirmed against Vimeo's official player-parameters docs.
            // `#t=` is a URL fragment, not a query param, and must come
            // last.
            Platform::Vimeo => {
                let mut url = format!("https://player.vimeo.com/video/{}?autoplay=1", self.id);
                if let Some(hash) = &self.vimeo_hash {
                    url.push_str(&format!("&h={hash}"));
                }
                if self.start_seconds > 0 {
                    url.push_str(&format!("#t={}s", self.start_seconds));
                }
                url
            }
            // Confirmed live against Dailymotion's own oEmbed response
            // today: `geo.dailymotion.com/player.html?video={id}` needs no
            // partner Player ID, unlike the newer "Player Embed Script"
            // product their current docs otherwise push.
            Platform::Dailymotion => {
                let mut url =
                    format!("https://geo.dailymotion.com/player.html?video={}&autoplay=1", self.id);
                if self.start_seconds > 0 {
                    url.push_str(&format!("&start={}", self.start_seconds));
                }
                url
            }
            // Base embed path confirmed via Rumble's help center.
            // `autoplay=2` (muted autoplay) is a widely-used community
            // convention, not from official docs — best-effort; the
            // browser-level autoplay flag in `open` is the part that's
            // actually load-bearing.
            Platform::Rumble => format!("https://rumble.com/embed/{}/?autoplay=2", self.id),
            // Confirmed via Kick's help center (live-channel embed only).
            Platform::Kick => format!("https://player.kick.com/{}?autoplay=true", self.id),
        }
    }

    /// "Watch externally" escape hatch — always the original link.
    pub fn watch_url(&self) -> String {
        self.source_url.clone()
    }

    /// The URL the webview actually navigates to. For YouTube that's the
    /// wrapper page (served by [`wrapper_page`] via a custom protocol),
    /// not the embed URL itself: since July 2025 YouTube rejects embedded
    /// players whose requests carry no HTTP Referer as "error 153", and a
    /// top-level webview navigation has none. Injecting a Referer header
    /// on the navigation is not enough — the player's own config check
    /// reads the embedding context (`document.referrer`), which Chromium
    /// derives from a real referring page, not from smuggled headers. So
    /// the player gets what every website gives it: an actual page (with
    /// a real `https://{scheme}.localhost` origin, thanks to wry's custom
    /// protocol mapping on Windows) containing the official iframe.
    pub fn player_url(&self) -> String {
        match self.platform {
            Platform::YouTube => {
                let mut url =
                    format!("{WRAPPER_SCHEME}://localhost/player?v={}", self.id);
                if self.start_seconds > 0 {
                    url.push_str(&format!("&start={}", self.start_seconds));
                }
                url
            }
            // The other platforms don't referer-gate their embeds; keep
            // the direct navigation that has always worked for them.
            _ => self.embed_url(),
        }
    }
}

/// The YouTube iframe src — youtube-nocookie.com is YouTube's own
/// reduced-tracking embed host. autoplay works because the webview is
/// created with the no-user-gesture autoplay browser flag (see `open`).
fn youtube_embed_src(id: &str, start_seconds: u32) -> String {
    let mut url = format!("https://www.youtube-nocookie.com/embed/{id}?autoplay=1&rel=0");
    if start_seconds > 0 {
        url.push_str(&format!("&start={start_seconds}"));
    }
    url
}

/// Custom-protocol scheme the YouTube wrapper page is served from. On
/// Windows, wry surfaces `{scheme}://localhost/...` to the page as
/// `https://{scheme}.localhost/...` (see `with_https_scheme`) — a real
/// origin, which is the whole point (see [`EmbedVideo::player_url`]).
const WRAPPER_SCHEME: &str = "thornyplayer";

/// Serves the wrapper page: a black full-bleed document whose sole content
/// is the official YouTube iframe. The video id and start offset come from
/// the request's query string and are re-validated here (the id charset
/// check doubles as HTML-injection proofing, since the values are
/// interpolated into markup).
fn wrapper_page(
    request: wry::http::Request<Vec<u8>>,
) -> wry::http::Response<std::borrow::Cow<'static, [u8]>> {
    let respond = |status: u16, body: String| {
        wry::http::Response::builder()
            .status(status)
            .header(wry::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
            .body(std::borrow::Cow::Owned(body.into_bytes()))
            .expect("static response parts are valid")
    };

    let query = request.uri().query().unwrap_or("");
    let Some(id) = query_param(query, "v").filter(|id| valid_id(id, 20)) else {
        return respond(404, "missing or invalid video id".into());
    };
    let start = query_param(query, "start").and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);

    // `referrerpolicy="origin"` pins exactly what YouTube requires the
    // iframe request to carry, independent of default-policy changes.
    let html = format!(
        r#"<!doctype html>
<html><head><meta charset="utf-8">
<style>html,body{{margin:0;height:100%;background:#000;overflow:hidden}}iframe{{display:block;width:100%;height:100%;border:0}}</style>
</head><body>
<iframe src="{src}" allow="autoplay; encrypted-media; fullscreen; picture-in-picture" allowfullscreen referrerpolicy="origin"></iframe>
</body></html>"#,
        src = youtube_embed_src(&id, start),
    );
    respond(200, html)
}

/// Tries each platform's parser in turn.
pub fn video_in(url: &str) -> Option<EmbedVideo> {
    youtube_video_in(url)
        .or_else(|| vimeo_video_in(url))
        .or_else(|| dailymotion_video_in(url))
        .or_else(|| rumble_video_in(url))
        .or_else(|| kick_video_in(url))
}

/// Splits a URL into (lowercased host without a leading "www.", path,
/// query). Good enough for these platforms' link shapes — not a general
/// URL parser (no percent-decoding, no userinfo beyond stripping it).
fn split_url(url: &str) -> Option<(String, &str, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return None;
    }
    let (host, path_and_query) = match rest.split_once('/') {
        Some((host, tail)) => (host, tail),
        None => (rest, ""),
    };
    let host = host.split('@').next_back()?.split(':').next()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();

    let (path, query) = match path_and_query.split_once('?') {
        Some((path, query)) => (path, query),
        None => (path_and_query, ""),
    };
    let path = path.split('#').next().unwrap_or(path);
    let query = query.split('#').next().unwrap_or(query);
    Some((host, path, query))
}

fn query_param(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name && !value.is_empty()).then(|| value.to_string())
    })
}

fn valid_id(id: &str, max_len: usize) -> bool {
    !id.is_empty()
        && id.len() <= max_len
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// `t=90`, `t=90s`, and `t=1h2m3s` all appear in shared links.
fn parse_timestamp(value: &str) -> Option<u32> {
    if let Ok(seconds) = value.parse::<u32>() {
        return Some(seconds);
    }
    let mut total: u32 = 0;
    let mut digits = String::new();
    for c in value.chars() {
        match c {
            '0'..='9' => digits.push(c),
            'h' | 'm' | 's' => {
                let unit = match c {
                    'h' => 3600,
                    'm' => 60,
                    _ => 1,
                };
                total = total.checked_add(digits.parse::<u32>().ok()?.checked_mul(unit)?)?;
                digits.clear();
            }
            _ => return None,
        }
    }
    digits.is_empty().then_some(total)
}

fn embed_video(platform: Platform, id: String, start_seconds: u32, url: &str) -> EmbedVideo {
    EmbedVideo { platform, id, start_seconds, vimeo_hash: None, source_url: url.to_string() }
}

/// Recognizes the common YouTube URL shapes: `watch?v=`, `youtu.be/`,
/// `/shorts/`, `/live/`, `/embed/`, `/v/` — with an optional `t=`/`start=`
/// timestamp (`90`, `90s`, or `1h2m3s`).
fn youtube_video_in(url: &str) -> Option<EmbedVideo> {
    let (host, path, query) = split_url(url)?;
    let id = match host.as_str() {
        "youtu.be" => path.split('/').next().map(str::to_string),
        "youtube.com" | "m.youtube.com" | "music.youtube.com" | "youtube-nocookie.com" => {
            let mut segments = path.split('/');
            match segments.next() {
                Some("watch") => query_param(query, "v"),
                Some("shorts" | "live" | "embed" | "v") => segments.next().map(str::to_string),
                _ => None,
            }
        }
        _ => None,
    }?;
    if !valid_id(&id, 20) {
        return None;
    }
    let start_seconds = query_param(query, "t")
        .or_else(|| query_param(query, "start"))
        .and_then(|value| parse_timestamp(&value))
        .unwrap_or(0);
    Some(embed_video(Platform::YouTube, id, start_seconds, url))
}

/// `vimeo.com/{id}` (optionally `/{hash}` for unlisted videos) and
/// `player.vimeo.com/video/{id}` (people sometimes paste the embed link
/// itself, `?h=` instead of a path hash). Channel/group/showcase URLs are
/// rejected naturally since their first segment isn't purely numeric.
fn vimeo_video_in(url: &str) -> Option<EmbedVideo> {
    let (host, path, query) = split_url(url)?;
    let mut segments = path.split('/').filter(|s| !s.is_empty());
    let (id, hash) = match host.as_str() {
        "vimeo.com" => (segments.next()?.to_string(), segments.next().map(str::to_string)),
        "player.vimeo.com" => {
            if segments.next()? != "video" {
                return None;
            }
            (segments.next()?.to_string(), query_param(query, "h"))
        }
        _ => return None,
    };
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let vimeo_hash = hash.filter(|h| !h.is_empty() && h.bytes().all(|b| b.is_ascii_alphanumeric()));
    Some(EmbedVideo {
        platform: Platform::Vimeo,
        id,
        start_seconds: 0,
        vimeo_hash,
        source_url: url.to_string(),
    })
}

/// `dailymotion.com/video/{id}[_slug]` and the `dai.ly/{id}` short link —
/// the id is truncated at the first `_` (share links append a title slug).
fn dailymotion_video_in(url: &str) -> Option<EmbedVideo> {
    let (host, path, query) = split_url(url)?;
    let mut segments = path.split('/').filter(|s| !s.is_empty());
    let raw_id = match host.as_str() {
        "dailymotion.com" | "m.dailymotion.com" => match segments.next()? {
            "video" => segments.next(),
            _ => None,
        },
        "dai.ly" => segments.next(),
        _ => None,
    }?;
    let id = raw_id.split('_').next().unwrap_or(raw_id).to_string();
    if !valid_id(&id, 20) {
        return None;
    }
    let start_seconds = query_param(query, "start").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    Some(embed_video(Platform::Dailymotion, id, start_seconds, url))
}

/// A Rumble watch page is exactly one path segment ending in `.html`, e.g.
/// `v70bqqu-my-title.html` — the id is the part before the first `-`.
/// Channel/category/search pages don't have that shape, so they're
/// rejected without needing an explicit blocklist. Already-embed links
/// (`rumble.com/embed/{id}/`) are also recognized.
fn rumble_video_in(url: &str) -> Option<EmbedVideo> {
    let (host, path, _query) = split_url(url)?;
    if host != "rumble.com" {
        return None;
    }
    let mut segments = path.split('/').filter(|s| !s.is_empty());
    let first = segments.next()?;
    let id = if first == "embed" {
        segments.next().filter(|id| valid_id(id, 24)).map(str::to_string)
    } else if segments.next().is_none() {
        first
            .strip_suffix(".html")
            .and_then(|slug| slug.split('-').next())
            .filter(|id| valid_id(id, 24))
            .map(str::to_string)
    } else {
        None
    }?;
    Some(embed_video(Platform::Rumble, id, 0, url))
}

/// Live-channel pages only: `kick.com/{channel}` with no further path
/// segments (VODs/clips live under `/videos/{id}` and `/clips/{id}`, which
/// aren't matched — see the `Platform::Kick` doc comment).
fn kick_video_in(url: &str) -> Option<EmbedVideo> {
    const RESERVED: &[&str] = &[
        "categories", "browse", "search", "following", "messages", "dashboard", "settings",
        "subscriptions", "wallet", "moderator", "explore", "discover", "signup", "login",
        "terms", "privacy", "about", "contact", "app",
    ];
    let (host, path, _query) = split_url(url)?;
    if host != "kick.com" {
        return None;
    }
    let mut segments = path.split('/').filter(|s| !s.is_empty());
    let channel = segments.next()?;
    if segments.next().is_some() || RESERVED.contains(&channel) || !valid_id(channel, 25) {
        return None;
    }
    Some(embed_video(Platform::Kick, channel.to_string(), 0, url))
}

/// A logical rect rounded to physical pixels: `(x, y, width, height)`.
fn to_physical(rect: iced::Rectangle, scale: f32) -> (i32, i32, i32, i32) {
    (
        (rect.x * scale).round() as i32,
        (rect.y * scale).round() as i32,
        ((rect.width * scale).round() as i32).max(1),
        ((rect.height * scale).round() as i32).max(1),
    )
}

/// The webview's bounds *relative to the clip host's client origin*: the
/// full stage rect shifted by wherever the host sits, so the video keeps
/// its true size and the host's edges do the cropping.
fn webview_bounds(full: (i32, i32, i32, i32), host_origin: (i32, i32)) -> wry::Rect {
    wry::Rect {
        position: wry::dpi::PhysicalPosition::new(full.0 - host_origin.0, full.1 - host_origin.1)
            .into(),
        size: wry::dpi::PhysicalSize::new(full.2 as u32, full.3 as u32).into(),
    }
}

/// The clip-host window: see the module docs. All host functions must run
/// on the event-loop thread, like everything else here.
mod host {
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::Graphics::Gdi::{GetStockObject, BLACK_BRUSH, HBRUSH};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, MoveWindow, RegisterClassExW, ShowWindow,
        HMENU, SW_HIDE, SW_SHOWNA, WINDOW_EX_STYLE, WNDCLASSEXW, WS_CHILD, WS_CLIPCHILDREN,
        WS_CLIPSIBLINGS, WS_VISIBLE,
    };

    /// `DefWindowProcW` itself is generic in windows-rs, so it can't be a
    /// `WNDPROC` directly — the host needs no behavior of its own beyond
    /// the class brush anyway.
    unsafe extern "system" fn host_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    fn class_name() -> PCWSTR {
        static REGISTER: std::sync::Once = std::sync::Once::new();
        let name = w!("THORNYCHAT_VIDEO_HOST");
        REGISTER.call_once(|| unsafe {
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(host_proc),
                hInstance: GetModuleHandleW(PCWSTR::null())
                    .map(HINSTANCE::from)
                    .unwrap_or_default(),
                // Black, so the moment before WebView2's first paint reads
                // as the player stage rather than a hole in the chat.
                hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
                lpszClassName: name,
                ..Default::default()
            };
            let _ = RegisterClassExW(&class);
        });
        name
    }

    /// Creates the host as a child of the app window, at the visible slice
    /// (hidden 1×1 when the stage is currently scrolled out of view).
    pub fn create(parent: isize, visible: Option<(i32, i32, i32, i32)>) -> Result<isize, String> {
        let (x, y, width, height) = visible.unwrap_or((0, 0, 1, 1));
        let mut style = WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS;
        if visible.is_some() {
            style |= WS_VISIBLE;
        }
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class_name(),
                PCWSTR::null(),
                style,
                x,
                y,
                width,
                height,
                HWND(parent as *mut core::ffi::c_void),
                HMENU::default(),
                GetModuleHandleW(PCWSTR::null()).map(HINSTANCE::from).unwrap_or_default(),
                None,
            )
        }
        .map_err(|e| e.to_string())?;
        Ok(hwnd.0 as isize)
    }

    /// Moves/shows the host over the visible slice, or hides it (`None`).
    /// `SW_SHOWNA` — repositioning during a scroll must never move focus.
    pub fn sync(host: isize, visible: Option<(i32, i32, i32, i32)>) {
        let hwnd = HWND(host as *mut core::ffi::c_void);
        unsafe {
            match visible {
                Some((x, y, width, height)) => {
                    let _ = MoveWindow(hwnd, x, y, width, height, true);
                    let _ = ShowWindow(hwnd, SW_SHOWNA);
                }
                None => {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
            }
        }
    }

    pub fn destroy(host: isize) {
        let _ = unsafe { DestroyWindow(HWND(host as *mut core::ffi::c_void)) };
    }
}

/// Hands wry the clip host as its parent window.
struct HostHandle(std::num::NonZeroIsize);

impl HasWindowHandle for HostHandle {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        let raw = raw_window_handle::RawWindowHandle::Win32(
            raw_window_handle::Win32WindowHandle::new(self.0),
        );
        // SAFETY: the host HWND outlives this borrow — it is only destroyed
        // in `close`, after the webview built on it is dropped.
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(raw) })
    }
}

struct NativePlayer {
    webview: wry::WebView,
    /// Clip-host HWND. Destroyed *after* `webview`: wry's drop impl closes
    /// the WebView2 controller and unhooks its subclass from this window.
    host: isize,
}

thread_local! {
    /// The live player, if any. Only ever touched from the event-loop
    /// thread (see module docs) — one player at a time.
    static PLAYER: RefCell<Option<NativePlayer>> = const { RefCell::new(None) };
}

/// Creates the clip host + child webview over the stage geometry. Must run
/// on the event-loop thread (inside `window::run_with_handle`). `data_dir`
/// keeps WebView2's profile data out of the exe's directory.
///
/// Deliberately no `webview.focus()` (the old overlay player did):
/// playback starting inline must not yank keyboard focus out of the
/// composer. Clicking the video focuses it naturally.
pub fn open(
    parent: &impl HasWindowHandle,
    video: &EmbedVideo,
    full: iced::Rectangle,
    visible: Option<iced::Rectangle>,
    scale: f32,
    data_dir: Option<std::path::PathBuf>,
) -> Result<(), String> {
    close();

    let raw = parent.window_handle().map_err(|e| e.to_string())?.as_raw();
    let raw_window_handle::RawWindowHandle::Win32(parent_handle) = raw else {
        return Err("unsupported window handle".into());
    };

    let full_phys = to_physical(full, scale);
    let visible_phys = visible.map(|v| to_physical(v, scale));
    let host = host::create(parent_handle.hwnd.get(), visible_phys)?;
    let host_origin =
        visible_phys.map(|(x, y, _, _)| (x, y)).unwrap_or((full_phys.0, full_phys.1));

    let mut context = wry::WebContext::new(data_dir);
    let builder = wry::WebViewBuilder::new_with_web_context(&mut context)
        .with_bounds(webview_bounds(full_phys, host_origin))
        // YouTube plays via a wrapper page served from this protocol so
        // the player sees a real embedding page — see `player_url` for
        // the whole story. Registered unconditionally (it's inert for the
        // platforms that navigate straight to their embed URL).
        .with_custom_protocol(WRAPPER_SCHEME.to_string(), |_webview_id, request| {
            wrapper_page(request)
        })
        .with_url(video.player_url());

    #[cfg(target_os = "windows")]
    let builder = {
        use wry::WebViewBuilderExtWindows;
        builder
            // Surface the wrapper as https://thornyplayer.localhost — an
            // https origin makes the iframe's referer situation identical
            // to a normal website embedding YouTube.
            .with_https_scheme(true)
            .with_additional_browser_args("--autoplay-policy=no-user-gesture-required")
    };

    let host_handle = match std::num::NonZeroIsize::new(host) {
        Some(hwnd) => HostHandle(hwnd),
        None => return Err("clip host window handle was null".into()),
    };
    let webview = match builder.build_as_child(&host_handle) {
        Ok(webview) => webview,
        Err(e) => {
            host::destroy(host);
            return Err(e.to_string());
        }
    };
    PLAYER.with(|player| *player.borrow_mut() = Some(NativePlayer { webview, host }));
    Ok(())
}

/// Reglues the live player to freshly probed stage geometry: host window
/// to the visible slice (hidden when `None` — scrolled away or an iced
/// overlay is open), webview offset so the full rect stays put underneath.
/// Event-loop thread only; harmless when no player is live.
pub fn sync_bounds(full: iced::Rectangle, visible: Option<iced::Rectangle>, scale: f32) {
    PLAYER.with(|player| {
        let borrowed = player.borrow();
        let Some(native) = borrowed.as_ref() else {
            return;
        };
        let visible_phys = visible.map(|v| to_physical(v, scale));
        host::sync(native.host, visible_phys);
        if let Some((x, y, _, _)) = visible_phys {
            let _ = native.webview.set_bounds(webview_bounds(to_physical(full, scale), (x, y)));
        }
    });
}

/// Hides the player without stopping playback — used while the stage
/// container is transiently unlocatable. Event-loop thread only.
pub fn hide() {
    PLAYER.with(|player| {
        if let Some(native) = player.borrow().as_ref() {
            host::sync(native.host, None);
        }
    });
}

/// Tears down the player (stops audio/video). Event-loop thread only;
/// harmless when no player is live.
pub fn close() {
    PLAYER.with(|player| {
        if let Some(native) = player.borrow_mut().take() {
            // Order matters — see `NativePlayer::host`.
            drop(native.webview);
            host::destroy(native.host);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_youtube_shapes() {
        for (url, id, start) in [
            ("https://www.youtube.com/watch?v=dQw4w9WgXcQ", "dQw4w9WgXcQ", 0),
            ("https://youtu.be/dQw4w9WgXcQ?t=43", "dQw4w9WgXcQ", 43),
            ("http://m.youtube.com/watch?v=abc-DEF_123&t=1h2m3s", "abc-DEF_123", 3723),
            ("https://www.youtube.com/shorts/abc-DEF_123", "abc-DEF_123", 0),
            ("https://www.youtube.com/live/abc-DEF_123?feature=share", "abc-DEF_123", 0),
            ("https://music.youtube.com/watch?v=abc-DEF_123", "abc-DEF_123", 0),
            ("https://www.youtube-nocookie.com/embed/abc-DEF_123?start=90s", "abc-DEF_123", 90),
        ] {
            let video = video_in(url).unwrap_or_else(|| panic!("no match: {url}"));
            assert_eq!(video.platform, Platform::YouTube, "{url}");
            assert_eq!(video.id, id, "{url}");
            assert_eq!(video.start_seconds, start, "{url}");
        }
    }

    #[test]
    fn recognizes_vimeo_shapes() {
        let video = video_in("https://vimeo.com/76979871").unwrap();
        assert_eq!(video.platform, Platform::Vimeo);
        assert_eq!(video.id, "76979871");
        assert_eq!(video.vimeo_hash, None);

        let unlisted = video_in("https://vimeo.com/1039818823/73f8e67672").unwrap();
        assert_eq!(unlisted.id, "1039818823");
        assert_eq!(unlisted.vimeo_hash.as_deref(), Some("73f8e67672"));

        let embed_link = video_in("https://player.vimeo.com/video/76979871?h=abc123").unwrap();
        assert_eq!(embed_link.id, "76979871");
        assert_eq!(embed_link.vimeo_hash.as_deref(), Some("abc123"));

        assert!(video_in("https://vimeo.com/channels/staffpicks").is_none());
    }

    #[test]
    fn recognizes_dailymotion_shapes() {
        let video =
            video_in("https://www.dailymotion.com/video/x84sh87_dailymotion-demo_tech").unwrap();
        assert_eq!(video.platform, Platform::Dailymotion);
        assert_eq!(video.id, "x84sh87");

        let short = video_in("https://dai.ly/x84sh87").unwrap();
        assert_eq!(short.id, "x84sh87");
    }

    #[test]
    fn recognizes_rumble_shapes() {
        let video = video_in("https://rumble.com/v70bqqu-some-title-here.html").unwrap();
        assert_eq!(video.platform, Platform::Rumble);
        assert_eq!(video.id, "v70bqqu");

        let embed_link = video_in("https://rumble.com/embed/v70bqqu/").unwrap();
        assert_eq!(embed_link.id, "v70bqqu");

        assert!(video_in("https://rumble.com/c/SomeChannel").is_none());
        assert!(video_in("https://rumble.com/search/video?q=cats").is_none());
    }

    #[test]
    fn recognizes_kick_channel_only() {
        let video = video_in("https://kick.com/somestreamer").unwrap();
        assert_eq!(video.platform, Platform::Kick);
        assert_eq!(video.id, "somestreamer");

        // No confirmed VOD/clip embed — must not misfire the live player.
        assert!(video_in("https://kick.com/somestreamer/videos/abc-123").is_none());
        assert!(video_in("https://kick.com/categories/gaming").is_none());
    }

    #[test]
    fn rejects_non_video_urls() {
        for url in [
            "https://www.youtube.com/@somechannel",
            "https://www.youtube.com/playlist?list=PL123",
            "https://notyoutube.com/watch?v=dQw4w9WgXcQ",
            "https://example.com/https://youtube.com/watch?v=x",
            "ftp://youtube.com/watch?v=dQw4w9WgXcQ",
            "https://example.com/",
        ] {
            assert!(video_in(url).is_none(), "should not match: {url}");
        }
    }
}
