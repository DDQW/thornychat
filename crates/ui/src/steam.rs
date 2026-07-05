//! Rich Steam store previews via the storefront `appdetails` API
//! (`store.steampowered.com/api/appdetails`) — the same JSON the store
//! widget embeds use. The homeserver's OpenGraph proxy only surfaces
//! title/description/image; the storefront API adds what makes a Steam
//! card recognizable: capsule art, platforms, and live pricing with the
//! discount percentage. Prices come back in the currency Steam picks for
//! our IP, matching what the user would see in their browser. Used only
//! for recognized store links; everything else keeps the homeserver
//! preview path. Like FxTwitter/Twemoji, this queries a third-party
//! service directly.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct SteamAppData {
    pub name: String,
    #[serde(default)]
    pub short_description: Option<String>,
    #[serde(default)]
    pub header_image: Option<String>,
    #[serde(default)]
    pub is_free: bool,
    #[serde(default)]
    pub platforms: Option<SteamPlatforms>,
    #[serde(default)]
    pub price_overview: Option<SteamPrice>,
    #[serde(default)]
    pub release_date: Option<SteamReleaseDate>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SteamPlatforms {
    #[serde(default)]
    pub windows: bool,
    #[serde(default)]
    pub mac: bool,
    #[serde(default)]
    pub linux: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SteamPrice {
    #[serde(default)]
    pub discount_percent: u32,
    /// Pre-discount price, already formatted with the currency symbol.
    /// Steam sends an empty string when there is no discount.
    #[serde(default)]
    pub initial_formatted: String,
    #[serde(default)]
    pub final_formatted: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SteamReleaseDate {
    #[serde(default)]
    pub coming_soon: bool,
    #[serde(default)]
    pub date: String,
}

/// The API response is keyed by the requested app id:
/// `{"<appid>": {"success": true, "data": {...}}}`.
#[derive(Debug, Deserialize)]
struct ApiEntry {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    data: Option<SteamAppData>,
}

/// Maps a Steam store URL to its `appdetails` API endpoint; `None` when
/// the URL isn't a recognizable `/app/{id}` store link.
pub fn steam_api_url(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let (host, path) = rest.split_once('/')?;
    let host = host.trim_start_matches("www.").to_ascii_lowercase();

    let path = path.split(['?', '#']).next().unwrap_or("");
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    let id_segment = match host.as_str() {
        // store.steampowered.com/app/{id}/{slug}, possibly behind
        // /agecheck/app/{id} for mature-rated titles.
        "store.steampowered.com" => {
            let app_position = segments.iter().position(|s| *s == "app")?;
            *segments.get(app_position + 1)?
        }
        // Steam's own shortlinks: s.team/a/{id}.
        "s.team" => {
            if segments.first() != Some(&"a") {
                return None;
            }
            segments.get(1)?
        }
        _ => return None,
    };

    if id_segment.is_empty() || !id_segment.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(format!("https://store.steampowered.com/api/appdetails?appids={id_segment}"))
}

pub async fn fetch(api_url: String) -> anyhow::Result<SteamAppData> {
    let response =
        crate::twemoji::http_client().get(&api_url).send().await?.error_for_status()?;
    let parsed: std::collections::HashMap<String, ApiEntry> = response.json().await?;
    // Single-app request, so the map has exactly one entry — keyed by the
    // id we asked for, which we no longer need to reconstruct.
    let entry = parsed.into_values().next().ok_or_else(|| anyhow::anyhow!("empty appdetails response"))?;
    if !entry.success {
        anyhow::bail!("appdetails reported success=false (unknown or region-locked app)");
    }
    entry.data.ok_or_else(|| anyhow::anyhow!("appdetails response missing data body"))
}
