//! Decoded-image cache. Two independent stores: Matrix media (`mxc://`
//! URLs, raster, via `ClientCommand::FetchMedia`) and Twemoji SVGs (unicode
//! emoji, vector, fetched directly by the UI layer — see `twemoji.rs`).
//! `view()` functions can't have side effects, so fetches are kicked off
//! from `update()` whenever new content referencing an unfetched item
//! arrives.

use std::collections::{HashMap, HashSet};

use client_core::commands::RequestId;
use iced::widget::{container, image, svg, text};
use iced::{Element, Length};

#[derive(Default)]
pub struct State {
    pub images: HashMap<String, image::Handle>,
    /// Some `mxc://` media (custom emoji in particular — plenty of packs
    /// use vector art) is SVG rather than raster. `iced::widget::image`
    /// can't decode SVG at all (its decoder just silently produces
    /// nothing), so fetched bytes are sniffed and routed here instead of
    /// `images` when they are.
    pub mxc_svgs: HashMap<String, svg::Handle>,
    /// Animated GIFs (animated custom emotes, GIF image messages) —
    /// iced's core image widget draws only the first frame, so these are
    /// pre-decoded into [`crate::animated_image`] frame sets and rendered with
    /// its animating widget. `Arc` because the decode happens off-thread and
    /// rides back on a `Message`, which must be `Clone`.
    pub mxc_gifs: HashMap<String, std::sync::Arc<crate::animated_image::Frames>>,
    pub pending: HashMap<RequestId, String>,
    /// URL-keyed mirror of `pending` (plus GIF decodes still in flight) so
    /// `is_known` is a set lookup instead of a linear scan of every in-flight
    /// fetch — that scan went quadratic when a large emoji pack loaded.
    pub pending_urls: HashSet<String>,
    /// Negative cache: mxc URLs whose fetch failed. Without it, one
    /// deleted/unreachable avatar or image would be re-requested on every
    /// single sync tick of an open room, forever.
    pub failed_mxc: HashSet<String>,
    /// In-flight `FetchMedia` requests raised by the lightbox's Download
    /// button (as opposed to display fetches tracked in `pending`). Downloading
    /// re-fetches the original bytes rather than digging them back out of a
    /// decoded handle, so it works the same for raster, GIF, and SVG. When the
    /// matching `MediaFetched` lands, the bytes go to a save dialog instead of
    /// the display caches.
    pub download_requests: HashSet<RequestId>,

    pub emoji: HashMap<String, svg::Handle>,
    pub emoji_pending: HashSet<String>,
    /// Negative cache for Twemoji fetches — reaction keys that aren't real
    /// Twemoji assets 404 on the CDN and would otherwise refetch per tick.
    pub emoji_failed: HashSet<String>,

    /// Plain-HTTPS images (tweet avatars/photos from `pbs.twimg.com` etc.)
    /// — separate from the `mxc://` maps since they bypass Matrix media
    /// entirely.
    pub web_images: HashMap<String, image::Handle>,
    pub web_pending: HashSet<String>,

    /// Media decoded during a burst but not yet promoted into the visible
    /// caches above. `view()` reads only the visible maps, so staging keeps the
    /// rendered element tree byte-identical until one `FlushStagedMedia`
    /// promotes the whole batch at once. That collapses a storm of per-fetch
    /// reflows — each of which retriggered the timeline's scroll-anchor
    /// correction cascade and blew past iced's 3-consecutive-redraw layout
    /// settle budget — into a single reflow per coalescing window. Staged URLs
    /// stay in `pending_urls`, so `is_known` keeps suppressing re-fetches while
    /// they wait.
    pub staged: Vec<StagedMedia>,
    /// Armed between scheduling a flush and it firing, so a burst starts the
    /// coalescing timer exactly once.
    pub flush_scheduled: bool,
}

/// One decoded item parked in [`State::staged`] until the next flush promotes
/// it into the matching visible cache.
pub enum StagedMedia {
    Raster(String, image::Handle),
    Svg(String, svg::Handle),
    Gif(String, std::sync::Arc<crate::animated_image::Frames>),
}

impl State {
    pub fn is_known(&self, mxc_url: &str) -> bool {
        self.images.contains_key(mxc_url)
            || self.mxc_svgs.contains_key(mxc_url)
            || self.mxc_gifs.contains_key(mxc_url)
            || self.pending_urls.contains(mxc_url)
            || self.failed_mxc.contains(mxc_url)
    }

    pub fn is_emoji_known(&self, emoji: &str) -> bool {
        self.emoji.contains_key(emoji)
            || self.emoji_pending.contains(emoji)
            || self.emoji_failed.contains(emoji)
    }

    pub fn is_web_image_known(&self, url: &str) -> bool {
        self.web_images.contains_key(url) || self.web_pending.contains(url)
    }

    /// Promote every staged item into its visible cache and clear the staging
    /// area (dropping each URL from `pending_urls`, since it now lives in a
    /// real cache). Returns whether anything moved, so the flush handler can
    /// skip a redundant redraw when the batch was already drained.
    pub fn flush_staged(&mut self) -> bool {
        if self.staged.is_empty() {
            return false;
        }
        for item in std::mem::take(&mut self.staged) {
            match item {
                StagedMedia::Raster(url, handle) => {
                    self.pending_urls.remove(&url);
                    self.images.insert(url, handle);
                }
                StagedMedia::Svg(url, handle) => {
                    self.pending_urls.remove(&url);
                    self.mxc_svgs.insert(url, handle);
                }
                StagedMedia::Gif(url, frames) => {
                    self.pending_urls.remove(&url);
                    self.mxc_gifs.insert(url, frames);
                }
            }
        }
        true
    }
}

/// Renders an `mxc://`-keyed piece of fetched Matrix media. Checks the
/// animated-GIF cache first (so emotes actually play), then raster, then
/// SVG — plenty of custom-emoji packs ship vector art, which
/// `iced::widget::image` silently fails to decode, so fetched bytes are
/// sniffed and routed into the right map (see [`looks_like_svg`] /
/// [`looks_like_gif`]). Returns `None` while the fetch hasn't landed,
/// letting each call site pick its own placeholder.
pub fn mxc_visual<'a, M: 'a>(
    media: &'a State,
    url: &'a str,
    width: u16,
    height: Option<u16>,
) -> Option<Element<'a, M>> {
    if let Some(frames) = media.mxc_gifs.get(url) {
        let mut widget = crate::animated_image::gif(frames)
            .debug_label(url)
            .width(iced::Length::Fixed(width as f32));
        if let Some(height) = height {
            widget = widget.height(iced::Length::Fixed(height as f32));
        }
        return Some(widget.into());
    }
    if let Some(handle) = media.images.get(url) {
        let mut widget = image(handle.clone()).width(width as f32);
        if let Some(height) = height {
            widget = widget.height(height as f32);
        }
        return Some(widget.into());
    }
    if let Some(handle) = media.mxc_svgs.get(url) {
        let mut widget = svg(handle.clone()).width(width as f32);
        if let Some(height) = height {
            widget = widget.height(height as f32);
        }
        return Some(widget.into());
    }
    None
}

/// An avatar at `size`px: the fetched image if available, otherwise a
/// colored circle showing the name's first letter (what Element does while
/// media loads or when none is set).
pub fn avatar<'a, M: 'a>(
    media: &'a State,
    avatar_url: Option<&'a str>,
    name: &str,
    size: u16,
) -> Element<'a, M> {
    if let Some(visual) = avatar_url.and_then(|url| mxc_visual(media, url, size, Some(size))) {
        return visual;
    }
    let initial = name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string());
    container(text(initial).size(size as f32 * 0.5))
        .width(Length::Fixed(size as f32))
        .height(Length::Fixed(size as f32))
        .center_x(Length::Fixed(size as f32))
        .center_y(Length::Fixed(size as f32))
        .style(crate::theme::pill_badge)
        .into()
}

/// Matrix media has no reliable content-type carried alongside fetched
/// bytes at this layer, so format is sniffed from the bytes themselves.
/// Good enough for the two shapes actually seen in the wild: raster
/// (PNG/JPEG/GIF/WEBP, all with binary magic-number headers) and SVG
/// (always starts, after whitespace/an optional UTF-8 BOM, with `<?xml`
/// or `<svg`).
pub fn looks_like_gif(bytes: &[u8]) -> bool {
    bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")
}

pub fn looks_like_svg(bytes: &[u8]) -> bool {
    let mut trimmed = bytes;
    if let Some(rest) = trimmed.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        trimmed = rest;
    }
    let trimmed = trimmed
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .map(|start| &trimmed[start..])
        .unwrap_or(trimmed);
    trimmed.starts_with(b"<?xml") || trimmed.starts_with(b"<svg")
}

/// File extension for downloaded media, sniffed from the same magic numbers
/// as the routing above. `"bin"` when nothing matches, so a save never picks a
/// misleading extension.
pub fn image_extension(bytes: &[u8]) -> &'static str {
    if looks_like_gif(bytes) {
        "gif"
    } else if looks_like_svg(bytes) {
        "svg"
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "jpg"
    } else if bytes.len() >= 12 && bytes[0..4] == *b"RIFF" && bytes[8..12] == *b"WEBP" {
        "webp"
    } else {
        "bin"
    }
}

/// ISOBMFF (`ftyp`-boxed) image containers our compiled `image` crate build
/// has no decoder for — AVIF and HEIC/HEIF, both common exports from newer
/// phones and some bridges/CDNs. `iced`'s raster cache swallows a decode
/// failure completely silently (any non-IO `image::ImageError` becomes a
/// blank 1x1 "Invalid" placeholder with no log line anywhere in that path),
/// so left unchecked these bytes would "fetch successfully" and then just
/// never visibly render — indistinguishable from a fetch that never
/// happened. Catching it here lets the caller fall back to the initials
/// avatar and log *why*, instead of a silent permanent blank.
pub fn looks_like_unsupported_container(bytes: &[u8]) -> bool {
    if bytes.len() < 12 || &bytes[4..8] != b"ftyp" {
        return false;
    }
    matches!(
        &bytes[8..12],
        b"avif" | b"avis" | b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" | b"mif1"
    )
}

/// Cheap non-cryptographic content fingerprint, logged alongside every fetch
/// so a "this emote looks wrong" report can be diagnosed from the log file
/// alone — no filesystem access to the reporting machine needed. Two
/// different URLs logging the same fingerprint means the server is actually
/// serving identical bytes for both (a real content alias, not a client
/// bug); the same URL logging two different fingerprints across sessions
/// means the server-side image changed.
pub fn fingerprint(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}
