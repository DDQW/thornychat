//! Everything the page needs from third-party servers. The webview's CSP
//! allows no remote connections or remote images, so link cards, Twemoji and
//! the images inside them all come through here: fetched by the backend,
//! restricted to known hosts, and handed to the page over the media protocol
//! (see `media.rs`) or as plain data.

use std::sync::OnceLock;

pub mod steam;
pub mod tweets;
pub mod twemoji;

/// One shared connection pool for every outbound third-party fetch. The
/// User-Agent is required: api.fxtwitter.com (Cloudflare) rejects UA-less
/// requests with 401, and reqwest sends none by default.
pub fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .expect("default reqwest client config is valid")
    })
}

/// Hosts the card renderers load images from: Twitter's media CDN for tweet
/// avatars and photos, Steam's CDNs for store capsule art. An exact-match
/// allow-list (plus the subdomain forms those CDNs really use), not a general
/// image proxy — the URL comes from the page, which renders other people's
/// messages, so it must never be able to make this process fetch an arbitrary
/// address (a LAN service, a metadata endpoint).
const WEB_IMAGE_HOSTS: &[&str] = &[
    "pbs.twimg.com",
    "abs.twimg.com",
    "video.twimg.com",
    "cdn.akamai.steamstatic.com",
    "cdn.cloudflare.steamstatic.com",
    "shared.akamai.steamstatic.com",
    "shared.cloudflare.steamstatic.com",
    "shared.fastly.steamstatic.com",
    "steamcdn-a.akamaihd.net",
    "media.steampowered.com",
];

/// Whether `url` is an HTTPS image URL on an allowed host.
pub fn is_allowed_web_image(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else { return false };
    parsed.scheme() == "https"
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.port().is_none()
        && parsed.host_str().is_some_and(|host| WEB_IMAGE_HOSTS.contains(&host.to_ascii_lowercase().as_str()))
}

/// Largest image the proxy will buffer. Card art is tens of KB; a cap keeps a
/// hostile or misconfigured server from filling memory.
const MAX_WEB_IMAGE_BYTES: usize = 16 * 1024 * 1024;

/// Fetches an allowed web image, caching it under `cache_dir` keyed by a hash
/// of the URL. Returns the bytes; the caller sniffs the content type.
pub async fn fetch_web_image(cache_dir: &std::path::Path, url: &str) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(is_allowed_web_image(url), "image host is not on the allow-list");

    let cache_path = cache_dir.join(format!("web-{:016x}", fnv1a(url)));
    if let Ok(bytes) = tokio::fs::read(&cache_path).await {
        return Ok(bytes);
    }

    let response = http_client().get(url).send().await?.error_for_status()?;
    if response.content_length().is_some_and(|len| len as usize > MAX_WEB_IMAGE_BYTES) {
        anyhow::bail!("image is too large");
    }
    let bytes = response.bytes().await?;
    anyhow::ensure!(bytes.len() <= MAX_WEB_IMAGE_BYTES, "image is too large");

    if let Some(parent) = cache_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let _ = tokio::fs::write(&cache_path, &bytes).await;
    Ok(bytes.to_vec())
}

/// FNV-1a: a stable, dependency-free hash for cache file names (not security
/// relevant — the allow-list is the guard, and a collision would only serve
/// the wrong card image).
pub(crate) fn fnv1a(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_cdns_are_allowed() {
        assert!(is_allowed_web_image("https://pbs.twimg.com/media/abc.jpg"));
        assert!(is_allowed_web_image("https://cdn.akamai.steamstatic.com/steam/apps/10/header.jpg"));
        assert!(is_allowed_web_image("https://PBS.TWIMG.COM/x.png"));
    }

    #[test]
    fn everything_else_is_refused() {
        for bad in [
            "http://pbs.twimg.com/x.png",
            "https://evil.example/x.png",
            "https://pbs.twimg.com.evil.example/x.png",
            "https://user:pw@pbs.twimg.com/x.png",
            "https://pbs.twimg.com:8443/x.png",
            "https://127.0.0.1/x.png",
            "https://169.254.169.254/latest/meta-data",
            "file:///C:/Windows/win.ini",
            "not a url",
            "",
        ] {
            assert!(!is_allowed_web_image(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_cache_key_is_stable_and_distinguishes_urls() {
        assert_eq!(fnv1a("https://pbs.twimg.com/a.png"), fnv1a("https://pbs.twimg.com/a.png"));
        assert_ne!(fnv1a("https://pbs.twimg.com/a.png"), fnv1a("https://pbs.twimg.com/b.png"));
    }
}
