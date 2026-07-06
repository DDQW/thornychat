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
    /// pre-decoded into `iced_gif` frame sets and rendered with its
    /// animating widget. `Arc` because the decode happens off-thread and
    /// rides back on a `Message`, which must be `Clone`.
    pub mxc_gifs: HashMap<String, std::sync::Arc<iced_gif::Frames>>,
    pub pending: HashMap<RequestId, String>,
    /// URL-keyed mirror of `pending` (plus GIF decodes still in flight) so
    /// `is_known` is a set lookup instead of a linear scan of every in-flight
    /// fetch — that scan went quadratic when a large emoji pack loaded.
    pub pending_urls: HashSet<String>,
    /// Negative cache: mxc URLs whose fetch failed. Without it, one
    /// deleted/unreachable avatar or image would be re-requested on every
    /// single sync tick of an open room, forever.
    pub failed_mxc: HashSet<String>,

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
    url: &str,
    width: u16,
    height: Option<u16>,
) -> Option<Element<'a, M>> {
    if let Some(frames) = media.mxc_gifs.get(url) {
        let mut widget = iced_gif::gif(frames).width(iced::Length::Fixed(width as f32));
        if let Some(height) = height {
            widget = widget.height(iced::Length::Fixed(height as f32));
        }
        return Some(widget.into());
    }
    if let Some(handle) = media.images.get(url) {
        let mut widget = image(handle.clone()).width(width);
        if let Some(height) = height {
            widget = widget.height(height);
        }
        return Some(widget.into());
    }
    if let Some(handle) = media.mxc_svgs.get(url) {
        let mut widget = svg(handle.clone()).width(width);
        if let Some(height) = height {
            widget = widget.height(height);
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
    avatar_url: Option<&str>,
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
