//! Structured logging: console output plus rotated files under
//! `%APPDATA%\ThornyChat\ThornyChat\data\<profile>\logs`. Returns a guard that must be kept
//! alive for the process lifetime (dropping it stops the non-blocking
//! writer from flushing).

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

pub fn init(profile: &str) -> Option<WorkerGuard> {
    let paths = match client_core::store::AppPaths::for_profile(profile) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("failed to resolve app data directory for logging: {e}");
            return None;
        }
    };

    let logs_dir = paths.logs_dir();
    if let Err(e) = std::fs::create_dir_all(&logs_dir) {
        eprintln!("failed to create log directory {logs_dir:?}: {e}");
        return None;
    }

    let file_appender = tracing_appender::rolling::daily(&logs_dir, "thornychat.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    // Default to `info`, but silence one specific WARN that matrix-sdk-crypto
    // emits on *every* sync when the account has server-side key backup enabled
    // while this device holds no backup key: "Trying to backup room keys but no
    // backup key was found". It fires every few seconds and buries the logs
    // (tens of thousands of lines a day). Dropping just that target to `error`
    // keeps genuine backup failures visible. A caller-set RUST_LOG still
    // overrides all of this via the env path above.
    //
    // Also silence `matrix_sdk::latest_events` entirely (`=off`, not `=error`,
    // because it logs at ERROR): `SyncService`'s sliding-sync "room-list"
    // connection subscribes to per-room "latest events" before the room object
    // is registered in the client store, so it logs "Room is unknown" for every
    // room on the first sync — plus equally useless INFO "Timer … finished"
    // lines. The app never uses that SDK feature (no `latest_event` reference
    // anywhere, and no Cargo feature or builder option controls it), so the
    // whole module is pure noise.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info,matrix_sdk_crypto::backups=error,matrix_sdk::latest_events=off")
    });

    tracing_subscriber::registry()
        .with(EnvFilter::clone(&filter))
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(fmt::layer().with_writer(non_blocking).with_ansi(false))
        .init();

    Some(guard)
}
