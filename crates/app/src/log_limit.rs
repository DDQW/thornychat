//! A per-callsite rate limit on log events.
//!
//! Retention caps bound how many log *files* survive; they do nothing about a
//! single source screaming. A wedged render loop once wrote 209 million copies
//! of one line in ten hours (17 GB in a day, half a billion lines across three
//! weeks) — see `docs/log-archive-findings-2026-09.md`. This layer makes that
//! shape of failure cost kilobytes instead of gigabytes, whatever produces it,
//! including dependencies we don't control.
//!
//! Each callsite gets its own token bucket, so one runaway source can never
//! starve the rest of the log. The bucket is deliberately generous up front:
//! a normal room load legitimately writes ~156 lines from a single callsite
//! inside one second, and a flat "one per second" rule would throw away 155 of
//! them. Bursts pass untouched; only a *sustained* stream is clamped.
//!
//! Suppression is always accounted for — when a throttled callsite is allowed
//! through again it first emits how many events were dropped. Losing the count
//! would mean losing the rate, and the rate is what made the render-loop bug
//! diagnosable in the first place.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tracing::callsite::Identifier;
use tracing::subscriber::Interest;
use tracing::{Level, Metadata, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

/// Events a single callsite may emit back-to-back before throttling starts.
/// Sized above the largest legitimate burst observed in the archive (156 media
/// lines in one second during a room load) with room to spare.
const BURST: f64 = 250.0;

/// Sustained rate for warnings and errors. A callsite that genuinely has more
/// than one warning a second to report, forever, is malfunctioning — that is
/// exactly the case being defended against.
const REFILL_WARN: f64 = 1.0;

/// Sustained rate for INFO and below. Looser, because this is where the app's
/// ordinary bulk telemetry lives (media fetches, gif decodes) and those lines
/// have been useful for diagnosing cache and decode problems.
const REFILL_INFO: f64 = 20.0;

/// Cap on distinct callsites tracked at once. Callsites are static, so this is
/// really a guard against unbounded growth from macro-generated call sites; a
/// normal build has a few hundred.
const MAX_TRACKED_CALLSITES: usize = 4096;

/// How often the reporter thread publishes what it has dropped.
///
/// A minute, not five seconds: this thread wakes on a fixed timer for the
/// whole life of a process that is meant to run for days, and in the ordinary
/// case it takes the lock, finds nothing suppressed, and goes back to sleep.
/// At five seconds that is ~17k pointless wakeups a day keeping the process
/// off its idle floor. Nothing reads these lines in real time — they exist so
/// a throttled stretch is still accounted for in the log afterwards — so a
/// minute of latency costs nothing, and a flood is still visible within a
/// minute of starting.
const REPORT_EVERY: Duration = Duration::from_secs(60);

type Buckets = Arc<Mutex<HashMap<Identifier, Bucket>>>;

/// Rate-limits log events per callsite. Install ahead of the formatting layers.
#[derive(Debug)]
pub struct RateLimit {
    buckets: Buckets,
}

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    last_refill: Instant,
    /// Events dropped since this callsite was last reported.
    suppressed: u64,
    /// Where this callsite is, captured when tracing registers it. Only
    /// `register_callsite` hands out `&'static Metadata`, and the reporter
    /// needs something owned-or-static to name the source after the fact.
    source: Option<Source>,
}

/// Enough to identify a throttled callsite in the log.
#[derive(Debug, Clone, Copy)]
struct Source {
    target: &'static str,
    line: u32,
}

impl Bucket {
    fn new(now: Instant) -> Self {
        Self { tokens: BURST, last_refill: now, suppressed: 0, source: None }
    }

    /// Refills for elapsed time and spends a token. `false` means "drop this
    /// event", and bumps the suppressed counter.
    fn take(&mut self, now: Instant, refill_per_sec: f64) -> bool {
        let elapsed = now.saturating_duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * refill_per_sec).min(BURST);
        self.last_refill = now;

        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            self.suppressed += 1;
            false
        }
    }
}

/// Sustained rate allowed for `level`.
fn refill_for(level: &Level) -> f64 {
    // tracing orders ERROR as the *most* severe (lowest), so "WARN or worse"
    // is `<= WARN`.
    if *level <= Level::WARN {
        REFILL_WARN
    } else {
        REFILL_INFO
    }
}

impl RateLimit {
    pub fn new() -> Self {
        let buckets: Buckets = Arc::default();
        spawn_reporter(Arc::clone(&buckets));
        Self { buckets }
    }

    /// Spends a token for `meta`, returning whether the event may proceed.
    fn admit(&self, meta: &Metadata<'_>) -> bool {
        let now = Instant::now();
        let Ok(mut buckets) = self.buckets.lock() else {
            // A poisoned mutex means another thread panicked mid-update. Log
            // suppression is not worth propagating that, so fail open.
            return true;
        };

        // Static callsites mean this is bounded in practice; the clear is a
        // backstop, and losing the counters only costs one extra burst.
        if buckets.len() >= MAX_TRACKED_CALLSITES {
            buckets.clear();
        }

        let bucket =
            buckets.entry(meta.callsite()).or_insert_with(|| Bucket::new(now));
        bucket.take(now, refill_for(meta.level()))
    }

    /// Publishes and clears the suppression tallies. Called on a timer by the
    /// reporter thread, and directly by tests.
    ///
    /// Reporting cannot happen inside `enabled`/`on_event`: `tracing` guards
    /// against re-entrant dispatch, so an event emitted from within a
    /// subscriber callback is silently discarded — the summaries simply never
    /// appeared. Hence a thread that is outside any callback when it logs.
    fn report_suppressed(&self) {
        // Collect and release before logging: emitting takes the same lock
        // again on the way through `enabled`, and it is not reentrant.
        let dropped: Vec<(Source, u64)> = {
            let Ok(mut buckets) = self.buckets.lock() else {
                return;
            };
            buckets
                .values_mut()
                .filter(|bucket| bucket.suppressed > 0)
                .map(|bucket| {
                    // Clear the tally whether or not the source is known: the
                    // `log` bridge dispatches through a shared per-level
                    // callsite, so `register_callsite` never runs for those and
                    // `source` stays `None`. Skipping them (as this once did)
                    // left their counters growing forever and reported nothing
                    // — silently losing exactly the numbers that make a flood
                    // legible.
                    let source = bucket.source.unwrap_or(Source {
                        target: "<unregistered callsite>",
                        line: 0,
                    });
                    (source, std::mem::take(&mut bucket.suppressed))
                })
                .collect()
        };

        for (source, suppressed) in dropped {
            tracing::warn!(
                target: "log_limit",
                suppressed,
                source = source.target,
                line = source.line,
                "rate limit: dropped repeated events from this source"
            );
        }
    }
}

/// Runs [`RateLimit::report_suppressed`] forever, so a throttled stretch is
/// always accounted for even if that callsite never fires again.
fn spawn_reporter(buckets: Buckets) {
    std::thread::Builder::new()
        .name("log-rate-limit-reporter".into())
        .spawn(move || {
            let limiter = RateLimit { buckets };
            loop {
                std::thread::sleep(REPORT_EVERY);
                limiter.report_suppressed();
            }
        })
        .ok();
}

impl Default for RateLimit {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: Subscriber> Layer<S> for RateLimit {
    fn register_callsite(&self, meta: &'static Metadata<'static>) -> Interest {
        // The only place a `&'static Metadata` is offered, so it's where the
        // reporter's description of this callsite gets recorded.
        if let Ok(mut buckets) = self.buckets.lock() {
            let bucket = buckets
                .entry(meta.callsite())
                .or_insert_with(|| Bucket::new(Instant::now()));
            bucket.source =
                Some(Source { target: meta.target(), line: meta.line().unwrap_or(0) });
        }

        // `sometimes` is load-bearing: the default caches interest per callsite
        // and `enabled` would never be consulted again, silently disabling the
        // whole limiter.
        Interest::sometimes()
    }

    fn enabled(&self, meta: &Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        self.admit(meta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn bucket() -> (Bucket, Instant) {
        let now = Instant::now();
        (Bucket::new(now), now)
    }

    #[test]
    fn a_full_burst_passes_untouched() {
        // The measured worst case: 156 lines from one callsite in one second.
        let (mut b, now) = bucket();
        for _ in 0..156 {
            assert!(b.take(now, REFILL_WARN));
        }
        assert_eq!(b.suppressed, 0);
    }

    #[test]
    fn sustained_output_collapses_to_the_refill_rate() {
        let (mut b, now) = bucket();
        // Drain the burst.
        for _ in 0..BURST as usize {
            assert!(b.take(now, REFILL_WARN));
        }
        assert!(!b.take(now, REFILL_WARN));

        // A second later exactly one more gets through, then it's shut again.
        let later = now + Duration::from_secs(1);
        assert!(b.take(later, REFILL_WARN));
        assert!(!b.take(later, REFILL_WARN));
    }

    #[test]
    fn suppressed_events_are_counted_exactly() {
        let (mut b, now) = bucket();
        for _ in 0..BURST as usize {
            b.take(now, REFILL_WARN);
        }
        for _ in 0..1000 {
            assert!(!b.take(now, REFILL_WARN));
        }
        assert_eq!(b.suppressed, 1000);
    }

    #[test]
    fn warnings_and_info_get_different_sustained_rates() {
        assert_eq!(refill_for(&Level::ERROR), REFILL_WARN);
        assert_eq!(refill_for(&Level::WARN), REFILL_WARN);
        assert_eq!(refill_for(&Level::INFO), REFILL_INFO);
        assert_eq!(refill_for(&Level::DEBUG), REFILL_INFO);
    }

    #[test]
    fn tokens_never_bank_beyond_the_burst() {
        // An idle callsite shouldn't accumulate an unlimited allowance and
        // then let a flood straight through.
        let (mut b, now) = bucket();
        let much_later = now + Duration::from_secs(3600);
        assert!(b.take(much_later, REFILL_INFO));
        assert!(b.tokens <= BURST);
    }

    #[test]
    fn a_ten_hour_flood_is_bounded() {
        // The 2026-08-20 shape: 20,000 events/second for ten hours. Simulated
        // one second at a time (the bucket only cares about elapsed time).
        let (mut b, mut now) = bucket();
        let mut allowed = 0u64;
        for _ in 0..(10 * 3600) {
            for _ in 0..20_000 {
                if b.take(now, REFILL_WARN) {
                    allowed += 1;
                }
            }
            now += Duration::from_secs(1);
        }
        // Burst + one per second, versus the 720 million that would otherwise
        // have been written.
        assert_eq!(allowed, BURST as u64 + 10 * 3600 - 1);
        assert!(allowed < 40_000);
    }

    /// Collects formatted output so a real subscriber can be inspected.
    #[derive(Clone, Default)]
    struct Capture(std::sync::Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
        type Writer = Self;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// End-to-end through a real subscriber, not just the bucket. This is the
    /// test that catches the `Interest` trap: if `register_callsite` returned
    /// the default, tracing would cache interest per callsite, `enabled` would
    /// never be consulted again, and the limiter would silently do nothing
    /// while every unit test above still passed.
    #[test]
    fn the_layer_actually_throttles_a_live_subscriber() {
        use tracing_subscriber::layer::SubscriberExt;

        let capture = Capture::default();
        let limiter = RateLimit::new();
        let buckets = Arc::clone(&limiter.buckets);
        let subscriber = tracing_subscriber::registry().with(limiter).with(
            tracing_subscriber::fmt::layer().with_ansi(false).with_writer(capture.clone()),
        );

        tracing::subscriber::with_default(subscriber, || {
            // One callsite, hammered the way the render hang hammered its own.
            for _ in 0..5_000 {
                tracing::error!("wedged");
            }
            // Force the periodic report instead of sleeping through it.
            RateLimit { buckets }.report_suppressed();
        });

        let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        let wedged = output.matches("wedged").count();
        // Real time barely advances across the loop, so essentially only the
        // burst gets through.
        assert!(
            (BURST as usize..=BURST as usize + 5).contains(&wedged),
            "expected ~{BURST} lines through, got {wedged}"
        );
        // And the drop must be accounted for, not silent.
        assert!(
            output.contains("rate limit: dropped repeated events"),
            "suppression summary missing from:
{output}"
        );
    }

    #[test]
    fn distinct_callsites_get_independent_budgets() {
        use tracing_subscriber::layer::SubscriberExt;

        let capture = Capture::default();
        let subscriber = tracing_subscriber::registry()
            .with(RateLimit::new())
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(capture.clone()),
            );

        tracing::subscriber::with_default(subscriber, || {
            for _ in 0..5_000 {
                tracing::error!("noisy");
            }
            // A different callsite must not inherit the flood's exhausted
            // bucket — one runaway source can't silence the rest of the log.
            tracing::error!("quiet neighbour");
        });

        let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert!(output.contains("quiet neighbour"), "an unrelated callsite was starved");
    }

    #[test]
    fn a_quiet_callsite_recovers_its_full_burst() {
        let (mut b, now) = bucket();
        for _ in 0..BURST as usize {
            b.take(now, REFILL_INFO);
        }
        assert!(!b.take(now, REFILL_INFO));

        // Idle long enough to refill at 20/s, then a fresh burst is fine.
        let rested = now + Duration::from_secs(30);
        for _ in 0..BURST as usize {
            assert!(b.take(rested, REFILL_INFO));
        }
    }
}
