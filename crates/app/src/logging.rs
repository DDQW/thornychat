//! Structured logging: console output plus daily-rotated files under
//! `%APPDATA%\ThornyChat\ThornyChat\data\<profile>\logs`, keeping the most
//! recent [`LOG_FILES_KEPT`] of them. Returns a guard that must be kept alive
//! for the process lifetime (dropping it stops the non-blocking writer from
//! flushing).
//!
//! How much gets written is the user's choice — Settings → General →
//! Diagnostics, stored in [`ui::log_config`] — from `trace` down to nothing at
//! all. It is read once, here, before the first line: the level reaches the
//! whole process through the subscriber, and a subscriber can only be
//! installed once, so a change takes effect on the next launch.

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

use ui::log_config::LogConfig;

/// How many daily log files to keep. Two weeks is enough history for the
/// Settings → Diagnostics "copy log" flow to be useful after a weekend, while
/// bounding what the archive can grow to.
const LOG_FILES_KEPT: usize = 14;

/// Targets that are noise at any level, silenced on top of whatever the user
/// picked.
///
/// - `matrix_sdk_crypto::backups` emits one WARN on *every* sync when the
///   account has server-side key backup enabled while this device holds no
///   backup key: "Trying to backup room keys but no backup key was found". It
///   fires every few seconds and buries the logs (tens of thousands of lines a
///   day). Dropping just that target to `error` keeps genuine backup failures
///   visible.
/// - `matrix_sdk::latest_events` goes to `off`, not `error`, because it logs
///   *at* ERROR: `SyncService`'s sliding-sync "room-list" connection
///   subscribes to per-room "latest events" before the room object is
///   registered in the client store, so it logs "Room is unknown" for every
///   room on the first sync — plus equally useless INFO "Timer … finished"
///   lines. The app never uses that SDK feature (no `latest_event` reference
///   anywhere, and no Cargo feature or builder option controls it), so the
///   whole module is pure noise.
/// - `matrix_sdk_ui::timeline::tasks` is the same kind of noise from a
///   different corner: it logs "No avatar changes to update for <room>" on
///   essentially every live update (355 lines in the archive) and has never
///   said anything actionable.
///
/// These stay clamped even at `debug`/`trace`: they are not quiet-mode
/// trimming, they are three sources that have never carried information. A
/// caller-set `RUST_LOG` replaces this string wholesale and can bring them
/// back.
const SILENCED_TARGETS: &str = "matrix_sdk_crypto::backups=error,matrix_sdk::latest_events=off,\
                                matrix_sdk_ui::timeline::tasks=error";

pub fn init(profile: &str) -> Option<WorkerGuard> {
    // `RUST_LOG` wins over the stored setting, the same escape hatch the GPU
    // preferences give (`WindowConfig::apply_gpu_preference`): someone who
    // exports a filter to chase a bug gets it, whatever the picker says, and
    // does not have to remember to put the setting back afterwards.
    let env_filter = EnvFilter::try_from_default_env().ok();
    let level = LogConfig::load_or_default().level;

    if level.is_off() && env_filter.is_none() {
        init_watchdog_only();
        return None;
    }

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
        .filename_prefix(client_core::store::LOG_FILE_PREFIX)
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

    let filter = env_filter
        .unwrap_or_else(|| EnvFilter::new(format!("{},{SILENCED_TARGETS}", level.directive())));

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

/// "Off" — the whole subscriber is the render watchdog and nothing else.
///
/// Off has to mean off: no writers, no filter layer, and no appender, so the
/// log directory is never created and not a byte is written. Note what is
/// *not* here — an `EnvFilter::new("off")`. That would look equivalent and
/// would quietly disable the hang recovery: `Layered::pick_level_hint` takes
/// `cmp::max` of the layers' hints, so an `off` filter beneath the watchdog
/// pulls the process-wide max level to `OFF`, every callsite is then disabled
/// statically, and `iced_winit`'s present failures never reach the layer that
/// counts them. Leaving the filter out entirely keeps the watchdog's own
/// `ERROR` hint as the only one, which is exactly the callsites it needs.
///
/// The watchdog stays because it is not logging: it is what restarts the app
/// when the render loop wedges after standby (see [`crate::render_watchdog`]).
/// Turning the logs off is a request for quiet, not for a frozen window.
fn init_watchdog_only() {
    tracing_subscriber::registry().with(crate::render_watchdog::RenderWatchdog::new()).init();
}
