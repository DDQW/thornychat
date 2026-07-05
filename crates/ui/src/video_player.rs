//! In-app video playback for links to YouTube, Vimeo, Dailymotion, Rumble,
//! and Kick. Clicking a video card opens a lightbox-style overlay whose
//! video area is a native WebView2 child window (wry) hosting the
//! platform's own iframe player — no browser window is launched.
//!
//! The webview is a Win32 child HWND of the iced window, so it always
//! composites above the wgpu surface; the iced side draws the backdrop,
//! header, and a placeholder exactly under the rect the webview occupies
//! (`video_rect` is the single source of truth for that geometry).
//!
//! `wry::WebView` is not `Send`, and WebView2 requires the thread that owns
//! the parent window's message pump — so the instance lives in a
//! thread-local on the winit event-loop thread, and every operation
//! (open/reposition/close) is funneled through
//! `iced::window::run_with_handle`, whose closure runs on exactly that
//! thread.

use std::cell::RefCell;

use raw_window_handle::HasWindowHandle;

/// Header row drawn above the video inside the centered overlay block.
pub const HEADER_HEIGHT: f32 = 36.0;
pub const HEADER_GAP: f32 = 8.0;

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
            // youtube-nocookie.com is YouTube's own reduced-tracking embed
            // host. autoplay works because the webview is created with the
            // no-user-gesture autoplay browser flag (see `open`).
            Platform::YouTube => {
                let mut url = format!(
                    "https://www.youtube-nocookie.com/embed/{}?autoplay=1&rel=0",
                    self.id
                );
                if self.start_seconds > 0 {
                    url.push_str(&format!("&start={}", self.start_seconds));
                }
                url
            }
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

/// Logical (pre-DPI) rectangle the video surface occupies within the
/// window: a centered 16:9 box with room for the header row above and
/// breathing space around. Both the iced overlay layout and the native
/// webview bounds derive from this one function so they can't drift apart.
pub fn video_rect(window: iced::Size) -> iced::Rectangle {
    let max_width = (window.width - 120.0).clamp(320.0, 1280.0);
    let max_height = (window.height - HEADER_HEIGHT - HEADER_GAP - 100.0).max(180.0);

    let mut width = max_width;
    let mut height = width * 9.0 / 16.0;
    if height > max_height {
        height = max_height;
        width = (height * 16.0 / 9.0).max(320.0);
    }

    let block_height = HEADER_HEIGHT + HEADER_GAP + height;
    iced::Rectangle {
        x: (window.width - width) / 2.0,
        y: (window.height - block_height) / 2.0 + HEADER_HEIGHT + HEADER_GAP,
        width,
        height,
    }
}

fn physical_bounds(rect: iced::Rectangle, scale: f32) -> wry::Rect {
    wry::Rect {
        position: wry::dpi::PhysicalPosition::new(
            (rect.x * scale).round() as i32,
            (rect.y * scale).round() as i32,
        )
        .into(),
        size: wry::dpi::PhysicalSize::new(
            (rect.width * scale).round().max(1.0) as u32,
            (rect.height * scale).round().max(1.0) as u32,
        )
        .into(),
    }
}

thread_local! {
    /// The live player, if any. Only ever touched from the event-loop
    /// thread (see module docs) — one player at a time.
    static PLAYER: RefCell<Option<wry::WebView>> = const { RefCell::new(None) };
}

/// Creates the child webview over `rect`. Must run on the event-loop
/// thread (inside `window::run_with_handle`). `data_dir` keeps WebView2's
/// profile data out of the exe's directory.
pub fn open(
    parent: &impl HasWindowHandle,
    video: &EmbedVideo,
    rect: iced::Rectangle,
    scale: f32,
    data_dir: Option<std::path::PathBuf>,
) -> Result<(), String> {
    close();

    let mut context = wry::WebContext::new(data_dir);
    let builder = wry::WebViewBuilder::new_with_web_context(&mut context)
        .with_url(video.embed_url())
        .with_bounds(physical_bounds(rect, scale));

    #[cfg(target_os = "windows")]
    let builder = {
        use wry::WebViewBuilderExtWindows;
        builder.with_additional_browser_args("--autoplay-policy=no-user-gesture-required")
    };

    let webview = builder.build_as_child(parent).map_err(|e| e.to_string())?;
    let _ = webview.focus();
    PLAYER.with(|player| *player.borrow_mut() = Some(webview));
    Ok(())
}

/// Repositions the live player (window resized). Event-loop thread only.
pub fn set_bounds(rect: iced::Rectangle, scale: f32) {
    PLAYER.with(|player| {
        if let Some(webview) = player.borrow().as_ref() {
            let _ = webview.set_bounds(physical_bounds(rect, scale));
        }
    });
}

/// Tears down the player (stops audio/video). Event-loop thread only;
/// harmless when no player is live.
pub fn close() {
    PLAYER.with(|player| {
        let _ = player.borrow_mut().take();
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
