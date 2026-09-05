//! Detects the wedged render loop by counting the fault itself.
//!
//! On resume from standby the GPU surface stops presenting and `iced_winit`
//! logs `Error Other when presenting surface.` per failed frame, forever — it
//! recovers from `Lost` and `Outdated` but not `Other`. The window never paints
//! again and only a restart helps. See
//! `docs/iced-surface-error-other-hang.md`.
//!
//! # Why this counts errors and not redraws
//!
//! The first attempt watched redraw *rate* from a widget, on the assumption
//! that the loop always spins at thousands of frames a second — every hang in
//! the archive averaged 59/s to 20,116/s. A live reproduction on 2026-09-03
//! disproved it: 186 present failures arrived in three short bursts (125 of
//! them inside one second), then the process sat frozen and silent at 117% of
//! a core, producing no redraw events at all. A rate threshold cannot see that,
//! and a widget cannot report it — once the loop is wedged, nothing the UI
//! thread would publish ever gets processed.
//!
//! So this counts the errors directly, in the subscriber, and recovers from a
//! plain thread that owes nothing to iced.
//!
//! # Ordering
//!
//! `Layered::enabled` short-circuits outermost-first, so this layer must be
//! installed *outside* [`crate::log_limit`] — otherwise a throttled flood would
//! be invisible to it, which is exactly the flood worth catching. It never
//! filters anything itself.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tracing::subscriber::Interest;
use tracing::{Level, Metadata, Subscriber};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::{Context, Layer};

/// Window over which present failures are counted.
const FAILURE_WINDOW: Duration = Duration::from_secs(60);

/// Failures within [`FAILURE_WINDOW`] that mean the surface is not coming back.
///
/// Calibrated against every occurrence on record: the one benign case managed a
/// single failure and recovered, while every session that got past ~2,000 had
/// to be killed by hand, and the 2026-09-03 reproduction froze after 125 in a
/// burst. A hundred separates those cleanly.
const FAILURES_TO_TRIP: u32 = 100;

/// Breathing room for the non-blocking log writer to drain before the process
/// goes away. Exiting is abrupt by necessity — the UI thread is wedged and
/// cannot run iced's shutdown — so the final lines need a moment to land.
const FLUSH_GRACE: Duration = Duration::from_millis(400);

/// Counts `iced_winit` present failures and restarts the app when the render
/// loop is past saving.
#[derive(Debug, Clone)]
pub struct RenderWatchdog {
    state: Arc<Mutex<Failures>>,
}

#[derive(Debug, Default)]
struct Failures {
    window_start: Option<Instant>,
    count: u32,
    /// Latched: recovery runs at most once per process.
    tripped: bool,
}

impl Failures {
    /// Records one failure, returning whether this is the moment to act.
    fn record(&mut self, now: Instant) -> bool {
        if self.tripped {
            return false;
        }
        let start = *self.window_start.get_or_insert(now);
        if now.saturating_duration_since(start) > FAILURE_WINDOW {
            self.window_start = Some(now);
            self.count = 0;
        }
        self.count += 1;

        if self.count >= FAILURES_TO_TRIP {
            self.tripped = true;
            return true;
        }
        false
    }
}

impl RenderWatchdog {
    pub fn new() -> Self {
        Self { state: Arc::default() }
    }
}

impl Default for RenderWatchdog {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether this event is `iced_winit` reporting a failed present.
///
/// Matched on target and level rather than message text: `enabled` is handed
/// only the metadata, and it has to run there so throttled events still count.
/// `iced_winit` has never logged anything else at ERROR in three weeks of
/// archived logs, so this is specific in practice.
fn is_present_failure(meta: &Metadata<'_>) -> bool {
    *meta.level() == Level::ERROR && meta.target() == "iced_winit"
}

impl<S: Subscriber> Layer<S> for RenderWatchdog {
    fn register_callsite(&self, _meta: &'static Metadata<'static>) -> Interest {
        // Must be consulted per event, not cached — same reasoning as the rate
        // limiter's.
        Interest::sometimes()
    }

    /// The only events this layer cares about are ERRORs, and saying so is
    /// what keeps it alive when the user turns logging down or off.
    ///
    /// `Layered::pick_level_hint` combines layers with `cmp::max`, and a layer
    /// with no hint contributes `None` — which loses that maximum to whatever
    /// the layer underneath says. With no hint here, a subscriber built around
    /// an `off` filter would set the process-wide max level to `OFF`, every
    /// callsite would be disabled statically, and the present failures this
    /// exists to count would never reach `enabled`. `ERROR` is both the honest
    /// answer and the floor that keeps them coming: quieter than that is not
    /// representable, and anything more verbose wins the `max` on its own.
    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(LevelFilter::ERROR)
    }

    fn enabled(&self, meta: &Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        if is_present_failure(meta) {
            let trip = self
                .state
                .lock()
                .map(|mut failures| failures.record(Instant::now()))
                .unwrap_or(false);
            if trip {
                recover();
            }
        }
        // Purely an observer: filtering is the rate limiter's job.
        true
    }
}

/// Restarts the app on a thread of its own.
///
/// Deliberately not a `Message`: the UI thread is wedged by the time this runs,
/// so anything routed through iced's event loop would never be handled — which
/// is precisely how the first version of this failed. Logging from here also
/// has to happen outside the subscriber callback, or `tracing`'s re-entrancy
/// guard drops it silently.
fn recover() {
    let spawned = std::thread::Builder::new()
        .name("render-watchdog".into())
        .spawn(|| {
            tracing::error!(
                failures = FAILURES_TO_TRIP,
                window_secs = FAILURE_WINDOW.as_secs(),
                "render loop wedged (surface stopped presenting; known iced_winit hang \
                 after standby) — the window cannot paint, restarting"
            );

            let restarting = ui::platform::relaunch::restart_after_hang();
            let (title, body) = if restarting {
                ("ThornyChat is restarting", "Graphics stopped responding after the PC woke up.")
            } else {
                (
                    "ThornyChat needs a restart",
                    "Graphics stopped responding. Close and reopen it once the display has recovered.",
                )
            };
            if let Err(error) = ui::platform::notifications::show(title, body) {
                tracing::warn!(%error, "could not show the render-hang notification");
            }

            if restarting {
                // Let the appender drain, then go. iced's own shutdown is not
                // reachable — the thread that would run it is the stuck one.
                std::thread::sleep(FLUSH_GRACE);
                std::process::exit(0);
            }
        });

    if let Err(error) = spawned {
        tracing::error!(%error, "could not spawn the render-watchdog recovery thread");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_failure_is_not_a_hang() {
        // 2026-08-18: exactly one failure, and the app carried on fine.
        let mut failures = Failures::default();
        assert!(!failures.record(Instant::now()));
    }

    #[test]
    fn a_burst_trips_it() {
        // 2026-09-03: 125 failures inside one second, then a frozen window.
        let mut failures = Failures::default();
        let start = Instant::now();
        let mut tripped_at = None;
        for i in 0..125u32 {
            let now = start + Duration::from_millis(u64::from(i) * 8);
            if failures.record(now) {
                tripped_at = Some(i + 1);
            }
        }
        assert_eq!(tripped_at, Some(FAILURES_TO_TRIP));
    }

    #[test]
    fn a_sustained_flood_trips_it_immediately() {
        // The archived shape: thousands per second.
        let mut failures = Failures::default();
        let start = Instant::now();
        let mut trips = 0;
        for i in 0..20_000u32 {
            if failures.record(start + Duration::from_micros(u64::from(i) * 50)) {
                trips += 1;
            }
        }
        assert_eq!(trips, 1, "recovery must latch, not fire per frame");
    }

    #[test]
    fn failures_spread_thin_never_trip() {
        // One failure a minute is a display doing something odd, not a wedge:
        // each lands in a fresh window.
        let mut failures = Failures::default();
        let start = Instant::now();
        for minute in 0..600u64 {
            let now = start + Duration::from_secs(minute * 61);
            assert!(!failures.record(now), "tripped on an isolated failure at minute {minute}");
        }
    }

    #[test]
    fn the_window_resets_between_unrelated_bursts() {
        let mut failures = Failures::default();
        let start = Instant::now();
        // Just under the bar...
        for i in 0..(FAILURES_TO_TRIP - 1) {
            assert!(!failures.record(start + Duration::from_millis(u64::from(i))));
        }
        // ...then a long quiet stretch, and another near-miss burst.
        let later = start + FAILURE_WINDOW + Duration::from_secs(5);
        for i in 0..(FAILURES_TO_TRIP - 1) {
            assert!(
                !failures.record(later + Duration::from_millis(u64::from(i))),
                "two separate near-misses must not add up to a trip"
            );
        }
    }

    /// End-to-end through a real subscriber stack, in the same order
    /// `logging::init` builds it. Proves the layer is actually consulted for
    /// these events — the wiring, not just the arithmetic. Deliberately stays
    /// under the trip threshold: crossing it would restart the test runner.
    #[test]
    fn the_layer_counts_present_failures_through_a_real_subscriber() {
        use tracing_subscriber::layer::SubscriberExt;

        let watchdog = RenderWatchdog::new();
        let state = Arc::clone(&watchdog.state);
        let subscriber = tracing_subscriber::registry()
            .with(crate::log_limit::RateLimit::new())
            .with(watchdog);

        tracing::subscriber::with_default(subscriber, || {
            for _ in 0..(FAILURES_TO_TRIP - 1) {
                tracing::error!(target: "iced_winit", "Error Other when presenting surface.");
            }
        });

        let failures = state.lock().unwrap();
        assert_eq!(failures.count, FAILURES_TO_TRIP - 1);
        assert!(!failures.tripped);
    }

    /// The watchdog must be asked before the rate limiter, so a throttled
    /// flood is still counted. Checked here by ordering alone: a real
    /// suppression needs more than the limiter's 250-event burst, and emitting
    /// that many would trip recovery and restart the test runner. The trip
    /// arithmetic is covered above; this pins the layer order.
    #[test]
    fn the_watchdog_sits_outside_the_rate_limiter() {
        use tracing_subscriber::layer::SubscriberExt;

        let watchdog = RenderWatchdog::new();
        let state = Arc::clone(&watchdog.state);
        // Same nesting as `logging::init`: limiter inner, watchdog outer.
        let subscriber = tracing_subscriber::registry()
            .with(crate::log_limit::RateLimit::new())
            .with(watchdog);

        let emitted = FAILURES_TO_TRIP - 1;
        tracing::subscriber::with_default(subscriber, || {
            for _ in 0..emitted {
                tracing::error!(target: "iced_winit", "Error Other when presenting surface.");
            }
        });

        assert_eq!(
            state.lock().unwrap().count,
            emitted,
            "the limiter must not be able to hide events from the watchdog"
        );
    }

    /// The subscriber `logging::init` builds when logging is turned off: the
    /// watchdog on its own, no filter and no writers. Someone who wants no
    /// logs has not asked for a window that never recovers from a wedged
    /// render loop, so the counting has to survive that setting.
    #[test]
    fn the_layer_counts_with_logging_turned_off() {
        use tracing_subscriber::layer::SubscriberExt;

        let watchdog = RenderWatchdog::new();
        let state = Arc::clone(&watchdog.state);
        let subscriber = tracing_subscriber::registry().with(watchdog);

        let emitted = FAILURES_TO_TRIP - 1;
        tracing::subscriber::with_default(subscriber, || {
            for _ in 0..emitted {
                tracing::error!(target: "iced_winit", "Error Other when presenting surface.");
            }
        });

        assert_eq!(state.lock().unwrap().count, emitted, "turning logging off must not blind the watchdog");
    }

    /// Pins the level hint that makes the test above possible — see the
    /// method's own comment for why `None` here would be a silent failure
    /// rather than a missing optimisation.
    #[test]
    fn the_layer_hints_at_the_level_it_needs() {
        let hint = Layer::<tracing_subscriber::Registry>::max_level_hint(&RenderWatchdog::new());
        assert_eq!(hint, Some(LevelFilter::ERROR));
    }

    #[test]
    fn only_iced_winit_errors_count() {
        // A cheap guard on the matcher: everything here shares a level or a
        // target with the real thing without being it.
        let error_elsewhere = tracing::metadata!(
            name: "event",
            target: "matrix_sdk",
            level: Level::ERROR,
            fields: &[],
            callsite: &TEST_CALLSITE,
            kind: tracing::metadata::Kind::EVENT,
        );
        assert!(!is_present_failure(&error_elsewhere));
    }

    struct TestCallsite;
    impl tracing::Callsite for TestCallsite {
        fn set_interest(&self, _: Interest) {}
        fn metadata(&self) -> &Metadata<'_> {
            unreachable!("only used as a callsite identity in tests")
        }
    }
    static TEST_CALLSITE: TestCallsite = TestCallsite;
}
