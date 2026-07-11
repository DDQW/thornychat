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

/// Byte caps enforced on the disk caches by [`evict_cache_dir`] once per
/// startup. Generous — every image, avatar, and emoji ever displayed lands
/// in these directories and would otherwise accumulate forever — but
/// re-fetching an evicted entry is cheap, so the caps don't need headroom
/// for the ages.
pub const MEDIA_CACHE_CAP_BYTES: u64 = 512 * 1024 * 1024;
pub const EMOJI_CACHE_CAP_BYTES: u64 = 64 * 1024 * 1024;

/// How far under the cap an eviction sweeps (75%), so the very next
/// startup isn't immediately due another pass.
const EVICTION_TARGET_NUMERATOR: u64 = 3;
const EVICTION_TARGET_DENOMINATOR: u64 = 4;

/// A `.tmp` file this old is a crash leftover, not an in-flight write.
const STALE_TMP_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Best-effort size cap for a flat cache directory: when the total passes
/// `cap_bytes`, the oldest files (by modification time — write time, since
/// cache hits never touch an entry) are deleted until the directory is back
/// under 75% of the cap. Cache misses re-fetch, so a wrongly evicted file
/// costs one download, never data.
///
/// Two kinds of entries are exempt from the cap:
/// - `*.json` — the emoji cache dir doubles as home for `usage.json` and
///   `stickers.json`, which are state that must survive, not cache.
/// - Fresh `*.tmp` files — an in-flight write's temp file (see [`fetch`]).
///   Stale ones (older than a day: crash leftovers whose rename never
///   happened) are deleted regardless of the cap.
///
/// Synchronous on purpose (a directory walk is many small blocking ops) —
/// call it from `spawn_blocking`, not an async context.
pub fn evict_cache_dir(dir: &Path, cap_bytes: u64) {
    // A missing directory is a fresh install / never-used cache: no-op.
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let now = std::time::SystemTime::now();

    let mut files: Vec<(PathBuf, u64, std::time::SystemTime)> = Vec::new();
    let mut total: u64 = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else { continue };
        if !metadata.is_file() {
            continue;
        }
        // NB: cache filenames keep the homeserver's dots ("matrix.org_id"),
        // so `extension()` returns junk like "org_id" for them — but never
        // exactly "json"/"tmp", since a media id can't contain a dot.
        if path.extension().is_some_and(|ext| ext == "json") {
            continue;
        }
        let modified = metadata.modified().unwrap_or(now);
        if path.extension().is_some_and(|ext| ext == "tmp") {
            if now.duration_since(modified).is_ok_and(|age| age > STALE_TMP_AGE) {
                let _ = std::fs::remove_file(&path);
            }
            continue;
        }
        total += metadata.len();
        files.push((path, metadata.len(), modified));
    }
    if total <= cap_bytes {
        return;
    }

    files.sort_by_key(|(_, _, modified)| *modified);
    let target = cap_bytes / EVICTION_TARGET_DENOMINATOR * EVICTION_TARGET_NUMERATOR;
    let mut evicted_files = 0u64;
    let mut evicted_bytes = 0u64;
    for (path, len, _) in files {
        if total - evicted_bytes <= target {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            evicted_bytes += len;
            evicted_files += 1;
        }
    }
    tracing::info!(
        dir = %dir.display(),
        evicted_files,
        evicted_bytes,
        remaining_bytes = total - evicted_bytes,
        "cache directory was over its cap; oldest entries evicted"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fresh directory under the OS temp dir, removed on drop so failed
    /// asserts don't leave litter behind for the next run to trip on.
    struct TempCacheDir(PathBuf);

    impl TempCacheDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("thornychat-evict-{tag}-{}", std::process::id()));
            // A leftover from a killed previous run would skew the totals.
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempCacheDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Writes `len` bytes and backdates the mtime by `age_secs`, which is
    /// the eviction order's only input.
    fn write_aged(dir: &Path, name: &str, len: usize, age_secs: u64) {
        let path = dir.join(name);
        std::fs::write(&path, vec![0u8; len]).unwrap();
        let mtime = std::time::SystemTime::now() - std::time::Duration::from_secs(age_secs);
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(mtime).unwrap();
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn under_cap_is_untouched() {
        let tmp = TempCacheDir::new("under-cap");
        write_aged(&tmp.0, "matrix.org_aaa", 100, 300);
        write_aged(&tmp.0, "matrix.org_bbb", 100, 200);
        evict_cache_dir(&tmp.0, 1000);
        assert_eq!(names_in(&tmp.0), ["matrix.org_aaa", "matrix.org_bbb"]);
    }

    #[test]
    fn oldest_evicted_down_to_target() {
        let tmp = TempCacheDir::new("oldest-first");
        // 4 × 300 bytes = 1200 total against a 1000 cap; target is 750, so
        // the two oldest must go (1200 → 900 → 600 ≤ 750).
        write_aged(&tmp.0, "matrix.org_oldest", 300, 4000);
        write_aged(&tmp.0, "matrix.org_older", 300, 3000);
        write_aged(&tmp.0, "matrix.org_newer", 300, 2000);
        write_aged(&tmp.0, "matrix.org_newest", 300, 1000);
        evict_cache_dir(&tmp.0, 1000);
        assert_eq!(names_in(&tmp.0), ["matrix.org_newer", "matrix.org_newest"]);
    }

    #[test]
    fn json_state_files_survive_eviction() {
        let tmp = TempCacheDir::new("json-survives");
        // usage.json is both the oldest and the biggest — still exempt.
        write_aged(&tmp.0, "usage.json", 5000, 9000);
        write_aged(&tmp.0, "matrix.org_old", 600, 5000);
        write_aged(&tmp.0, "matrix.org_new", 600, 100);
        evict_cache_dir(&tmp.0, 1000);
        assert_eq!(names_in(&tmp.0), ["matrix.org_new", "usage.json"]);
    }

    #[test]
    fn stale_tmp_removed_fresh_tmp_kept() {
        let tmp = TempCacheDir::new("tmp-files");
        write_aged(&tmp.0, "matrix.org_crashed.tmp", 100, 2 * 24 * 60 * 60);
        write_aged(&tmp.0, "matrix.org_inflight.tmp", 100, 10);
        write_aged(&tmp.0, "matrix.org_kept", 100, 100);
        // Well under the cap: tmp cleanup must not depend on eviction firing.
        evict_cache_dir(&tmp.0, 1000);
        assert_eq!(names_in(&tmp.0), ["matrix.org_inflight.tmp", "matrix.org_kept"]);
    }
}
