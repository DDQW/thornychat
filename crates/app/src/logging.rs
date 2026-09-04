//! Structured logging: console output plus daily-rotated files under
//! `%APPDATA%\ThornyChat\ThornyChat\data\<profile>\logs`, keeping the most
//! recent [`LOG_FILES_KEPT`] of them. Returns a guard that must be kept alive
//! for the process lifetime (dropping it stops the non-blocking writer from
//! flushing).

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

/// How many daily log files to keep. Two weeks is enough history for the
/// Settings → Diagnostics "copy log" flow to be useful after a weekend, while
/// bounding what the archive can grow to.
const LOG_FILES_KEPT: usize = 14;

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

    // Daily rotation with a hard retention cap. `rolling::daily` (the obvious
    // call) keeps every file forever: a single bad day can write gigabytes —
    // a wedged render loop logging the same error thousands of times a second
    // will do it — and nothing ever reclaims them. Capping the *count* doesn't
    // bound a single day's size, but it does stop the archive growing without
    // limit, which is what actually filled the disk.
    //
    // Pruning is by filename prefix and runs at each rollover, so it only ever
    // touches this app's own `thornychat.log.*` files.
    let file_appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("thornychat.log")
        .max_log_files(LOG_FILES_KEPT)
        .build(&logs_dir);
    let file_appender = match file_appender {
        Ok(appender) => appender,
        Err(e) => {
            eprintln!("failed to create the rolling log appender in {logs_dir:?}: {e}");
            return None;
        }
    };
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
    //
    // `matrix_sdk_ui::timeline::tasks` is the same kind of noise from a
    // different corner: it logs "No avatar changes to update for <room>" on
    // essentially every live update (355 lines in the archive) and has never
    // said anything actionable.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(
            "info,matrix_sdk_crypto::backups=error,matrix_sdk::latest_events=off,\
             matrix_sdk_ui::timeline::tasks=error",
        )
    });

    // The rate limit goes on before the filters and the writers: a runaway
    // source is stopped once, for every output, rather than per sink. See
    // `log_limit` for why it is generous with bursts and strict with sustained
    // output.
    // Layer order matters, and not in the obvious direction: `Layered::enabled`
    // is evaluated outermost-first and short-circuits, so the render watchdog
    // goes on *last* to be asked *first*. Installed any deeper, the rate limit
    // would swallow a flood before the watchdog could count it — and a flood is
    // exactly what it exists to notice.
    tracing_subscriber::registry()
        .with(crate::log_limit::RateLimit::new())
        .with(EnvFilter::clone(&filter))
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(fmt::layer().with_writer(non_blocking).with_ansi(false))
        .with(crate::render_watchdog::RenderWatchdog::new())
        .init();

    Some(guard)
}
