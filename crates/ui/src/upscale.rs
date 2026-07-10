//! Super-resolution upscaling for the lightbox. When a zoomed-in image is
//! magnified well past its resolution (the widget fires once past ~300%), the
//! pipeline here produces a larger, sharper version off-thread and the lightbox
//! swaps to it (see `update`'s `Message::UpscaleZoomedImage` /
//! `Message::ImageUpscaled`) without disturbing the live zoom/pan — the
//! swapped handle just carries more pixels.
//!
//! The kernel is a compact Real-ESRGAN model (`realesr-general-x4v3`,
//! SRVGGNetCompact, 4x) run through ONNX Runtime via `ort`, preferring the
//! DirectML (GPU) execution provider and falling back to CPU. If the runtime
//! can't initialize at all — or inference fails — it degrades to a Lanczos3
//! resample so the feature still does *something* (and the log line tells us
//! which path ran).
//!
//! v1 runs a single inference on a size-capped source (no tiling yet): the
//! images that actually look bad magnified are the small ones, and those fit
//! in one pass. Larger sources are already sharp enough that GPU bilinear is
//! fine, so they're skipped. Tiling for big sources is a later optimization.

use std::sync::{Mutex, OnceLock};

use iced::widget::image::Handle;
use image::RgbImage;
use ort::execution_providers::{CPUExecutionProvider, DirectMLExecutionProvider};
use ort::session::Session;
use ort::value::Tensor;

/// Bundled model. Input `input` (dynamic NCHW, RGB f32 in [0,1]); output
/// `output` (same layout, 4x spatial, clipped to [0,1]).
static MODEL: &[u8] = include_bytes!("../../../assets/models/realesr-general-x4v3.onnx");

/// The network's fixed upscale factor.
const SCALE: u32 = 4;

/// Only images whose long edge is at most this get super-resolved: they're the
/// ones that look bad magnified, and it keeps the single-shot inference and its
/// buffers bounded (a 1024px source → 4096px result). Larger sources are
/// skipped — bilinear on them is already fine, and one-shot SR would be huge.
const MAX_SR_SOURCE_EDGE: u32 = 1024;

/// Longest edge the Lanczos fallback will produce, so its buffer stays bounded
/// even though it isn't gated by [`MAX_SR_SOURCE_EDGE`] the same way.
const MAX_FALLBACK_EDGE: u32 = 4096;

/// Lazily-built, process-wide inference session behind a `Mutex` — `ort`'s
/// `run` takes `&mut self`, and the lightbox only ever upscales one image at a
/// time, so serializing is free in practice. A failed init is cached too — no
/// point retrying a missing runtime on every zoom.
fn session() -> Result<&'static Mutex<Session>, String> {
    static SESSION: OnceLock<Result<Mutex<Session>, String>> = OnceLock::new();
    SESSION
        .get_or_init(|| {
            let builder = Session::builder().map_err(|error| error.to_string())?;
            let session = builder
                .with_execution_providers([
                    DirectMLExecutionProvider::default().build(),
                    CPUExecutionProvider::default().build(),
                ])
                .map_err(|error| error.to_string())?
                .commit_from_memory(MODEL)
                .map_err(|error| error.to_string())?;
            tracing::info!("ESRGAN upscaler session initialized");
            Ok(Mutex::new(session))
        })
        .as_ref()
        .map_err(|error| error.clone())
}

/// Decodes `bytes` and returns a super-resolved, ready-to-draw RGBA handle.
/// Meant to run on a blocking thread. `Err` when the source is already large
/// enough that there's nothing worth doing (the caller just keeps the
/// original); ONNX failures fall back to a Lanczos resample rather than
/// erroring, so the picture still sharpens.
pub fn upscale_to_handle(bytes: &[u8]) -> Result<Handle, String> {
    let decoded = image::load_from_memory(bytes).map_err(|error| error.to_string())?;
    let source = decoded.to_rgb8();
    let (width, height) = source.dimensions();
    if width == 0 || height == 0 {
        return Err("image has a zero dimension".into());
    }
    if width.max(height) > MAX_SR_SOURCE_EDGE {
        return Err("source already high-resolution; skipping upscale".into());
    }

    match esrgan(&source, width, height) {
        Ok(handle) => Ok(handle),
        Err(reason) => {
            tracing::warn!(reason, "ESRGAN upscale failed; falling back to Lanczos");
            Ok(lanczos(&source, width, height))
        }
    }
}

/// Runs the source through the ESRGAN model in one pass.
fn esrgan(source: &RgbImage, width: u32, height: u32) -> Result<Handle, String> {
    let session = session()?;

    // HWC u8 [0,255] → planar CHW f32 [0,1], the layout the network expects.
    let (w, h) = (width as usize, height as usize);
    let plane = w * h;
    let mut input = vec![0f32; 3 * plane];
    let (r, gb) = input.split_at_mut(plane);
    let (g, b) = gb.split_at_mut(plane);
    for (i, pixel) in source.pixels().enumerate() {
        r[i] = pixel[0] as f32 / 255.0;
        g[i] = pixel[1] as f32 / 255.0;
        b[i] = pixel[2] as f32 / 255.0;
    }

    let tensor = Tensor::from_array(([1usize, 3, h, w], input)).map_err(|e| e.to_string())?;
    let mut session = session.lock().map_err(|e| e.to_string())?;
    let outputs =
        session.run(ort::inputs!["input" => tensor]).map_err(|e| e.to_string())?;
    let (shape, data) =
        outputs["output"].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;

    // Expected shape [1, 3, height*4, width*4].
    let out_h = shape[2] as usize;
    let out_w = shape[3] as usize;
    let out_plane = out_w * out_h;
    if data.len() < 3 * out_plane {
        return Err("model output smaller than its declared shape".into());
    }

    // Planar CHW f32 → interleaved RGBA u8.
    let mut rgba = vec![0u8; out_plane * 4];
    for i in 0..out_plane {
        rgba[i * 4] = to_u8(data[i]);
        rgba[i * 4 + 1] = to_u8(data[out_plane + i]);
        rgba[i * 4 + 2] = to_u8(data[2 * out_plane + i]);
        rgba[i * 4 + 3] = 255;
    }
    Ok(Handle::from_rgba(out_w as u32, out_h as u32, rgba))
}

/// Classical fallback: Lanczos3 resample toward [`SCALE`]x, long edge capped at
/// [`MAX_FALLBACK_EDGE`]. Sharper than GPU bilinear, and always available.
fn lanczos(source: &RgbImage, width: u32, height: u32) -> Handle {
    let long_edge = width.max(height) as f32;
    let factor = (MAX_FALLBACK_EDGE as f32 / long_edge).min(SCALE as f32).max(1.0);
    let target_w = ((width as f32 * factor).round() as u32).max(1);
    let target_h = ((height as f32 * factor).round() as u32).max(1);
    let resized =
        image::imageops::resize(source, target_w, target_h, image::imageops::FilterType::Lanczos3);
    let rgba = image::DynamicImage::ImageRgb8(resized).to_rgba8();
    Handle::from_rgba(target_w, target_h, rgba.into_raw())
}

fn to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}
