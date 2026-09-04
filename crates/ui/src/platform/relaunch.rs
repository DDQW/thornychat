//! Restarting the app in place, for the one failure it cannot recover from
//! without a fresh process: the wedged render loop (see [`crate::watchdog`]).
//!
//! The window can't paint during that hang, so there is no way to ask the user
//! anything — the choice is between leaving them with a dead window they have
//! to notice and kill themselves, and doing the restart for them. Fifteen
//! times in the archive it was the latter, by hand, after up to twenty-two
//! hours.

use std::sync::atomic::{AtomicBool, Ordering};

/// Argument marking a process that this module spawned. A process carrying it
/// will not spawn another.
pub const RESTARTED_FLAG: &str = "--restarted-after-hang";

/// Set once at startup from the command line. Process-wide because it is a
/// property of *this* process, not of any one window or account.
static RESTARTED_AFTER_HANG: AtomicBool = AtomicBool::new(false);

/// Records whether this process was itself spawned by a hang restart. Call
/// once during startup, before any window exists.
pub fn set_restarted_after_hang(restarted: bool) {
    RESTARTED_AFTER_HANG.store(restarted, Ordering::Relaxed);
}

/// Whether this process came from a hang restart.
pub fn was_restarted_after_hang() -> bool {
    RESTARTED_AFTER_HANG.load(Ordering::Relaxed)
}

/// Spawns a fresh copy of this executable with the same arguments plus
/// [`RESTARTED_FLAG`], returning whether it started.
///
/// Refuses if this process is already a restart. If the GPU is still wedged the
/// new process will hang the same way within seconds, and without this guard
/// the two would trade places forever, each spawning the next — a far worse
/// failure than the frozen window, because it would never stop. The second
/// process notifies and stays put instead, leaving the user in control.
pub fn restart_after_hang() -> bool {
    if was_restarted_after_hang() {
        tracing::error!(
            "render loop wedged again in a process that was already restarted for it — \
             not restarting a second time; restart manually once the display has recovered"
        );
        return false;
    }

    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            tracing::error!(%error, "cannot locate our own executable to restart");
            return false;
        }
    };

    // Skip argv[0] and any flag we're about to re-add ourselves.
    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|arg| arg != RESTARTED_FLAG)
        .collect();

    match std::process::Command::new(&exe).args(&args).arg(RESTARTED_FLAG).spawn() {
        Ok(child) => {
            tracing::error!(pid = child.id(), ?exe, "restarted after a wedged render loop");
            true
        }
        Err(error) => {
            tracing::error!(%error, ?exe, "failed to restart after a wedged render loop");
            false
        }
    }
}
