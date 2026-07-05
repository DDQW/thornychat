//! Fetches Twemoji SVGs for unicode emoji, replacing font-glyph rendering
//! (iced's bundled font has no emoji coverage, and per-glyph fallback to
//! whatever the OS happens to have installed gives inconsistent, partial
//! results). Uses the actively-maintained `jdecked/twemoji` fork (the
//! original `twitter/twemoji` repo was archived after Twitter/X); assets
//! are content-addressed by Unicode codepoint, so any recent release
//! works and results cache to disk indefinitely.

use std::path::Path;
use std::sync::OnceLock;

/// One shared connection pool for every outbound UI-side HTTP fetch
/// (Twemoji CDN, FxTwitter, tweet images). A fetch-everything emoji-picker
/// open kicks off ~1800 requests — per-request `reqwest::get` would open a
/// fresh TLS connection for each one. The User-Agent is required:
/// api.fxtwitter.com (Cloudflare) rejects UA-less requests with 401, and
/// reqwest sends no User-Agent by default.
pub fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("default reqwest client config is valid")
    })
}

/// Converts an emoji grapheme cluster into Twemoji's filename convention:
/// lowercase hex codepoints joined by `-`. The variation selector (U+FE0F)
/// is stripped ONLY when the sequence has no zero-width joiner — that's
/// twemoji.js's exact rule; ZWJ sequences (all the gendered/profession
/// emoji) keep their FE0F in the filename (verified: `1f3c3-200d-2642-fe0f`
/// is 200 on the CDN, the stripped form 404s).
fn codepoints(emoji: &str) -> String {
    let has_zwj = emoji.chars().any(|c| c == '\u{200d}');
    emoji
        .chars()
        .filter(|&c| has_zwj || c != '\u{fe0f}')
        .map(|c| format!("{:x}", c as u32))
        .collect::<Vec<_>>()
        .join("-")
}

fn cdn_url(codepoints: &str) -> String {
    format!("https://cdn.jsdelivr.net/gh/jdecked/twemoji@latest/assets/svg/{codepoints}.svg")
}

/// Fetches the SVG bytes for `emoji`, checking the on-disk cache first.
pub async fn fetch(cache_dir: &Path, emoji: &str) -> anyhow::Result<Vec<u8>> {
    let codepoints = codepoints(emoji);
    let cache_path = cache_dir.join(format!("{codepoints}.svg"));

    if let Ok(bytes) = tokio::fs::read(&cache_path).await {
        return Ok(bytes);
    }

    let response = http_client().get(cdn_url(&codepoints)).send().await?.error_for_status()?;
    let bytes = response.bytes().await?.to_vec();

    if let Some(parent) = cache_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let _ = tokio::fs::write(&cache_path, &bytes).await;

    Ok(bytes)
}
