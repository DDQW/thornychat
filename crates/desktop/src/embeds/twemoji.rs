//! Twemoji SVGs for unicode emoji, so the app looks the same on every
//! machine instead of depending on whatever emoji font Windows ships. Uses
//! the actively-maintained `jdecked/twemoji` fork (the original
//! `twitter/twemoji` repo was archived after Twitter/X); assets are
//! content-addressed by Unicode codepoint, so any recent release works and
//! results cache to disk indefinitely.
//!
//! The frontend computes the filename (`lib/emoji.ts`, `twemojiCodepoints`)
//! and asks for it through the media protocol; this is the fetch-and-cache
//! behind that route.

use std::path::Path;

/// A Twemoji filename stem is lowercase hex codepoints joined by `-`. Anything
/// else is refused before it can reach a path or a URL — the stem arrives from
/// the webview, which is rendering other people's messages.
pub fn is_valid_codepoints(stem: &str) -> bool {
    !stem.is_empty()
        && stem.len() <= 64
        && stem.split('-').all(|part| {
            (1..=6).contains(&part.len()) && part.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}

fn cdn_url(codepoints: &str) -> String {
    format!("https://cdn.jsdelivr.net/gh/jdecked/twemoji@latest/assets/svg/{codepoints}.svg")
}

/// Fetches the SVG bytes for `codepoints` (e.g. `1f600`, `1f3c3-200d-2642-fe0f`),
/// checking the on-disk cache first.
pub async fn fetch(cache_dir: &Path, codepoints: &str) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(is_valid_codepoints(codepoints), "not a twemoji codepoint sequence");
    let cache_path = cache_dir.join(format!("{codepoints}.svg"));
    // The same emoji is often asked for twice at once (in a message and in a
    // reaction under it); the shared cache downloads it once and never lets a
    // half-written file be read or kept.
    client_core::media::cached(&cache_path, || async {
        let response = super::http_client().get(cdn_url(codepoints)).send().await?.error_for_status()?;
        Ok(response.bytes().await?.to_vec())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_sequences_are_accepted() {
        for ok in ["1f600", "1f3c3-200d-2642-fe0f", "0023-20e3", "1f1e9-1f1ea"] {
            assert!(is_valid_codepoints(ok), "{ok}");
        }
    }

    #[test]
    fn anything_that_could_escape_the_cache_dir_is_refused() {
        for bad in ["", "..", "../x", "1f600/../x", "1F600", "1f600.svg", "1f600-", "-1f600", "zz", "1f6000000"] {
            assert!(!is_valid_codepoints(bad), "{bad:?}");
        }
    }
}
