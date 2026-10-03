//! High-quality upscaling for the lightbox. Once an image is magnified past
//! its own resolution, the page asks the media protocol for a larger copy
//! (`?upscale=<long edge>`, see `media.rs`) and swaps it in under the live
//! zoom; this module makes that copy, caches it on disk and hands back the
//! path to serve.
//!
//! The kernel is **Lanczos3**: the best-quality resampler that adds no detail
//! of its own (what ffmpeg and ImageMagick reach for), noticeably sharper than
//! the browser's bilinear/bicubic magnification. Generative super-resolution
//! (Real-ESRGAN) was tried on the iced build and rejected: it invents
//! plausible-but-wrong detail, which looks uncanny on faces, text and
//! illustrations. `FilterType::CatmullRom` is the softer, ring-free
//! alternative if Lanczos halos on hard edges ever bother.
//!
//! The work runs on a blocking thread, one image at a time, never on the
//! async threads that serve the rest of the media protocol.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Instant;

use image::codecs::gif::GifDecoder;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType as PngFilter, PngDecoder, PngEncoder};
use image::codecs::webp::WebPDecoder;
use image::imageops::FilterType;
use image::{AnimationDecoder, DynamicImage, ImageEncoder, ImageFormat, ImageReader, Limits};

/// Longest output edge produced: bounds the RGBA buffer (a 4096-long image is
/// ~48 MB while it's being made). Past this, the browser's own scaling of the
/// source is already fine.
pub const MAX_OUTPUT_EDGE: u32 = 4096;

/// An upscale smaller than this is indistinguishable from the browser's own.
const MIN_GAIN: f64 = 1.25;

/// Largest source decoded (pixels and per-axis). A decompression bomb gets a
/// refusal, not a multi-gigabyte allocation.
const MAX_SOURCE_PIXELS: u64 = 40_000_000;
const MAX_SOURCE_EDGE: u32 = 16_384;

/// JPEG quality for opaque results. At the magnifications this copy is shown
/// at, q92 artifacts are well below a screen pixel, and the file is a quarter
/// of a fast-compressed PNG. Measured (release, 1024x576 → 4096x2304, see
/// `encoding_cost` below): resize 190 ms, JPEG 150 ms / 2.8 MB, PNG 46 ms /
/// 11.2 MB — the extra 100 ms buys four times the cache and a smaller copy
/// into the webview.
const JPEG_QUALITY: u8 = 92;

/// Disk cap for upscaled copies; the oldest go first.
pub const CACHE_CAP_BYTES: u64 = 256 * 1024 * 1024;

/// Why there is no upscaled copy. The page keeps showing the original.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Already at (or near) the output cap: nothing to gain.
    NotWorthIt,
    /// An animated GIF/WebP/APNG: a still copy would freeze it.
    Animated,
    /// Not a raster format this build decodes (SVG, AVIF…), or broken.
    Undecodable,
    TooLarge,
    /// The source couldn't be fetched, or the copy couldn't be written.
    Unavailable,
}

/// The size to upscale a `width`×`height` image to so its long edge is
/// `requested` (capped at [`MAX_OUTPUT_EDGE`]), aspect preserved. `None` when
/// that wouldn't enlarge it by at least [`MIN_GAIN`] — this never downscales.
pub fn output_size(width: u32, height: u32, requested: u32) -> Option<(u32, u32)> {
    let long = width.max(height);
    if long == 0 {
        return None;
    }
    let edge = requested.min(MAX_OUTPUT_EDGE);
    let factor = f64::from(edge) / f64::from(long);
    if factor < MIN_GAIN {
        return None;
    }
    let scale = |side: u32| ((f64::from(side) * factor).round() as u32).max(1);
    Some((scale(width), scale(height)))
}

/// The cached copy of `source_id` upscaled to `requested`, making it first if
/// needed. `load` fetches the source bytes and is only called on a miss.
pub async fn upscaled<F, Fut>(cache_dir: &Path, source_id: &str, requested: u32, load: F) -> Result<PathBuf, Refusal>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<Vec<u8>>>,
{
    let path = cache_dir.join(format!("{:016x}-{requested}", crate::embeds::fnv1a(source_id)));
    if is_file(&path).await {
        return Ok(path);
    }
    // One at a time: each is up to a few hundred MB of transient buffers and
    // a few hundred ms of CPU, and a second request for the same image (the
    // page's preload racing the swap) should find the first one's result.
    static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _turn = ONE_AT_A_TIME.lock().await;
    if is_file(&path).await {
        return Ok(path);
    }

    let source = load().await.map_err(|error| {
        tracing::debug!(%error, "upscale source unavailable");
        Refusal::Unavailable
    })?;
    let dir = cache_dir.to_path_buf();
    let target = path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let started = Instant::now();
        let (bytes, mime) = upscale(&source, requested)?;
        write_atomically(&dir, &target, &bytes).map_err(|error| {
            tracing::warn!(%error, path = %target.display(), "could not cache an upscaled image");
            Refusal::Unavailable
        })?;
        tracing::debug!(elapsed_ms = started.elapsed().as_millis() as u64, bytes = bytes.len(), mime, "upscaled an image");
        client_core::media::evict_cache_dir(&dir, CACHE_CAP_BYTES);
        Ok(target)
    })
    .await
    .map_err(|_| Refusal::Unavailable)?
}

async fn is_file(path: &Path) -> bool {
    tokio::fs::metadata(path).await.is_ok_and(|meta| meta.is_file())
}

fn write_atomically(dir: &Path, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// Decodes `source`, Lanczos3-resizes it so its long edge is `requested`
/// (see [`output_size`]) and encodes the result: JPEG when opaque, PNG when
/// it has transparency. Returns the bytes and their content type.
pub fn upscale(source: &[u8], requested: u32) -> Result<(Vec<u8>, &'static str), Refusal> {
    let reader = ImageReader::new(Cursor::new(source)).with_guessed_format().map_err(|_| Refusal::Undecodable)?;
    let format = reader.format().ok_or(Refusal::Undecodable)?;
    if is_animated(format, source) {
        return Err(Refusal::Animated);
    }
    let (width, height) = reader.into_dimensions().map_err(|_| Refusal::Undecodable)?;
    if width > MAX_SOURCE_EDGE || height > MAX_SOURCE_EDGE || u64::from(width) * u64::from(height) > MAX_SOURCE_PIXELS {
        return Err(Refusal::TooLarge);
    }
    let (out_width, out_height) = output_size(width, height, requested).ok_or(Refusal::NotWorthIt)?;

    let mut reader = ImageReader::with_format(Cursor::new(source), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_EDGE);
    limits.max_image_height = Some(MAX_SOURCE_EDGE);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|_| Refusal::Undecodable)?;
    encode(decoded, out_width, out_height)
}

fn encode(decoded: DynamicImage, width: u32, height: u32) -> Result<(Vec<u8>, &'static str), Refusal> {
    let mut out = Vec::new();
    if decoded.color().has_alpha() {
        let resized = image::imageops::resize(&decoded.to_rgba8(), width, height, FilterType::Lanczos3);
        PngEncoder::new_with_quality(&mut out, CompressionType::Fast, PngFilter::Adaptive)
            .write_image(resized.as_raw(), width, height, image::ExtendedColorType::Rgba8)
            .map_err(|_| Refusal::Unavailable)?;
        Ok((out, "image/png"))
    } else {
        let resized = image::imageops::resize(&decoded.to_rgb8(), width, height, FilterType::Lanczos3);
        JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
            .write_image(resized.as_raw(), width, height, image::ExtendedColorType::Rgb8)
            .map_err(|_| Refusal::Unavailable)?;
        Ok((out, "image/jpeg"))
    }
}

/// Whether `source` holds more than one frame. Anything that can't be probed
/// counts as still; the decode that follows will refuse it if it's broken.
fn is_animated(format: ImageFormat, source: &[u8]) -> bool {
    match format {
        ImageFormat::Gif => GifDecoder::new(Cursor::new(source)).is_ok_and(|decoder| decoder.into_frames().take(2).count() > 1),
        ImageFormat::WebP => WebPDecoder::new(Cursor::new(source)).is_ok_and(|decoder| decoder.has_animation()),
        ImageFormat::Png => PngDecoder::new(Cursor::new(source)).is_ok_and(|decoder| decoder.is_apng().unwrap_or(false)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage, Rgba, RgbaImage};

    fn png(image: &DynamicImage) -> Vec<u8> {
        let mut out = Vec::new();
        image.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).unwrap();
        out
    }

    fn decode(bytes: &[u8]) -> DynamicImage {
        image::load_from_memory(bytes).unwrap()
    }

    #[test]
    fn small_sources_reach_the_requested_edge() {
        assert_eq!(output_size(100, 50, 400), Some((400, 200)));
        assert_eq!(output_size(50, 100, 400), Some((200, 400)));
    }

    #[test]
    fn the_output_cap_binds_before_the_request() {
        // 4096 / 2000 = 2.048x, aspect preserved.
        assert_eq!(output_size(2000, 1000, 8000), Some((4096, 2048)));
    }

    #[test]
    fn it_never_downscales_or_bothers_with_a_tiny_gain() {
        assert_eq!(output_size(8000, 4000, 4096), None);
        assert_eq!(output_size(3600, 2000, 4096), None); // 1.14x
        assert_eq!(output_size(3200, 2000, 4096), Some((4096, 2560))); // 1.28x
        assert_eq!(output_size(0, 0, 400), None);
    }

    #[test]
    fn a_thin_image_keeps_at_least_one_pixel_across() {
        assert_eq!(output_size(1000, 1, 4000), Some((4000, 4)));
        assert_eq!(output_size(3000, 1, 4000), Some((4000, 1)));
    }

    #[test]
    fn an_opaque_image_comes_back_as_a_larger_jpeg() {
        let source = DynamicImage::ImageRgb8(RgbImage::from_fn(64, 32, |x, y| Rgb([(x * 4) as u8, (y * 8) as u8, 128])));
        let (bytes, mime) = upscale(&png(&source), 256).unwrap();
        assert_eq!(mime, "image/jpeg");
        let out = decode(&bytes);
        assert_eq!((out.width(), out.height()), (256, 128));
    }

    #[test]
    fn transparency_survives_as_png() {
        let source = DynamicImage::ImageRgba8(RgbaImage::from_fn(40, 40, |x, _| Rgba([255, 0, 0, if x < 20 { 0 } else { 255 }])));
        let (bytes, mime) = upscale(&png(&source), 160).unwrap();
        assert_eq!(mime, "image/png");
        let out = decode(&bytes).to_rgba8();
        assert_eq!(out.dimensions(), (160, 160));
        assert_eq!(out.get_pixel(5, 80)[3], 0);
        assert_eq!(out.get_pixel(150, 80)[3], 255);
    }

    #[test]
    fn lanczos_keeps_an_edge_sharper_than_bilinear() {
        // A hard black/white edge, magnified 8x. The steepest step between
        // neighbouring output pixels is how crisp the edge still looks;
        // bilinear (what the browser does) spreads it into an even ramp.
        let source = DynamicImage::ImageRgb8(RgbImage::from_fn(16, 4, |x, _| if x < 8 { Rgb([0; 3]) } else { Rgb([255; 3]) }));
        let steepest = |filter| {
            let out = image::imageops::resize(&source.to_rgb8(), 128, 32, filter);
            (0..127).map(|x| out.get_pixel(x + 1, 16)[0].abs_diff(out.get_pixel(x, 16)[0])).max().unwrap_or(0)
        };
        let (lanczos, bilinear) = (steepest(FilterType::Lanczos3), steepest(FilterType::Triangle));
        assert!(lanczos > bilinear, "lanczos {lanczos} vs bilinear {bilinear}");
    }

    #[test]
    fn already_large_images_are_refused_not_shrunk() {
        let source = DynamicImage::ImageRgb8(RgbImage::new(3800, 10));
        assert_eq!(upscale(&png(&source), 4096), Err(Refusal::NotWorthIt));
    }

    #[test]
    fn animations_are_left_alone() {
        use image::codecs::gif::GifEncoder;
        use image::Frame;
        let mut gif = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut gif);
            for shade in [0u8, 255] {
                encoder.encode_frame(Frame::new(RgbaImage::from_pixel(8, 8, Rgba([shade, shade, shade, 255])))).unwrap();
            }
        }
        assert_eq!(upscale(&gif, 64), Err(Refusal::Animated));
    }

    #[test]
    fn a_single_frame_gif_is_upscaled() {
        use image::codecs::gif::GifEncoder;
        use image::Frame;
        let mut gif = Vec::new();
        GifEncoder::new(&mut gif).encode_frame(Frame::new(RgbaImage::from_pixel(8, 8, Rgba([9, 9, 9, 255])))).unwrap();
        let (bytes, _) = upscale(&gif, 64).unwrap();
        assert_eq!(decode(&bytes).width(), 64);
    }

    #[test]
    fn junk_and_vector_images_are_undecodable() {
        assert_eq!(upscale(b"not an image at all", 512), Err(Refusal::Undecodable));
        assert_eq!(upscale(b"<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'/>", 512), Err(Refusal::Undecodable));
    }

    /// CRC-32 (IEEE), bitwise: enough to re-sign one patched PNG chunk.
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
        !crc
    }

    #[test]
    fn a_decompression_bomb_is_refused_before_decoding() {
        // A well-formed PNG whose header claims 20000x20000: refused on the
        // dimensions, before anything is allocated for the pixels.
        let mut bomb = Vec::new();
        PngEncoder::new(&mut bomb).write_image(&[0; 3], 1, 1, image::ExtendedColorType::Rgb8).unwrap();
        // IHDR data is bytes 16..29 (width, height, …), its CRC 29..33 over
        // the chunk type and data (12..29).
        bomb[16..20].copy_from_slice(&20_000u32.to_be_bytes());
        bomb[20..24].copy_from_slice(&20_000u32.to_be_bytes());
        let crc = crc32(&bomb[12..29]);
        bomb[29..33].copy_from_slice(&crc.to_be_bytes());
        assert_eq!(upscale(&bomb, 4096), Err(Refusal::TooLarge));
    }

    #[tokio::test]
    async fn results_are_cached_and_the_source_is_loaded_once() {
        let dir = std::env::temp_dir().join(format!("thornychat-upscale-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let source = png(&DynamicImage::ImageRgb8(RgbImage::from_pixel(32, 32, Rgb([10, 20, 30]))));
        let loads = std::sync::atomic::AtomicUsize::new(0);
        let load = || async {
            loads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(source.clone())
        };
        let first = upscaled(&dir, "mxc://example.org/abc", 128, load).await.unwrap();
        let second = upscaled(&dir, "mxc://example.org/abc", 128, load).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(loads.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(decode(&std::fs::read(&first).unwrap()).width(), 128);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The numbers behind [`JPEG_QUALITY`] and the PNG-only-for-alpha rule:
    /// `cargo test -p thornychat --release encoding_cost -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn encoding_cost() {
        // A photo-like 1024x576 source (smooth gradients plus fine detail),
        // upscaled 4x to the output cap.
        let source = RgbImage::from_fn(1024, 576, |x, y| {
            let n = (x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503)) % 32;
            Rgb([((x / 4) as u8).wrapping_add(n as u8), ((y / 3) as u8).wrapping_add(n as u8), ((x + y) / 7) as u8])
        });
        let started = Instant::now();
        let resized = image::imageops::resize(&source, 4096, 2304, FilterType::Lanczos3);
        println!("lanczos3 1024x576 -> 4096x2304: {:?}", started.elapsed());
        let started = Instant::now();
        let mut jpeg = Vec::new();
        JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY).write_image(resized.as_raw(), 4096, 2304, image::ExtendedColorType::Rgb8).unwrap();
        println!("jpeg q{JPEG_QUALITY}: {:?}, {} KB", started.elapsed(), jpeg.len() / 1024);
        let started = Instant::now();
        let mut png = Vec::new();
        PngEncoder::new_with_quality(&mut png, CompressionType::Fast, PngFilter::Adaptive)
            .write_image(resized.as_raw(), 4096, 2304, image::ExtendedColorType::Rgb8)
            .unwrap();
        println!("png fast: {:?}, {} KB", started.elapsed(), png.len() / 1024);
    }
}
