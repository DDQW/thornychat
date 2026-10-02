//! In-app video playback for links to YouTube, Vimeo, Dailymotion, Rumble,
//! Kick, and direct links to video files (`.mp4`, `.webm`, ...). Clicking a
//! video card starts the platform's own iframe player — or, for a direct
//! file, a plain `<video>` tag — *inline in the chat*, in place of the
//! card's thumbnail — no overlay, no browser window.
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

use std::cell::{Cell, RefCell};

use raw_window_handle::HasWindowHandle;

/// Logical size of the inline player stage (16:9), fixed so the message
/// row's height never depends on playback state beyond the card→player
/// swap itself.
pub const STAGE_WIDTH: f32 = 448.0;
pub const STAGE_HEIGHT: f32 = 252.0;

/// Widget id of the inline player's stage container. There is at most one
/// playing video, so a single well-known id links the timeline's stage
/// element to the bounds probe without threading ids around.
pub fn stage_id() -> iced::widget::Id {
    iced::widget::Id::new("inline-video-stage")
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
        // (viewport, translation) saved on entry to each node, restored on
        // exit — mirrors iced 0.14's own `selector` traversal, which drives
        // child recursion through `traverse` rather than a per-container
        // callback. A sentinel base entry keeps `last().unwrap()` valid.
        stack: Vec<(Rectangle, Vector)>,
        viewport: Rectangle,
        translation: Vector,
        result: StageResult,
    }

    impl Operation<StageResult> for StageBounds {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<StageResult>)) {
            if self.result.is_some() {
                return;
            }
            self.stack.push((self.viewport, self.translation));
            operate(self);
            let _ = self.stack.pop();
            let (viewport, translation) = *self.stack.last().unwrap();
            self.viewport = viewport;
            self.translation = translation;
        }

        fn scrollable(
            &mut self,
            _id: Option<&widget::Id>,
            bounds: Rectangle,
            _content_bounds: Rectangle,
            translation: Vector,
            _state: &mut dyn operation::Scrollable,
        ) {
            if self.result.is_some() {
                return;
            }
            // The scrollable's viewport (clipped by ancestors) and its content
            // translation apply to everything nested under it, until `traverse`
            // pops back out of this subtree.
            let visible = self
                .viewport
                .intersection(&(bounds + self.translation))
                .unwrap_or(Rectangle::new(Point::ORIGIN, Size::ZERO));
            self.translation -= translation;
            self.viewport = visible;
        }

        fn container(&mut self, id: Option<&widget::Id>, bounds: Rectangle) {
            if self.result.is_some() {
                return;
            }
            if id == Some(&self.target) {
                // `full` is the stage's scroll-translated on-screen rect (needed
                // to place the fixed-size webview even when partly scrolled off);
                // the intersection is the part actually showing through.
                let full = bounds + self.translation;
                self.result = Some((full, self.viewport.intersection(&full)));
            }
        }

        fn finish(&self) -> operation::Outcome<StageResult> {
            operation::Outcome::Some(self.result)
        }
    }

    iced::advanced::widget::operate(StageBounds {
        target: stage_id(),
        stack: vec![(Rectangle::INFINITE, Vector::ZERO)],
        viewport: Rectangle::INFINITE,
        translation: Vector::ZERO,
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
    /// Not a hosting platform at all — a direct link to a video *file*
    /// (`.mp4`, `.webm`, ...). Reuses the same card/label/accent plumbing
    /// as the platforms above; only `embed_url`/`player_url` differ in how
    /// they turn it into something playable.
    File,
    /// A video *attachment* (`m.video`) — not something the webview could
    /// load by URL at all, since Matrix media needs an authenticated
    /// download. The bytes land in the media cache first and are then
    /// streamed off disk through the wrapper protocol (see [`LocalVideo`]).
    Attachment,
}

impl Platform {
    pub fn label(&self) -> &'static str {
        match self {
            Platform::YouTube => "YouTube",
            Platform::Vimeo => "Vimeo",
            Platform::Dailymotion => "Dailymotion",
            Platform::Rumble => "Rumble",
            Platform::Kick => "Kick",
            Platform::File | Platform::Attachment => "Video",
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
            // No brand to match — a neutral gray.
            Platform::File | Platform::Attachment => iced::Color::from_rgb8(0x8A, 0x8A, 0x8A),
        }
    }
}

/// The downloaded file behind a `Platform::Attachment` video, plus the
/// content type to serve it as. The wrapper protocol streams it straight
/// off disk, range requests and all, so seeking works and no copy of the
/// video is ever held in memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalVideo {
    pub path: std::path::PathBuf,
    /// The sender's declared `info.mimetype` when the event carried one,
    /// else a guess from the filename. Chromium picks its decoder from
    /// this, and it can't sniff one from the media cache's own filename —
    /// cache entries are named after the mxc URL, extension and all
    /// stripped.
    pub mime: String,
}

/// A video reference parsed out of a message-body URL, plus enough to build
/// the platform's embed URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedVideo {
    pub platform: Platform,
    /// Unused (empty) for `Platform::File`/`Attachment` — the source alone
    /// is playable, there's no separate video id to track.
    pub id: String,
    pub start_seconds: u32,
    /// Vimeo's unlisted-video privacy hash (`?h=...`); unused elsewhere.
    pub vimeo_hash: Option<String>,
    /// The exact URL as it appeared in the message body — used verbatim as
    /// the "watch externally" link. Reconstructing a canonical watch URL
    /// per platform would risk drifting from whatever the platform actually
    /// wants; the original link is always correct by construction. A
    /// `Platform::Attachment` has no such URL: it holds the local path
    /// instead, which is exactly what "open externally" wants there (the
    /// system's own video player).
    pub source_url: String,
    /// `Platform::Attachment` only; `None` for everything that plays from a
    /// URL.
    pub local: Option<LocalVideo>,
}

impl EmbedVideo {
    /// A downloaded `m.video` attachment, played from disk.
    pub fn attachment(path: std::path::PathBuf, mime: String) -> Self {
        Self {
            platform: Platform::Attachment,
            id: String::new(),
            start_seconds: 0,
            vimeo_hash: None,
            source_url: path.to_string_lossy().into_owned(),
            local: Some(LocalVideo { path, mime }),
        }
    }

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
                // `title=0&byline=0&portrait=0` drop the title/author/avatar
                // overlay chrome; `dnt=1` opts out of Vimeo's tracking
                // cookies. No official per-icon toggle exists for the
                // share/"copy link" button short of `controls=0`, which
                // would remove playback controls entirely — not worth it.
                let mut url = format!(
                    "https://player.vimeo.com/video/{}?autoplay=1&title=0&byline=0&portrait=0&dnt=1",
                    self.id
                );
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
                // `sharing-enable=false` drops the share/"copy link" button;
                // `endscreen-enable=false` and `queue-enable=false` drop the
                // "up next" suggestion grid; `ui-logo=false` is a bonus
                // branding removal. All confirmed Dailymotion player params.
                let mut url = format!(
                    "https://geo.dailymotion.com/player.html?video={}&autoplay=1&sharing-enable=false&endscreen-enable=false&queue-enable=false&ui-logo=false",
                    self.id
                );
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
            // The file itself is the "embed" — there's no separate player
            // page. `player_url` doesn't actually reach this arm (it routes
            // through the wrapper instead, for consistent styling) but it's
            // kept correct/total here regardless.
            Platform::File => self.source_url.clone(),
            // Streamed from the wrapper protocol, never from an address of
            // its own — `player_url` is the only meaningful one here.
            Platform::Attachment => self.player_url(),
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
            // No referer to satisfy, but routed through the same wrapper
            // trick as YouTube (rather than a raw navigation) so playback
            // looks consistent — black backdrop, video filling the stage —
            // instead of whatever Chromium's default media-document viewer
            // does with an arbitrary video's native size.
            Platform::File => {
                format!("{WRAPPER_SCHEME}://localhost/video?src={}", percent_encode(&self.source_url))
            }
            // The same wrapper, except the `<video>` inside it points back
            // at this protocol's own `/attachment/stream` instead of a
            // remote URL. The `v=` token is a cache-buster: every player
            // navigates to the same two paths, and one video must never be
            // able to answer for the next one out of some response cache
            // along the way.
            Platform::Attachment => {
                format!("{WRAPPER_SCHEME}://localhost/attachment?v={}", self.attachment_token())
            }
            // The other platforms don't referer-gate their embeds; keep
            // the direct navigation that has always worked for them.
            _ => self.embed_url(),
        }
    }

    /// Filename from the URL path, used as a card title when there's no OG
    /// title to show — the common case for a raw file link, since there's
    /// no page to scrape one from. `None` for hosted platforms, where an OG
    /// title (once resolved) is always preferred over an opaque video id.
    pub fn file_name(&self) -> Option<String> {
        (self.platform == Platform::File).then(|| path_file_name(&self.source_url)).flatten().map(str::to_string)
    }

    /// Per-file cache-buster for the attachment wrapper's URLs: an FNV-1a
    /// hash of the path, which is unique per mxc URL since the media cache
    /// names its entries after them.
    fn attachment_token(&self) -> String {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in self.source_url.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("{hash:016x}")
    }
}

/// The YouTube iframe src — youtube-nocookie.com is YouTube's own
/// reduced-tracking embed host. autoplay works because the webview is
/// created with the no-user-gesture autoplay browser flag (see `open`).
/// `rel=0` restricts end-screen suggestions to the same channel (YouTube
/// removed the option to disable them entirely in 2018 — this is the
/// ceiling); `modestbranding=1` and `iv_load_policy=3` trim branding and
/// annotations; `disablekb=1` defers to the app's own shortcuts;
/// `playsinline=1` keeps playback inline. Deliberately no `fs=0` — that
/// would disable the fullscreen button the wrapper page/`subscribe_fullscreen`
/// machinery exists to support.
fn youtube_embed_src(id: &str, start_seconds: u32) -> String {
    let mut url = format!(
        "https://www.youtube-nocookie.com/embed/{id}?autoplay=1&rel=0&modestbranding=1&iv_load_policy=3&disablekb=1&playsinline=1"
    );
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

/// Reports the wrapper document's own fullscreen state to the host over the
/// `window.ipc` bridge wry injects. Exiting fullscreen always ripples a
/// `fullscreenchange` event up to this top-level document — even though the
/// element that actually went fullscreen is the nested YouTube iframe (or,
/// for a direct file, the `<video>` tag) — because the iframe element itself
/// becomes *this* document's fullscreen element per spec. That makes it a
/// reliable second signal alongside WebView2's own
/// `ContainsFullScreenElementChanged` (see `subscribe_fullscreen`), which in
/// practice fires on entering fullscreen but not always on exit — leaving
/// the platform's own "exit fullscreen" button looking dead.
const FULLSCREEN_WATCHER_SCRIPT: &str = r#"<script>document.addEventListener('fullscreenchange',()=>{window.ipc.postMessage(document.fullscreenElement?'fs:1':'fs:0');});</script>"#;

/// Dispatches the wrapper-page custom protocol by path: `/player` is
/// YouTube's referer-satisfying wrapper, `/video` is the plain `<video>`
/// wrapper for direct file links, and `/attachment[/stream]` is that same
/// wrapper plus the bytes for a downloaded `m.video` attachment.
///
/// `local` is the attachment this player was opened for, captured when the
/// webview was built (see [`open`]) — the stream path serves that one file
/// and nothing else, so no page, iframe or embed loaded here can name a
/// path of its own to read.
fn wrapper_response(
    request: wry::http::Request<Vec<u8>>,
    local: Option<&LocalVideo>,
) -> wry::http::Response<std::borrow::Cow<'static, [u8]>> {
    match request.uri().path() {
        "/video" => file_wrapper_page(request),
        "/attachment" => attachment_wrapper_page(request, local),
        "/attachment/stream" => attachment_stream(&request, local),
        _ => youtube_wrapper_page(request),
    }
}

fn html_response(status: u16, body: String) -> wry::http::Response<std::borrow::Cow<'static, [u8]>> {
    wry::http::Response::builder()
        .status(status)
        .header(wry::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(std::borrow::Cow::Owned(body.into_bytes()))
        .expect("static response parts are valid")
}

/// Serves the wrapper page: a black full-bleed document whose sole content
/// is the official YouTube iframe. The video id and start offset come from
/// the request's query string and are re-validated here (the id charset
/// check doubles as HTML-injection proofing, since the values are
/// interpolated into markup).
fn youtube_wrapper_page(
    request: wry::http::Request<Vec<u8>>,
) -> wry::http::Response<std::borrow::Cow<'static, [u8]>> {
    let query = request.uri().query().unwrap_or("");
    let Some(id) = query_param(query, "v").filter(|id| valid_id(id, 20)) else {
        return html_response(404, "missing or invalid video id".into());
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
{watcher}
</body></html>"#,
        src = youtube_embed_src(&id, start),
        watcher = FULLSCREEN_WATCHER_SCRIPT,
    );
    html_response(200, html)
}

/// Serves the wrapper page for a direct video-file link: a black full-bleed
/// document containing a plain HTML5 `<video>` tag pointing at the original
/// URL. Same custom-protocol trick as the YouTube wrapper above, but here
/// it's purely cosmetic (no referer to satisfy) — it keeps the black
/// backdrop and contained sizing consistent with every other platform
/// instead of Chromium's default media-document viewer. The URL is
/// percent-decoded and re-validated as http(s) (never trust the round trip
/// through a request we technically built ourselves), then HTML-escaped
/// before interpolation — it can't be restricted to a safe charset like the
/// YouTube id above, since URLs need `/`, `:`, `?`, ...
fn file_wrapper_page(
    request: wry::http::Request<Vec<u8>>,
) -> wry::http::Response<std::borrow::Cow<'static, [u8]>> {
    let query = request.uri().query().unwrap_or("");
    let src = query_param(query, "src")
        .and_then(|encoded| percent_decode(&encoded))
        .filter(|url| split_url(url).is_some());
    let Some(src) = src else {
        return html_response(404, "missing or invalid video url".into());
    };

    let html = format!(
        r#"<!doctype html>
<html><head><meta charset="utf-8">
<style>html,body{{margin:0;height:100%;background:#000;overflow:hidden}}video{{display:block;width:100%;height:100%;object-fit:contain}}</style>
</head><body>
<video src="{src}" autoplay controls></video>
{watcher}
</body></html>"#,
        src = html_escape(&src),
        watcher = FULLSCREEN_WATCHER_SCRIPT,
    );
    html_response(200, html)
}

/// Serves the wrapper page for a downloaded attachment: the same black
/// full-bleed `<video>` document as [`file_wrapper_page`], pointed at
/// `/attachment/stream` on this very protocol (same origin, so no CORS
/// dance) instead of at a remote URL. The `v=` cache-buster is carried
/// over from the page's own URL so the stream is unique per file too.
fn attachment_wrapper_page(
    request: wry::http::Request<Vec<u8>>,
    local: Option<&LocalVideo>,
) -> wry::http::Response<std::borrow::Cow<'static, [u8]>> {
    if local.is_none() {
        return html_response(404, "no attachment is playing".into());
    }
    let token = query_param(request.uri().query().unwrap_or(""), "v")
        .filter(|token| valid_id(token, 32))
        .unwrap_or_default();

    let html = format!(
        r#"<!doctype html>
<html><head><meta charset="utf-8">
<style>html,body{{margin:0;height:100%;background:#000;overflow:hidden}}video{{display:block;width:100%;height:100%;object-fit:contain}}</style>
</head><body>
<video src="/attachment/stream?v={token}" autoplay controls></video>
{watcher}
</body></html>"#,
        watcher = FULLSCREEN_WATCHER_SCRIPT,
    );
    html_response(200, html)
}

/// How much of the file one range response may carry. Chromium asks for
/// "everything from here on" (`bytes=0-`) and is perfectly happy with a
/// shorter answer, so this caps the handler's peak allocation instead of
/// materializing a whole feature-length video per request.
const STREAM_CHUNK_BYTES: u64 = 8 * 1024 * 1024;

/// Streams the playing attachment off disk, honoring `Range` so the
/// player's scrub bar works (a 200-only response leaves Chromium unable to
/// seek). Errors return a bare status — the only thing the user can see of
/// them is a player that doesn't start, which the timeline's playing card
/// already has an "open externally" fallback for.
fn attachment_stream(
    request: &wry::http::Request<Vec<u8>>,
    local: Option<&LocalVideo>,
) -> wry::http::Response<std::borrow::Cow<'static, [u8]>> {
    use std::io::{Read, Seek, SeekFrom};

    let status_only = |status: u16| {
        wry::http::Response::builder()
            .status(status)
            .body(std::borrow::Cow::Owned(Vec::new()))
            .expect("static response parts are valid")
    };

    let Some(local) = local else { return status_only(404) };
    let (Ok(mut file), Ok(metadata)) =
        (std::fs::File::open(&local.path), std::fs::metadata(&local.path))
    else {
        return status_only(404);
    };
    let len = metadata.len();

    let requested = request
        .headers()
        .get(wry::http::header::RANGE)
        .and_then(|value| value.to_str().ok())
        .map(|value| parse_byte_range(value, len));
    // A `Range` header we couldn't make sense of is answered per RFC 9110
    // rather than silently served whole — a player that asked for a slice
    // must not be handed the file and left to think it got one.
    let range = match requested {
        Some(None) => {
            return wry::http::Response::builder()
                .status(416)
                .header(wry::http::header::CONTENT_RANGE, format!("bytes */{len}"))
                .body(std::borrow::Cow::Owned(Vec::new()))
                .expect("static response parts are valid");
        }
        Some(Some(range)) => Some(range),
        None => None,
    };

    let (start, end) = range.unwrap_or((0, len.saturating_sub(1)));
    let end = end.min(start.saturating_add(STREAM_CHUNK_BYTES - 1));
    let count = end.saturating_sub(start) + 1;

    let mut body = vec![0u8; count as usize];
    if file.seek(SeekFrom::Start(start)).is_err() || file.read_exact(&mut body).is_err() {
        return status_only(500);
    }

    let mut response = wry::http::Response::builder()
        .header(wry::http::header::CONTENT_TYPE, local.mime.clone())
        .header(wry::http::header::ACCEPT_RANGES, "bytes")
        .header(wry::http::header::CONTENT_LENGTH, count.to_string())
        // The file lives in the media cache already; a second copy in the
        // webview's HTTP cache would double every video's disk cost.
        .header(wry::http::header::CACHE_CONTROL, "no-store");
    // Partial whenever a range was asked for, and also when the cap above
    // shortened a full-file response — claiming 200 for part of a file
    // would truncate playback.
    if range.is_some() || count < len {
        response = response
            .status(206)
            .header(wry::http::header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}"));
    }
    response.body(std::borrow::Cow::Owned(body)).expect("static response parts are valid")
}

/// Parses a `Range: bytes=…` header against a known length into an
/// inclusive `(start, end)`. `None` means unsatisfiable or malformed (a 416
/// for the caller). Only the first range of a multi-range request is
/// honored — Chromium's media stack never sends one, and answering the
/// first slice is still a valid response.
fn parse_byte_range(header: &str, len: u64) -> Option<(u64, u64)> {
    let spec = header.trim().strip_prefix("bytes=")?.split(',').next()?.trim();
    let (first, last) = spec.split_once('-')?;
    let (start, end) = match (first.trim(), last.trim()) {
        // `bytes=-500`: the *last* 500 bytes, not "up to byte 500".
        ("", suffix) => {
            let suffix: u64 = suffix.parse().ok()?;
            if suffix == 0 {
                return None;
            }
            (len.saturating_sub(suffix), len.checked_sub(1)?)
        }
        (first, "") => (first.parse().ok()?, len.checked_sub(1)?),
        (first, last) => {
            (first.parse().ok()?, last.parse::<u64>().ok()?.min(len.checked_sub(1)?))
        }
    };
    (start <= end).then_some((start, end))
}

/// Content type for a video attachment whose event declared none —
/// guessed from the filename, defaulting to mp4 (overwhelmingly the most
/// common thing anyone sends). Only what Chromium can actually decode
/// matters here; a container it can't play fails the same either way.
pub fn mime_for_video(filename: &str) -> String {
    let extension = filename.rsplit('.').next().unwrap_or_default().to_ascii_lowercase();
    match extension.as_str() {
        "webm" => "video/webm",
        "ogv" | "ogg" => "video/ogg",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "m4v" => "video/x-m4v",
        _ => "video/mp4",
    }
    .to_string()
}

/// Tries each platform's parser in turn.
pub fn video_in(url: &str) -> Option<EmbedVideo> {
    youtube_video_in(url)
        .or_else(|| vimeo_video_in(url))
        .or_else(|| dailymotion_video_in(url))
        .or_else(|| rumble_video_in(url))
        .or_else(|| kick_video_in(url))
        .or_else(|| direct_file_video_in(url))
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

/// The final `/`-segment of a URL's path — used both to sniff a file
/// extension and, once matched, as a fallback card title.
fn path_file_name(url: &str) -> Option<&str> {
    let (_, path, _) = split_url(url)?;
    path.rsplit('/').next().filter(|s| !s.is_empty())
}

/// Percent-encodes every byte outside RFC 3986's unreserved set. Used to
/// smuggle an arbitrary URL through a single query-string value — not a
/// general encoder, just enough that `query_param`'s naive `&`/`=` split
/// can never misfire on the result (see `percent_decode`, its inverse).
fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Inverse of `percent_encode`. Fails closed (`None`) on any malformed
/// escape rather than panicking or lossily substituting.
fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                out.push(u8::from_str_radix(input.get(i + 1..i + 3)?, 16).ok()?);
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// Escapes the characters that matter inside an HTML attribute value.
/// `valid_id` keeps the other wrapper page safe by restricting to an
/// alphanumeric charset; a URL can't be restricted like that (it needs
/// `/`, `:`, `?`, ...), so this is the file wrapper's equivalent guard
/// against breaking out of the `src="..."` attribute it's interpolated into.
fn html_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
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
    EmbedVideo {
        platform,
        id,
        start_seconds,
        vimeo_hash: None,
        source_url: url.to_string(),
        local: None,
    }
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
        local: None,
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

/// A direct link to a video *file* rather than a hosting platform's page —
/// e.g. someone pasting a CDN link straight to a `.mp4`. Matched purely on
/// the URL's file extension, like the platform matchers above: no network
/// round-trip just to decide whether a play button belongs on the card.
fn direct_file_video_in(url: &str) -> Option<EmbedVideo> {
    // Restricted to containers/codecs WebView2's Chromium engine decodes
    // out of the box. `.mov`/`.mkv`/`.avi` are deliberately left out —
    // Chromium has no built-in demuxer for those, so they'd silently fail
    // to play instead of degrading to a normal link.
    const FILE_VIDEO_EXTENSIONS: &[&str] = &["mp4", "m4v", "webm", "ogv"];

    let name = path_file_name(url)?;
    let ext = name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase())?;
    FILE_VIDEO_EXTENSIONS
        .contains(&ext.as_str())
        .then(|| embed_video(Platform::File, String::new(), 0, url))
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
        SW_HIDE, SW_SHOWNA, WINDOW_EX_STYLE, WNDCLASSEXW, WS_CHILD, WS_CLIPCHILDREN,
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
                Some(HWND(parent as *mut core::ffi::c_void)),
                None,
                GetModuleHandleW(PCWSTR::null()).ok().map(HINSTANCE::from),
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
    /// True while a page element is fullscreen (the player fills the whole
    /// app window). `sync_bounds` leaves the geometry alone until it clears
    /// so the stage-glue loop can't shrink the video back mid-fullscreen.
    static FULLSCREEN: Cell<bool> = const { Cell::new(false) };
    /// The last `(full, visible)` physical rects `sync_bounds` computed, so
    /// leaving fullscreen can restore the glued geometry immediately —
    /// without waiting on the iced probe loop, whose `synced` guard may
    /// suppress a re-sync when the stage hasn't otherwise moved.
    static LAST_GLUED: Cell<Option<Glued>> = const { Cell::new(None) };
}

/// `(full, visible)` in physical pixels — the inputs `apply_glued` needs.
type Glued = ((i32, i32, i32, i32), Option<(i32, i32, i32, i32)>);

/// Creates the clip host + child webview over the stage geometry. Must run
/// on the event-loop thread (inside `window::run_with_handle`). `data_dir`
/// keeps WebView2's profile data out of the exe's directory.
///
/// Deliberately no `webview.focus()` (the old overlay player did):
/// playback starting inline must not yank keyboard focus out of the
/// composer. Clicking the video focuses it naturally.
pub fn open(
    parent: &(impl HasWindowHandle + ?Sized),
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
        // Don't seize keyboard focus when playback starts — the composer has
        // to stay typeable while a video plays inline. wry defaults
        // `focused: true`, which `MoveFocus`es to the WebView2 document at
        // creation and leaves winit (and thus the iced text_input) unable to
        // receive WM_CHAR. Clicking the video still focuses it for player
        // shortcuts (wry's container forwards WM_SETFOCUS to the document);
        // `reclaim_focus` hands focus back on the next app-surface click.
        .with_focused(false)
        // YouTube and direct file links both play via a wrapper page served
        // from this protocol — see `player_url` for the whole story.
        // Registered unconditionally (it's inert for the platforms that
        // navigate straight to their embed URL).
        .with_custom_protocol(WRAPPER_SCHEME.to_string(), {
            // Moved in, so the handler can only ever reach the file this
            // player was opened for (see `wrapper_response`).
            let local = video.local.clone();
            move |_webview_id, request| wrapper_response(request, local.as_ref())
        })
        // The YouTube logo / "Watch on YouTube" (and any target=_blank link
        // inside the player) ask to open a new window. WebView2 denies that
        // by default, so those buttons appear dead — hand the URL to the
        // system browser instead, which is the behavior the user expects
        // anyway ("open on YouTube" opening their real browser).
        .with_new_window_req_handler(|url, _features| {
            let _ = open::that(url);
            wry::NewWindowResponse::Deny
        })
        // See `FULLSCREEN_WATCHER_SCRIPT` — the wrapper page's own
        // `fullscreenchange` listener, which catches exits the native
        // `ContainsFullScreenElementChanged` event (`subscribe_fullscreen`)
        // sometimes misses.
        .with_ipc_handler(|request| match request.body().as_str() {
            "fs:1" => set_fullscreen(true),
            "fs:0" => set_fullscreen(false),
            _ => {}
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
    subscribe_fullscreen(&webview);
    PLAYER.with(|player| *player.borrow_mut() = Some(NativePlayer { webview, host }));
    Ok(())
}

/// Reglues the live player to freshly probed stage geometry: host window
/// to the visible slice (hidden when `None` — scrolled away or an iced
/// overlay is open), webview offset so the full rect stays put underneath.
/// Event-loop thread only; harmless when no player is live.
pub fn sync_bounds(full: iced::Rectangle, visible: Option<iced::Rectangle>, scale: f32) {
    let glued = (to_physical(full, scale), visible.map(|v| to_physical(v, scale)));
    LAST_GLUED.with(|g| g.set(Some(glued)));
    // While a page element is fullscreen the webview fills the window — the
    // stage geometry is irrelevant until it exits (see `enter_fullscreen`),
    // and applying it would visibly snap the video back to card size.
    if FULLSCREEN.with(|f| f.get()) {
        return;
    }
    apply_glued(glued);
}

/// Positions the host + webview for the given glued geometry. The single
/// place stage-glue bounds are actually pushed to the OS.
fn apply_glued((full_phys, visible_phys): Glued) {
    PLAYER.with(|player| {
        let borrowed = player.borrow();
        let Some(native) = borrowed.as_ref() else {
            return;
        };
        host::sync(native.host, visible_phys);
        if let Some((x, y, _, _)) = visible_phys {
            let _ = native.webview.set_bounds(webview_bounds(full_phys, (x, y)));
        }
    });
}

/// Applies a fullscreen state change: fill the app window on enter, restore
/// the glued stage geometry on exit. Idempotent (ignores no-op repeats).
/// Event-loop thread only.
fn set_fullscreen(on: bool) {
    if FULLSCREEN.with(|f| f.get()) == on {
        return;
    }
    FULLSCREEN.with(|f| f.set(on));
    if on {
        fill_window();
    } else if let Some(glued) = LAST_GLUED.with(|g| g.get()) {
        // Restore the glued stage geometry directly — the iced loop's
        // `synced` guard may not re-issue a sync on its own.
        apply_glued(glued);
    }
}

/// Expands the clip host + webview to cover the whole app window, escaping
/// the stage clip — the fullscreen state. Uses the parent (winit) window's
/// client rect, already in physical pixels like everything else here.
/// `MoveWindow`/`ShowWindow` (like `host::sync`) — no z-order change is
/// needed since the clip host is the app window's only child.
fn fill_window() {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetParent, MoveWindow, ShowWindow, SW_SHOWNA,
    };

    PLAYER.with(|player| {
        let borrowed = player.borrow();
        let Some(native) = borrowed.as_ref() else {
            return;
        };
        let host = HWND(native.host as *mut core::ffi::c_void);
        let Ok(parent) = (unsafe { GetParent(host) }) else {
            return;
        };
        let mut rc = RECT::default();
        if unsafe { GetClientRect(parent, &mut rc) }.is_err() {
            return;
        }
        let (w, h) = ((rc.right - rc.left).max(1), (rc.bottom - rc.top).max(1));
        unsafe {
            let _ = MoveWindow(host, 0, 0, w, h, true);
            let _ = ShowWindow(host, SW_SHOWNA);
        }
        let _ = native.webview.set_bounds(wry::Rect {
            position: wry::dpi::PhysicalPosition::new(0, 0).into(),
            size: wry::dpi::PhysicalSize::new(w as u32, h as u32).into(),
        });
    });
}

/// Subscribes the live webview to WebView2's fullscreen event so the
/// player's fullscreen button works — wry doesn't surface it, so we reach
/// the raw controller directly (`webview2-com`). The handler reads the
/// current state off the sender and applies it. It fires on the event-loop
/// thread (WebView2 posts it to the UI message pump), where the player
/// thread-locals live. The registration token is intentionally dropped: the
/// subscription lives until the webview is destroyed.
fn subscribe_fullscreen(webview: &wry::WebView) {
    use webview2_com::ContainsFullScreenElementChangedEventHandler;
    use wry::WebViewExtWindows;

    let core = webview.webview();
    let handler = ContainsFullScreenElementChangedEventHandler::create(Box::new(|sender, _args| {
        if let Some(sender) = sender {
            let mut is_fs = windows::core::BOOL::default();
            if unsafe { sender.ContainsFullScreenElement(&mut is_fs) }.is_ok() {
                set_fullscreen(is_fs.as_bool());
            }
        }
        Ok(())
    }));
    let mut token = 0i64;
    let _ = unsafe { core.add_ContainsFullScreenElementChanged(&handler, &mut token) };
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

/// Returns Win32 keyboard focus to the app window. Clicking the playing
/// video focuses the WebView2 document (wry's container window proc
/// forwards WM_SETFOCUS to it), and winit doesn't reclaim focus on an
/// ordinary click of its own client area — so without this the composer
/// stays dead after the user has interacted with a playing video. Called
/// on any app-surface click while a video is live; presses on the video's
/// own child HWND never reach iced, so this can't fire from them.
/// Event-loop thread only. No-op unless a player is live, so a stray call
/// can't wrench focus around when nothing stole it.
pub fn reclaim_focus(window: &(impl HasWindowHandle + ?Sized)) {
    let live = PLAYER.with(|player| player.borrow().is_some());
    if !live {
        return;
    }
    let Ok(handle) = window.window_handle() else {
        return;
    };
    if let raw_window_handle::RawWindowHandle::Win32(h) = handle.as_raw() {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
        let _ = unsafe { SetFocus(Some(HWND(h.hwnd.get() as *mut core::ffi::c_void))) };
    }
}

/// Tears down the player (stops audio/video). Event-loop thread only;
/// harmless when no player is live.
pub fn close() {
    // Reset fullscreen bookkeeping so a stale flag can't make the next
    // video's first `sync_bounds` no-op itself into an unglued state.
    FULLSCREEN.with(|f| f.set(false));
    LAST_GLUED.with(|g| g.set(None));
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
    fn range_header_shapes() {
        // Inclusive ends, exactly as the header spells them.
        assert_eq!(parse_byte_range("bytes=0-99", 1000), Some((0, 99)));
        // Open end: everything left of the file.
        assert_eq!(parse_byte_range("bytes=500-", 1000), Some((500, 999)));
        // Suffix form is the LAST n bytes, not "up to byte n".
        assert_eq!(parse_byte_range("bytes=-200", 1000), Some((800, 999)));
        // A suffix longer than the file is the whole file, not an error.
        assert_eq!(parse_byte_range("bytes=-4000", 1000), Some((0, 999)));
        // An end past the last byte clamps rather than failing.
        assert_eq!(parse_byte_range("bytes=900-5000", 1000), Some((900, 999)));
        // Multi-range: the first range answers for the request.
        assert_eq!(parse_byte_range("bytes=0-9, 20-29", 1000), Some((0, 9)));
        // Unsatisfiable or malformed — the caller turns these into a 416.
        assert_eq!(parse_byte_range("bytes=1000-1100", 1000), None);
        assert_eq!(parse_byte_range("bytes=50-10", 1000), None);
        assert_eq!(parse_byte_range("bytes=-0", 1000), None);
        assert_eq!(parse_byte_range("bytes=abc-", 1000), None);
        assert_eq!(parse_byte_range("items=0-9", 1000), None);
        // An empty file can satisfy nothing.
        assert_eq!(parse_byte_range("bytes=0-", 0), None);
    }

    #[test]
    fn attachment_plays_from_the_wrapper_protocol() {
        let video =
            EmbedVideo::attachment(std::path::PathBuf::from("C:/cache/matrix.org_abc"), "video/mp4".into());
        let player_url = video.player_url();
        assert!(
            player_url.starts_with(&format!("{WRAPPER_SCHEME}://localhost/attachment?v=")),
            "{player_url}"
        );
        // Two different files must not share a player URL (see the
        // cache-buster in `player_url`).
        let other =
            EmbedVideo::attachment(std::path::PathBuf::from("C:/cache/matrix.org_xyz"), "video/mp4".into());
        assert_ne!(player_url, other.player_url());
        // "Open externally" hands the local file to the system player.
        assert_eq!(video.watch_url(), "C:/cache/matrix.org_abc");
    }

    #[test]
    fn video_mime_falls_back_to_mp4() {
        assert_eq!(mime_for_video("clip.webm"), "video/webm");
        assert_eq!(mime_for_video("CLIP.MOV"), "video/quicktime");
        assert_eq!(mime_for_video("no-extension"), "video/mp4");
    }

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
    fn recognizes_direct_video_file_links() {
        for url in [
            "https://cdn.example.com/clips/funny.mp4",
            "https://cdn.example.com/clips/funny.MP4",
            "https://cdn.example.com/a/b/c.webm",
            "https://cdn.example.com/old.m4v?token=abc",
            "https://cdn.example.com/demo.ogv#t=10",
        ] {
            let video = video_in(url).unwrap_or_else(|| panic!("no match: {url}"));
            assert_eq!(video.platform, Platform::File, "{url}");
            assert_eq!(video.source_url, url);
        }

        let named = video_in("https://cdn.example.com/clips/funny cat.mp4").unwrap();
        assert_eq!(named.file_name().as_deref(), Some("funny cat.mp4"));
        assert_eq!(video_in("https://cdn.example.com/clip.mp4").unwrap().watch_url(), "https://cdn.example.com/clip.mp4");
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
            // Video-adjacent but not a container WebView2 plays natively,
            // or not a video at all — must fall through to the regular
            // OpenGraph preview instead of a dead player.
            "https://example.com/video.mov",
            "https://example.com/video.mkv",
            "https://example.com/video.avi",
            "https://example.com/page.html",
            "https://example.com/video.mp4x",
            "https://example.com/watch?file=clip.mp4",
            "https://example.com/novideoext",
        ] {
            assert!(video_in(url).is_none(), "should not match: {url}");
        }
    }

    #[test]
    fn percent_round_trips_arbitrary_bytes() {
        let original = "https://cdn.example.com/a b/c.mp4?x=1&y=2%3";
        let encoded = percent_encode(original);
        assert!(encoded.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'%')));
        assert_eq!(percent_decode(&encoded).as_deref(), Some(original));
    }

    /// A temp file holding `bytes`, removed on drop — the stand-in for a
    /// media-cache entry in the streaming tests below.
    struct TempVideo(std::path::PathBuf);

    impl TempVideo {
        fn new(tag: &str, bytes: &[u8]) -> Self {
            let path = std::env::temp_dir()
                .join(format!("thornychat-stream-{tag}-{}", std::process::id()));
            std::fs::write(&path, bytes).unwrap();
            Self(path)
        }

        fn local(&self) -> LocalVideo {
            LocalVideo { path: self.0.clone(), mime: "video/mp4".into() }
        }
    }

    impl Drop for TempVideo {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn stream_request(range: Option<&str>) -> wry::http::Request<Vec<u8>> {
        let builder = wry::http::Request::builder()
            .uri(format!("{WRAPPER_SCHEME}://localhost/attachment/stream?v=0123456789abcdef"));
        let builder = match range {
            Some(range) => builder.header(wry::http::header::RANGE, range),
            None => builder,
        };
        builder.body(Vec::new()).unwrap()
    }

    /// The whole plumbing a playing attachment depends on: the wrapper page
    /// points at the stream path, and the stream path answers with the
    /// file's bytes — whole, or exactly the slice the player asked for.
    #[test]
    fn attachment_stream_serves_the_file_and_its_ranges() {
        let bytes: Vec<u8> = (0..=255u8).collect();
        let video = TempVideo::new("ranges", &bytes);
        let local = video.local();

        let page = wrapper_response(
            wry::http::Request::builder()
                .uri(format!("{WRAPPER_SCHEME}://localhost/attachment?v=0123456789abcdef"))
                .body(Vec::new())
                .unwrap(),
            Some(&local),
        );
        let html = String::from_utf8(page.body().to_vec()).unwrap();
        assert!(html.contains(r#"<video src="/attachment/stream?v=0123456789abcdef""#), "{html}");

        // No Range: the whole file, and an advertisement that ranges work.
        let whole = wrapper_response(stream_request(None), Some(&local));
        assert_eq!(whole.status(), 200);
        assert_eq!(whole.body().as_ref(), bytes.as_slice());
        assert_eq!(whole.headers()[wry::http::header::ACCEPT_RANGES], "bytes");
        assert_eq!(whole.headers()[wry::http::header::CONTENT_TYPE], "video/mp4");
        assert_eq!(whole.headers()[wry::http::header::CONTENT_LENGTH], "256");

        // A seek: exactly the requested slice, labelled with where it sits.
        let slice = wrapper_response(stream_request(Some("bytes=200-209")), Some(&local));
        assert_eq!(slice.status(), 206);
        assert_eq!(slice.body().as_ref(), &bytes[200..=209]);
        assert_eq!(slice.headers()[wry::http::header::CONTENT_RANGE], "bytes 200-209/256");

        // Past the end: 416, not a body the player would take for content.
        let bad = wrapper_response(stream_request(Some("bytes=900-")), Some(&local));
        assert_eq!(bad.status(), 416);
        assert!(bad.body().is_empty());

        // Nothing playing — the handler has no file to hand out at all.
        let orphan = wrapper_response(stream_request(None), None);
        assert_eq!(orphan.status(), 404);
    }

    /// Builds a fake custom-protocol request for `file_wrapper_page`, as if
    /// `EmbedVideo::player_url` had encoded `src` for it.
    fn file_wrapper_request(src: &str) -> wry::http::Request<Vec<u8>> {
        wry::http::Request::builder()
            .uri(format!("thornyplayer://localhost/video?src={}", percent_encode(src)))
            .body(Vec::new())
            .unwrap()
    }

    // The next two tests cover the exact XSS shape this module has to guard
    // against: a chat message can make `source_url` anything, and it ends up
    // interpolated into an HTML document that then executes in a WebView2
    // child window.

    #[test]
    fn file_wrapper_page_rejects_non_http_scheme() {
        let response = file_wrapper_page(file_wrapper_request("javascript:alert(1)"));
        assert_eq!(response.status().as_u16(), 404);
    }

    #[test]
    fn file_wrapper_page_escapes_html_in_src() {
        let malicious = r#"https://example.com/"><script>alert(1)</script>.mp4"#;
        let response = file_wrapper_page(file_wrapper_request(malicious));
        let body = String::from_utf8(response.body().to_vec()).unwrap();
        assert!(!body.contains("<script>alert(1)</script>"));
        assert!(body.contains("&lt;script&gt;"));
    }
}
