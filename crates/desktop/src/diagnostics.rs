//! Settings → General → Diagnostics: reading the current log out, and
//! deleting the stored log files.

use std::path::Path;

use serde::Serialize;

/// What one clear actually did. `files` counts the rotated files removed; the
/// day's active file is emptied rather than removed (see [`clear_log_files`]),
/// so it is reported separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct LogsCleared {
    pub files: usize,
    pub emptied_active: bool,
    pub bytes: u64,
}

/// The text of the most recently written log file for `profile`, for the
/// "Copy log" button.
pub fn read_latest_log(profile: &str) -> Result<String, String> {
    let path = client_core::store::AppPaths::for_profile(profile)
        .map_err(|error| format!("couldn't resolve app data directory: {error}"))?
        .latest_log_file()
        .ok_or_else(|| "no log file found yet".to_string())?;
    std::fs::read_to_string(&path).map_err(|error| format!("couldn't read {}: {error}", path.display()))
}

/// Deletes this profile's log files and returns the one-line result for the
/// Settings screen.
///
/// The day's active file is emptied rather than deleted, and that is not a
/// detail worth hiding: the running process holds it open through the
/// non-blocking appender, and the appender only opens a new file at the next
/// daily rollover. Unlinking it would leave every line written between now and
/// tomorrow going to a file with no name — the log would look like it had
/// simply stopped. Truncating keeps the same handle valid; the appender writes
/// in append mode, so it carries on at the new end of the file, which is zero.
pub fn clear_logs(profile: &str) -> Result<String, String> {
    let paths = client_core::store::AppPaths::for_profile(profile)
        .map_err(|error| format!("couldn't resolve app data directory: {error}"))?;
    let cleared = clear_log_files(&paths.logs_dir(), paths.latest_log_file().as_deref())?;
    // Logged after the truncation, so the emptied file says why it is empty —
    // a log that starts mid-session otherwise reads like a crash. Nothing is
    // written if logging is off, which is fine: there was nothing to clear.
    tracing::info!(files = cleared.files, bytes = cleared.bytes, "log files cleared from Settings");
    Ok(describe_clear(cleared))
}

/// The file walk behind [`clear_logs`], split out so it can be tested against
/// a real directory without a profile or a running app.
///
/// Only files whose name starts with the appender's own prefix are touched, so
/// a stray file someone dropped in the directory is left alone (same posture
/// as the appender's own retention pruning). `active` is the file the appender
/// currently holds open, if any.
pub fn clear_log_files(logs_dir: &Path, active: Option<&Path>) -> Result<LogsCleared, String> {
    let entries = match std::fs::read_dir(logs_dir) {
        Ok(entries) => entries,
        // Nothing has been logged yet (or logging is off and the directory was
        // never created): a no-op, not a failure.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LogsCleared::default())
        }
        Err(error) => return Err(format!("couldn't read {}: {error}", logs_dir.display())),
    };

    let mut cleared = LogsCleared::default();
    let mut failures: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let path = entry.path();
        let is_ours = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(client_core::store::LOG_FILE_PREFIX));
        if !is_ours {
            continue;
        }
        let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);

        let outcome = if Some(path.as_path()) == active {
            std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&path)
                .map(|_| cleared.emptied_active = true)
        } else {
            std::fs::remove_file(&path).map(|()| cleared.files += 1)
        };
        match outcome {
            Ok(()) => cleared.bytes += size,
            Err(error) => failures.push(format!(
                "{}: {error}",
                path.file_name().unwrap_or(path.as_os_str()).to_string_lossy()
            )),
        }
    }

    if failures.is_empty() {
        Ok(cleared)
    } else {
        Err(failures.join("; "))
    }
}

fn describe_clear(cleared: LogsCleared) -> String {
    if cleared.files == 0 && !cleared.emptied_active {
        return "Nothing to delete — there are no log files.".to_string();
    }
    let files = match cleared.files {
        0 => "No stored files to delete".to_string(),
        1 => "Deleted 1 log file".to_string(),
        count => format!("Deleted {count} log files"),
    };
    let active = if cleared.emptied_active { ", emptied today's" } else { "" };
    format!("{files}{active} — {} freed.", format_bytes(cleared.bytes))
}

/// Byte counts at a glance; a log archive runs from a few kilobytes to
/// gigabytes, and raw bytes are unreadable at the top of that range.
fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let bytes = bytes as f64;
    if bytes < KIB {
        format!("{bytes:.0} bytes")
    } else if bytes < KIB * KIB {
        format!("{:.1} KB", bytes / KIB)
    } else if bytes < KIB * KIB * KIB {
        format!("{:.1} MB", bytes / (KIB * KIB))
    } else {
        format!("{:.2} GB", bytes / (KIB * KIB * KIB))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Fresh directory under the OS temp dir, removed on drop.
    struct TempLogsDir(PathBuf);

    impl TempLogsDir {
        fn new(tag: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("thornychat-logs-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn write(&self, name: &str, len: usize) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, vec![b'x'; len]).unwrap();
            path
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempLogsDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The whole point of the button: rotated files go, and the space they
    /// took is reported.
    #[test]
    fn rotated_files_are_deleted() {
        let dir = TempLogsDir::new("rotated");
        dir.write("thornychat.log.2026-09-01", 100);
        dir.write("thornychat.log.2026-09-02", 200);

        let cleared = clear_log_files(dir.path(), None).expect("clear should succeed");

        assert_eq!(cleared.files, 2);
        assert!(!cleared.emptied_active);
        assert_eq!(cleared.bytes, 300);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    /// The file the appender still holds open must survive as an empty file:
    /// unlinking it would send every line written before tomorrow's rollover
    /// to a file with no name.
    #[test]
    fn the_active_file_is_emptied_not_removed() {
        let dir = TempLogsDir::new("active");
        dir.write("thornychat.log.2026-09-03", 500);
        let active = dir.write("thornychat.log.2026-09-04", 900);

        let cleared = clear_log_files(dir.path(), Some(&active)).expect("clear should succeed");

        assert_eq!(cleared.files, 1, "only the rotated file is removed");
        assert!(cleared.emptied_active);
        assert_eq!(cleared.bytes, 1400, "both files' contents are reclaimed");
        assert!(active.exists(), "the appender's file must keep its name");
        assert_eq!(std::fs::metadata(&active).unwrap().len(), 0);
    }

    /// The reason the active file is truncated rather than deleted, checked
    /// against the real filesystem: `tracing_appender` holds it open with
    /// `OpenOptions::append(true).create(true)`, and Windows has opinions
    /// about files that are open. Truncating through a second handle has to
    /// succeed, and the appender's next write has to land at the new start of
    /// the file rather than back at the old offset.
    #[test]
    fn truncating_works_while_the_appender_holds_the_file_open() {
        use std::io::Write;

        let dir = TempLogsDir::new("open-handle");
        let active = dir.write("thornychat.log.2026-09-04", 0);
        let mut appender = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&active)
            .expect("open like tracing-appender does");
        appender.write_all(b"before the clear\n").unwrap();
        appender.flush().unwrap();

        let cleared = clear_log_files(dir.path(), Some(&active)).expect("clear should succeed");
        assert!(cleared.emptied_active);

        appender.write_all(b"after the clear\n").unwrap();
        appender.flush().unwrap();

        assert_eq!(
            std::fs::read_to_string(&active).unwrap(),
            "after the clear\n",
            "the appender must carry on writing from the truncated start"
        );
    }

    /// Same posture as the appender's own retention pruning: this deletes the
    /// app's log files, not whatever else happens to sit in the directory.
    #[test]
    fn foreign_files_are_left_alone() {
        let dir = TempLogsDir::new("foreign");
        dir.write("thornychat.log.2026-09-01", 10);
        let other = dir.write("notes.txt", 10);
        let subdir = dir.path().join("thornychat.log.d");
        std::fs::create_dir(&subdir).unwrap();

        let cleared = clear_log_files(dir.path(), None).expect("clear should succeed");

        assert_eq!(cleared.files, 1);
        assert!(other.exists(), "a file that isn't ours must not be deleted");
        assert!(subdir.exists(), "a directory must not be deleted, prefix or not");
    }

    /// Logging off (or a first run) means no directory at all, and pressing
    /// the button then is a no-op rather than an error.
    #[test]
    fn a_missing_directory_is_not_a_failure() {
        let dir = TempLogsDir::new("missing");
        let absent = dir.path().join("never-created");

        let cleared = clear_log_files(&absent, None).expect("a missing directory is fine");

        assert_eq!(cleared, LogsCleared::default());
        assert_eq!(describe_clear(cleared), "Nothing to delete — there are no log files.");
    }

    #[test]
    fn the_result_line_says_what_happened() {
        let both = LogsCleared { files: 3, emptied_active: true, bytes: 2048 };
        assert_eq!(describe_clear(both), "Deleted 3 log files, emptied today's — 2.0 KB freed.");

        let only_active = LogsCleared { files: 0, emptied_active: true, bytes: 512 };
        assert_eq!(
            describe_clear(only_active),
            "No stored files to delete, emptied today's — 512 bytes freed."
        );

        let one = LogsCleared { files: 1, emptied_active: false, bytes: 1_048_576 };
        assert_eq!(describe_clear(one), "Deleted 1 log file — 1.0 MB freed.");
    }
}
