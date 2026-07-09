//! Disk-backed cache for downloaded media (images, custom emoji, avatars),
//! keyed by mxc URI under `AppPaths::media_cache_dir()`. This is the single
//! persistent media cache: the SDK's own `use_cache` layer is deliberately
//! bypassed — our client is built with a persistent sqlite
//! `EventCacheStore`, so `use_cache=true` would write a second copy of
//! every download into SQLite that the early flat-file return below would
//! then never read back.

use std::path::{Path, PathBuf};

use matrix_sdk::media::{MediaFormat, MediaRequestParameters};
use matrix_sdk::ruma::events::room::MediaSource;
use matrix_sdk::ruma::OwnedMxcUri;
use matrix_sdk::Client;

/// Fetches the bytes for `mxc_url`, checking the on-disk cache first.
pub async fn fetch(client: &Client, cache_dir: &Path, mxc_url: &str) -> anyhow::Result<Vec<u8>> {
    let cache_path = cache_path_for(cache_dir, mxc_url);

    if let Ok(bytes) = tokio::fs::read(&cache_path).await {
        return Ok(bytes);
    }

    let uri = OwnedMxcUri::from(mxc_url);
    let request = MediaRequestParameters { source: MediaSource::Plain(uri), format: MediaFormat::File };
    let bytes = client.media().get_media_content(&request, false).await?;

    if let Some(parent) = cache_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    // Temp-file + rename: `fs::write` truncates first, so a crash or full
    // disk mid-write would otherwise leave a corrupt file that the cache-hit
    // path above then serves forever.
    //
    // The temp name must be derived by APPENDING to the real filename, not
    // via `Path::with_extension` — `cache_path_for`'s filenames are
    // `<server>_<media-id>` (e.g. `matrix.org_TQtMBGzGjgtdYfjYIMtXSLjZ`), and
    // `with_extension` replaces everything after the *first* dot. Since the
    // server name itself contains a dot, that collapsed every mxc URL from
    // the same homeserver to the identical temp path (`matrix.tmp`). A pack
    // load fires a dozen-plus concurrent fetches for the same server, so
    // those writes raced on that one shared file; whichever fetch renamed
    // last could carry a *different* URL's bytes into its own permanent
    // cache slot — and since the cache-hit path above never revalidates,
    // that wrong association stuck forever. (Confirmed: two custom emoji
    // with different mxc URLs were shown to return byte-identical content
    // — same length, same hash — despite being visually distinct in Cinny.)
    let mut tmp_name = cache_path.file_name().expect("cache_path always has a file name").to_os_string();
    tmp_name.push(".tmp");
    let tmp_path = cache_path.with_file_name(tmp_name);
    if tokio::fs::write(&tmp_path, &bytes).await.is_ok() {
        let _ = tokio::fs::rename(&tmp_path, &cache_path).await;
    }

    Ok(bytes)
}

fn cache_path_for(cache_dir: &Path, mxc_url: &str) -> PathBuf {
    // mxc://server/media_id -> a flat, filesystem-safe filename.
    let safe_name: String = mxc_url
        .trim_start_matches("mxc://")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' })
        .collect();
    cache_dir.join(safe_name)
}
