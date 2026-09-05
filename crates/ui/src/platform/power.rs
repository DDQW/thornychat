//! Windows suspend/resume notifications, so the app can put its GPU device
//! away before the machine sleeps and take a fresh one on the way back.
//!
//! # Why the app cares that the machine is going to sleep
//!
//! On resume the GPU device this process holds is frequently gone, and
//! `iced_winit` has no path back from that: `present` fails with
//! `SurfaceError::Other` forever, the window never paints again, and the
//! redraw loop spins a core writing the same error line thousands of times a
//! second (`docs/iced-surface-error-other-hang.md`; 15 occurrences and 39 GB
//! of logs in three weeks). `crate::render_watchdog` in the binary crate
//! catches that after the fact by restarting the process.
//!
//! This module is how the app stops being in that position at all. Windows
//! announces a suspend *before* it happens, which is the one moment when the
//! device can still be released cleanly: closing the last window makes
//! `iced_winit` drop the whole compositor — wgpu instance, adapter and device
//! (`iced_winit-0.14.0/src/lib.rs`, `*compositor = None` once the window
//! manager is empty) — so the machine sleeps with nothing of ours on the GPU,
//! and the window opened on resume builds a brand-new device.
//!
//! # Blocking in the callback
//!
//! The suspend notification is delivered on a system thread, and Windows waits
//! for it to return before continuing the transition. That wait is the only
//! reason this is useful: the callback hands the event to the UI thread and
//! then blocks, up to [`SUSPEND_GRACE`], for the acknowledgement that the
//! window is actually gone. Returning immediately would race the suspend and
//! usually lose. Windows allows a couple of seconds here; overrunning it is
//! not fatal (the transition continues regardless), it just means this attempt
//! did not beat the sleep — and then the watchdog is still there.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Power::{
    PowerRegisterSuspendResumeNotification, DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DEVICE_NOTIFY_CALLBACK, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND,
};

/// How long the suspend callback waits for the window to actually close.
///
/// Windows' documented budget for these callbacks is around two seconds, and
/// closing one window is far quicker than that in practice — this is a ceiling
/// for the case where the UI thread is busy or already wedged, not an expected
/// wait.
const SUSPEND_GRACE: std::time::Duration = std::time::Duration::from_millis(1_500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerEvent {
    /// The machine is about to sleep. Everything the GPU holds should be gone
    /// before this is acknowledged.
    Suspending,
    /// The machine is awake again.
    Resumed,
}

static SENDER: OnceLock<UnboundedSender<PowerEvent>> = OnceLock::new();
static RECEIVER: Mutex<Option<UnboundedReceiver<PowerEvent>>> = Mutex::new(None);

/// Bumped by [`acknowledge_suspend`]; the callback waits for it to change.
static ACKS: AtomicU64 = AtomicU64::new(0);
static ACK_SIGNAL: Condvar = Condvar::new();
/// Only exists because `Condvar::wait_timeout` needs a mutex to pair with —
/// the state being waited on is the atomic above.
static ACK_LOCK: Mutex<()> = Mutex::new(());

/// Registers for suspend/resume notifications and returns the receiving end,
/// once. Later calls return `None`: the subscription that owns the receiver is
/// started once for the life of the process.
///
/// The registration handle is deliberately never unregistered — it is wanted
/// for exactly as long as the process runs, and dropping it during shutdown
/// would only add a way for teardown to go wrong.
pub fn subscribe() -> Option<UnboundedReceiver<PowerEvent>> {
    if SENDER.get().is_none() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        if SENDER.set(tx).is_ok() {
            *RECEIVER.lock().expect("power receiver mutex") = Some(rx);
            register();
            simulate_from_env();
        }
    }
    RECEIVER.lock().expect("power receiver mutex").take()
}

/// Fires a fake standby cycle, for testing the sleep path without sleeping the
/// machine: `THORNYCHAT_SIMULATE_STANDBY=20,8` suspends 20 seconds after
/// launch and resumes 8 seconds later.
///
/// This exists because the interesting part — the window closing, the
/// compositor being dropped, and a *new* wgpu device being built for the
/// window that replaces it — is otherwise only reachable by putting the whole
/// PC to sleep, which is a poor edit-run-check loop and untestable in CI. The
/// one thing it cannot exercise is the Win32 callback itself (including
/// whether the close beats the real suspend), so a genuine standby is still
/// the final word.
fn simulate_from_env() {
    let Ok(spec) = std::env::var("THORNYCHAT_SIMULATE_STANDBY") else { return };
    let Some((sleep_after, wake_after)) = parse_standby_spec(&spec) else {
        tracing::warn!(%spec, "THORNYCHAT_SIMULATE_STANDBY must look like `20,8`");
        return;
    };
    tracing::warn!(sleep_after, wake_after, "simulating a standby cycle");
    let _ = std::thread::Builder::new().name("standby-sim".into()).spawn(move || {
        let Some(sender) = SENDER.get() else { return };
        std::thread::sleep(std::time::Duration::from_secs(sleep_after));
        let _ = sender.send(PowerEvent::Suspending);
        std::thread::sleep(std::time::Duration::from_secs(wake_after));
        let _ = sender.send(PowerEvent::Resumed);
    });
}

/// Tells a waiting suspend callback that the window is closed and the GPU
/// device is released. Safe to call when nothing is waiting.
pub fn acknowledge_suspend() {
    ACKS.fetch_add(1, Ordering::SeqCst);
    ACK_SIGNAL.notify_all();
}

fn register() {
    // Leaked on purpose: with `DEVICE_NOTIFY_CALLBACK` the "recipient" handle
    // is a pointer to this structure, and nothing documents the system as
    // copying it — it has to stay valid for as long as notifications may
    // arrive, which is the life of the process.
    let params = Box::leak(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
        Callback: Some(on_power_event),
        Context: std::ptr::null_mut(),
    }));
    let mut handle: *mut core::ffi::c_void = std::ptr::null_mut();
    // SAFETY: `params` is `'static`, and the callback it names is a plain
    // function that borrows nothing; `Context` is null because the callback
    // reaches its state through the statics above.
    let result = unsafe {
        PowerRegisterSuspendResumeNotification(
            DEVICE_NOTIFY_CALLBACK,
            HANDLE(params as *mut _ as *mut core::ffi::c_void),
            &mut handle,
        )
    };
    if result.is_err() {
        tracing::warn!(
            error = ?result,
            "could not register for suspend/resume notifications — the app will rely on the \
             render watchdog if the GPU is lost across standby"
        );
    } else {
        tracing::info!("registered for suspend/resume notifications");
    }
}

/// The system's callback. Runs on a Windows-owned thread, so it does the
/// minimum: publish the event, and — for a suspend — wait for the UI thread to
/// say the window is gone.
unsafe extern "system" fn on_power_event(
    _context: *const core::ffi::c_void,
    event_type: u32,
    _setting: *const core::ffi::c_void,
) -> u32 {
    let event = match event_type {
        PBT_APMSUSPEND => PowerEvent::Suspending,
        // Both resume flavours matter: `RESUMEAUTOMATIC` always arrives, and
        // `RESUMESUSPEND` additionally arrives when a user is present. Either
        // is a fine trigger, and the update side ignores a duplicate.
        PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => PowerEvent::Resumed,
        _ => return 0,
    };

    let Some(sender) = SENDER.get() else { return 0 };
    let before = ACKS.load(Ordering::SeqCst);
    if sender.send(event).is_err() {
        return 0;
    }

    if event == PowerEvent::Suspending {
        let started = std::time::Instant::now();
        let mut guard = ACK_LOCK.lock().expect("power ack mutex");
        while ACKS.load(Ordering::SeqCst) == before {
            let remaining = SUSPEND_GRACE.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                break;
            }
            let (next, timeout) =
                ACK_SIGNAL.wait_timeout(guard, remaining).expect("power ack wait");
            guard = next;
            if timeout.timed_out() {
                break;
            }
        }
    }
    0
}

/// `"20,8"` -> sleep after 20 s, wake 8 s later. Anything else is a typo, and
/// a typo must not silently mean "never sleep" *or* "sleep immediately".
fn parse_standby_spec(spec: &str) -> Option<(u64, u64)> {
    let (sleep_after, wake_after) = spec.split_once(',')?;
    Some((sleep_after.trim().parse().ok()?, wake_after.trim().parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_standby_spec_is_two_numbers() {
        assert_eq!(parse_standby_spec("20,8"), Some((20, 8)));
        assert_eq!(parse_standby_spec(" 20 , 8 "), Some((20, 8)));
        assert_eq!(parse_standby_spec("0,0"), Some((0, 0)));
    }

    #[test]
    fn anything_else_is_rejected_rather_than_guessed() {
        for spec in ["", "20", "20,", ",8", "20,8,3", "soon,later", "-1,8", "20;8"] {
            assert_eq!(parse_standby_spec(spec), None, "{spec:?} should not parse");
        }
    }
}
