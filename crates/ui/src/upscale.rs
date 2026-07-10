//! High-quality upscaling for the lightbox. When a zoomed-in image is
//! magnified past its resolution (the widget fires once past ~300%), the
//! pipeline here produces a larger, sharper version off-thread and the lightbox
//! swaps to it (see `update`'s `Message::UpscaleZoomedImage` /
//! `Message::ImageUpscaled`) without disturbing the live zoom/pan — the
//! swapped handle just carries more pixels.
//!
//! The kernel is **Lanczos3**, a classical resampling filter: the best-quality
//! *non-detail-adding* upscaler (what ffmpeg/ImageMagick reach for). It's
//! noticeably sharper than the GPU's bilinear sampling but never invents detail
//! that isn't in the source. A generative super-resolution model (Real-ESRGAN)
//! was tried and rejected — it hallucinates plausible-but-wrong detail, which
//! looks bad/uncanny on faces, text, and illustrations. If Lanczos ringing
//! (faint halos on hard edges) ever bothers, `FilterType::CatmullRom` is the
//! softer, ring-free bicubic alternative — a one-line swap.

use iced::widget::image::Handle;

/// Longest output edge we'll produce, to bound the decoded RGBA buffer (a
/// 4096-long image is ~48MB at 4 bytes/px, held only while one image is open —
/// cleared on close). Past this, GPU bilinear on the source is already fine.
const MAX_OUTPUT_EDGE: u32 = 4096;

/// How far past native we resample toward. Beyond ~4x there's no more real
/// detail to sharpen, so this is plenty and keeps the buffer bounded.
const TARGET_FACTOR: f32 = 4.0;

/// Decodes `bytes`, resamples toward [`TARGET_FACTOR`]x native with Lanczos3
/// (aspect preserved, long edge capped at [`MAX_OUTPUT_EDGE`], never
/// downscales), and returns a ready-to-draw RGBA handle. Meant to run on a
/// blocking thread. `Err` when the image can't be decoded or is already large
/// enough that there's nothing worth upscaling.
pub fn upscale_to_handle(bytes: &[u8]) -> Result<Handle, String> {
    let decoded = image::load_from_memory(bytes).map_err(|error| error.to_string())?;
    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();
    if width == 0 || height == 0 {
        return Err("image has a zero dimension".into());
    }

    let (target_w, target_h) = target_size(width, height);
    if target_w <= width && target_h <= height {
        return Err("already at or above the upscale cap; nothing to do".into());
    }

    let upscaled =
        image::imageops::resize(&rgba, target_w, target_h, image::imageops::FilterType::Lanczos3);
    Ok(Handle::from_rgba(target_w, target_h, upscaled.into_raw()))
}

/// [`TARGET_FACTOR`]x native, aspect-preserved, with the long edge clamped to
/// [`MAX_OUTPUT_EDGE`] so the buffer stays bounded.
fn target_size(width: u32, height: u32) -> (u32, u32) {
    let long_edge = width.max(height) as f32;
    let factor = (MAX_OUTPUT_EDGE as f32 / long_edge).clamp(1.0, TARGET_FACTOR);
    (
        ((width as f32 * factor).round() as u32).max(1),
        ((height as f32 * factor).round() as u32).max(1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_sources_get_the_full_factor() {
        assert_eq!(target_size(100, 50), (400, 200));
    }

    #[test]
    fn the_long_edge_caps_the_factor() {
        // 4096 / 2000 = 2.048x — the cap binds before TARGET_FACTOR does,
        // and the aspect ratio is preserved.
        assert_eq!(target_size(2000, 1000), (4096, 2048));
    }

    #[test]
    fn already_huge_sources_never_upscale_or_shrink() {
        // Factor clamps to 1.0 — the caller's `target <= source` check then
        // skips the pass entirely; this must never DOWNscale.
        assert_eq!(target_size(8000, 4000), (8000, 4000));
    }
}
