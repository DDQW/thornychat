//! Rich tweet previews via the FxTwitter API (`api.fxtwitter.com`) — the
//! same data source Discord-style embed fixers use. The homeserver's
//! OpenGraph proxy can only surface title/description/one-image, which is
//! nowhere near a real tweet card; FxTwitter returns author (name, handle,
//! avatar, verified), full text, photos, engagement counts, and quoted
//! tweets. Used only for recognized tweet hosts; everything else keeps the
//! homeserver preview path. Note this queries a third-party service
//! directly (like the Twemoji CDN fetches).

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct TweetData {
    pub text: String,
    pub author: TweetAuthor,
    #[serde(default)]
    pub replies: Option<u64>,
    #[serde(default)]
    pub retweets: Option<u64>,
    #[serde(default)]
    pub likes: Option<u64>,
    #[serde(default)]
    pub views: Option<u64>,
    #[serde(default)]
    pub created_timestamp: Option<i64>,
    #[serde(default)]
    pub media: Option<TweetMedia>,
    #[serde(default)]
    pub quote: Option<Box<TweetData>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TweetAuthor {
    pub name: String,
    pub screen_name: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub verification: Option<TweetVerification>,
}

impl TweetAuthor {
    pub fn is_verified(&self) -> bool {
        self.verification.as_ref().is_some_and(|v| v.verified)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TweetVerification {
    #[serde(default)]
    pub verified: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TweetMedia {
    #[serde(default)]
    pub photos: Vec<TweetPhoto>,
    #[serde(default)]
    pub videos: Vec<TweetVideo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TweetPhoto {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TweetVideo {
    #[serde(default)]
    pub thumbnail_url: Option<String>,
}

impl TweetData {
    pub fn photo_urls(&self) -> Vec<&str> {
        self.media.as_ref().map_or_else(Vec::new, |m| {
            m.photos.iter().map(|p| p.url.as_str()).collect()
        })
    }

    pub fn video_thumbnail_urls(&self) -> Vec<&str> {
        self.media.as_ref().map_or_else(Vec::new, |m| {
            m.videos.iter().filter_map(|v| v.thumbnail_url.as_deref()).collect()
        })
    }

    /// Every remote image this tweet needs on screen (avatar, photos, video
    /// stills, and the same for a quoted tweet).
    pub fn all_image_urls(&self) -> Vec<String> {
        let mut urls = Vec::new();
        if let Some(avatar) = &self.author.avatar_url {
            urls.push(avatar.clone());
        }
        urls.extend(self.photo_urls().iter().map(|s| s.to_string()));
        urls.extend(self.video_thumbnail_urls().iter().map(|s| s.to_string()));
        if let Some(quote) = &self.quote {
            urls.extend(quote.all_image_urls());
        }
        urls
    }
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    code: u32,
    #[serde(default)]
    tweet: Option<TweetData>,
}

/// Maps a tweet URL to its FxTwitter API endpoint; `None` when the URL
/// isn't a recognizable `/status/{id}` link on a tweet host.
pub fn tweet_api_url(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let (host, path) = rest.split_once('/')?;
    let host = host.trim_start_matches("www.").to_ascii_lowercase();
    let tweet_host = matches!(
        host.as_str(),
        "x.com"
            | "twitter.com"
            | "mobile.twitter.com"
            | "mobile.x.com"
            | "fxtwitter.com"
            | "vxtwitter.com"
            | "fixupx.com"
            // Nitter mirror: same `/user/status/{id}` shape, and the ids are
            // canonical tweet ids, so FxTwitter resolves them directly.
            | "xcancel.com"
    );
    if !tweet_host {
        return None;
    }

    let path = path.split(['?', '#']).next().unwrap_or("");
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let status_position = segments.iter().position(|s| *s == "status")?;
    let id: String = segments
        .get(status_position + 1)?
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if id.is_empty() {
        return None;
    }
    Some(format!("https://api.fxtwitter.com/status/{id}"))
}

pub async fn fetch(api_url: String) -> anyhow::Result<TweetData> {
    let response =
        crate::twemoji::http_client().get(&api_url).send().await?.error_for_status()?;
    let parsed: ApiResponse = response.json().await?;
    if parsed.code != 200 {
        anyhow::bail!("fxtwitter returned code {}", parsed.code);
    }
    parsed.tweet.ok_or_else(|| anyhow::anyhow!("fxtwitter response missing tweet body"))
}

/// Plain HTTPS image fetch (avatars, tweet photos — `pbs.twimg.com` etc.),
/// cached in memory by the caller.
pub async fn fetch_image(url: String) -> anyhow::Result<Vec<u8>> {
    let response = crate::twemoji::http_client().get(&url).send().await?.error_for_status()?;
    Ok(response.bytes().await?.to_vec())
}
