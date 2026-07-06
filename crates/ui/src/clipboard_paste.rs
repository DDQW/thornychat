//! What Ctrl+V finds on the Windows clipboard, for paste-to-attach: copied
//! files (Explorer's CF_HDROP) and copied/screenshotted bitmaps become room
//! attachments. Plain text is deliberately NOT handled here — the focused
//! `text_input` already pastes it natively, and anything that carries text
//! *alongside* media (an Excel range rides along as a bitmap, for instance)
//! must stay a text paste rather than surprise-post an image to the room.

use std::path::PathBuf;

/// Attachable clipboard content, in preference order.
pub enum Pasted {
    /// Paths copied to the clipboard (Explorer et al.) — paths, not bytes:
    /// reading them is async I/O the caller does outside this blocking probe.
    Files(Vec<PathBuf>),
    /// A bitmap (screenshot, browser "Copy image"), re-encoded as PNG under
    /// a timestamped filename.
    Image { filename: String, bytes: Vec<u8> },
    /// Nothing attachable — including "there's text": the focused widget's
    /// own paste handles that, and we must never double-act on one Ctrl+V.
    None,
}

/// Synchronous clipboard probe — call from `spawn_blocking`, not the update
/// thread: clipboard opens retry-wait when another process holds the
/// clipboard, and re-encoding a large screenshot to PNG is real CPU work.
pub fn read() -> Pasted {
    let mut clipboard = match arboard::Clipboard::new() {
        Ok(clipboard) => clipboard,
        Err(error) => {
            tracing::debug!(%error, "clipboard unavailable");
            return Pasted::None;
        }
    };

    // Text wins. See the module doc: if there's any text, the focused
    // text_input is already pasting it.
    if matches!(clipboard.get_text(), Ok(text) if !text.is_empty()) {
        return Pasted::None;
    }

    match clipboard.get().file_list() {
        Ok(paths) if !paths.is_empty() => return Pasted::Files(paths),
        _ => {}
    }

    let image = match clipboard.get_image() {
        Ok(image) => image,
        Err(_) => return Pasted::None,
    };
    match encode_png(image) {
        Some(bytes) => {
            let filename =
                format!("pasted-{}.png", chrono::Local::now().format("%Y%m%d-%H%M%S"));
            Pasted::Image { filename, bytes }
        }
        None => Pasted::None,
    }
}

/// Clipboard bitmaps arrive as raw RGBA. PNG keeps them lossless — they're
/// usually screenshots (text and UI, where JPEG artifacts show) — and every
/// Matrix client renders it.
fn encode_png(image: arboard::ImageData<'_>) -> Option<Vec<u8>> {
    let rgba = image::RgbaImage::from_raw(
        image.width as u32,
        image.height as u32,
        image.bytes.into_owned(),
    )?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(png)
}
