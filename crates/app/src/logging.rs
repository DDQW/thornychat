//! Structured logging: console output plus rotated files under
//! `%APPDATA%\Synapse\<profile>\logs`. Returns a guard that must be kept
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

    let file_appender = tracing_appender::rolling::daily(&logs_dir, "synapse.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(EnvFilter::clone(&filter))
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(fmt::layer().with_writer(non_blocking).with_ansi(false))
        .init();

    Some(guard)
}
