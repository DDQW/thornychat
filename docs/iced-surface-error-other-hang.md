# Upstream report: unbounded redraw loop on `SurfaceError::Other` (resume from standby)

Draft of an issue to file against [iced-rs/iced](https://github.com/iced-rs/iced/issues).
Written up from a reproduction in ThornyChat.

**Status in this repo (2026-09-04): worked around on three levels.** The issue
below is still accurate about upstream, and still worth filing, but the app no
longer depends on a fix landing.

| | What | Where |
|---|---|---|
| Prevention | The window is closed when Windows announces a suspend, which makes `iced_winit` drop the compositor — so the machine sleeps with no wgpu device of ours to lose, and the window opened on resume builds a new one. This is why the app is an `iced::daemon` rather than an `iced::application`: only a daemon may have no window for a while. | [`crates/ui/src/platform/power.rs`](../crates/ui/src/platform/power.rs), `Message::Power` in [`crates/ui/src/update.rs`](../crates/ui/src/update.rs) |
| Recovery | If the hang happens anyway, 100 present failures inside a minute trip a restart with a toast, from a thread that owes nothing to the wedged event loop. | [`crates/app/src/render_watchdog.rs`](../crates/app/src/render_watchdog.rs) |
| Damage control | Per-callsite rate limit and a retention cap, so a flood costs ~390 KB/hour instead of ~1.7 GB/hour; plus a log level (including off) in Settings. | [`crates/app/src/log_limit.rs`](../crates/app/src/log_limit.rs), [`crates/app/src/logging.rs`](../crates/app/src/logging.rs), [`log-volume-2026-09.md`](log-volume-2026-09.md) |

Measured on the prevention path (simulated standby, release build): 17 ms from
the suspend notification to the window being gone and the device released —
against a Windows budget of roughly two seconds — and a fresh compositor built
3.3 s later on resume, with no error or warning logged anywhere in the cycle.
What that test cannot cover is a real suspend beating the close; that needs a
genuine standby, and the watchdog is what catches it if it ever does.

---

## Title

`SurfaceError::Other` causes an unbounded, un-throttled redraw loop — window
never repaints again after resume from standby (Windows/DX12)

## Summary

When `present` fails with `SurfaceError::Other`, `iced_winit` logs the error and
immediately re-requests a redraw on every window, without attempting any
recovery and without any throttle or give-up. If the condition is persistent —
which it is after a GPU device is removed on resume from standby — this becomes
a tight infinite loop:

- the window never repaints again (the app appears frozen and must be killed),
- a CPU core is pegged,
- `log::error!` fires at ~12,000 times/second, which writes **~1 GB/hour** to
  any file-backed subscriber.

`Lost` and `Outdated` are both recovered from a few lines above; `Other` is the
only failure mode that has no path back.

## Environment

| | |
|---|---|
| iced | 0.14.0 (`iced_winit` 0.14.0) |
| wgpu | 27.0.1 |
| winit | 0.30.13 |
| OS | Windows 11 IoT Enterprise LTSC 2024 (10.0.26300) |
| Backend | DX12 |
| GPU | AMD Radeon RX 7900 XTX, driver 32.0.31041.1004 |

## Steps to reproduce

1. Run any `iced` application on Windows with the DX12 backend.
2. Leave it running and let the machine enter standby.
3. Wake the machine.

The window is blank/frozen from that point on and only a restart recovers it.

Reproduced on 11 separate days over three weeks on the same machine — roughly
every other overnight standby, so it is not a one-off driver hiccup.

## The code

[`iced_winit-0.14.0/src/lib.rs:991`](https://github.com/iced-rs/iced/blob/0.14.0/winit/src/lib.rs)

```rust
Err(error) => match error {
    compositor::SurfaceError::OutOfMemory => {
        panic!("{error:?}");
    }
    compositor::SurfaceError::Outdated
    | compositor::SurfaceError::Lost => {
        // ... recreates (Lost) or reconfigures (Outdated) the surface
        window.raw.request_redraw();
    }
    _ => {                                    // <-- Other, Timeout
        present_span.finish();

        log::error!("Error {error:?} when presenting surface.");

        // Try rendering all windows again next frame.
        for (_id, window) in window_manager.iter_mut() {
            window.raw.request_redraw();      // <-- nothing changed; will fail again
        }
    }
},
```

Nothing in the `_` arm alters the state that caused the failure, so the next
present fails identically, and the `request_redraw` schedules the next attempt
immediately. There is no backoff and no attempt limit.

`iced_wgpu` passes `wgpu::SurfaceError::Other` straight through
(`iced_wgpu-0.14.0/src/window/compositor.rs:255`), and `iced_graphics`
documents it as "Acquiring a texture failed with a generic error" — which is
what DX12 reports for a removed device.

## Evidence

Application log across a standby cycle. The last line before the gap is normal
activity; the gap is the machine asleep; the flood starts on the first frame
after resume:

```
22:08:29.578  INFO  ui::update: media fetched url="mxc://..." len=5168 ...
              << 22 minutes, nothing logged (machine in standby) >>
22:30:58.403  ERROR iced_winit: Error Other when presenting surface.
22:30:58.403  ERROR iced_winit: Error Other when presenting surface.
22:30:58.403  ERROR iced_winit: Error Other when presenting surface.
              ... 24,174 lines in 2.1 seconds, then the process was killed
```

From the first error line to the end of the file, **no other line is ever
written** — the application never runs another frame or logs anything else.

Log sizes for the affected days, all from this single failure mode:

```
2026-08-17  1.3G     2026-08-24  1.3G     2026-08-29  3.7G
2026-08-19  4.2G     2026-08-25  472M     2026-08-30  1.9G
2026-08-20   17G     2026-08-28  2.1G     2026-09-01  899M
2026-08-21  4.8G     2026-08-22  2.1G
```

17 GB in one day is the loop running unattended for roughly 17 hours.

## Suggested fix

Two independent changes; the second is worth making regardless of whether the
first fully recovers.

**1. Attempt recovery on `Other`.** Treat it at least as well as `Lost` and
recreate the surface:

```rust
compositor::SurfaceError::Outdated
| compositor::SurfaceError::Lost
| compositor::SurfaceError::Other => { ... }
```

Caveat: after a device removal, the `wgpu::Device` itself is gone, so a new
surface configured against the dead device will keep failing. A complete fix
likely needs the compositor (adapter + device) to be rebuildable, which isn't
reachable from this layer today. That makes (2) necessary either way.

**2. Bound the retry.** Any error arm that re-requests a redraw without
changing state should back off and eventually stop, e.g. count consecutive
present failures per window and, past a small threshold, stop scheduling
redraws (and log once, not per frame). Failing to draw is bad; spinning a core
and writing a gigabyte an hour about it is much worse, and it turns a
recoverable-looking glitch into "the user's disk filled up".

Even with no recovery at all, a bounded version of this loop would leave a
blank window instead of a hung process — and would have made the root cause
obvious from a handful of log lines instead of 17 GB of them.
