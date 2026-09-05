# What the log settings actually cost — measured 2026-09-04

Companion to [`log-archive-findings-2026-09.md`](log-archive-findings-2026-09.md),
which said *what went wrong* in 39 GB of logs. This one answers the question
that follows from it: how much does ThornyChat write, what does each new
**Settings → General → Diagnostics → Detail level** choice change, and how does
that compare with the client it is measured against (Cinny, running on the same
machine, same account).

Everything below is measured on this machine, against the real profile, with
the release build of the current tree. No projections except where labelled.

---

## 1. The archive as it stands

13 daily files still on disk, scanned end to end:

| | |
|---|---|
| Total | **10.39 GB**, 131,167,407 lines |
| Render-hang flood lines | 131,128,090 — **99.97%** of every line |
| Quiet days (no hang) | 0.05 – 2.1 MB/day |
| Worst day (2026-08-29) | **3.90 GB**, 46.9 M lines, 1.9 h of flood |

And the part that motivated an off switch:

| ThornyChat data directory | Size |
|---|---:|
| **logs** | **10,385 MB (94.5%)** |
| media-cache | 540 MB |
| webview-data | 45 MB |
| store (all sqlite) | 18 MB |
| emoji-cache | 4 MB |

The logs are nineteen times the size of everything else the app keeps, and
until now there was no way to ask for less of them.

### What a quiet day is made of

`thornychat.log.2026-09-04`, before the measurement runs (1.05 MB, 17 h):

| Lines | Bytes | Source |
|---:|---:|---|
| 11,065 | 364 KB | **continuation lines** — `iced_winit`'s `WindowAttributes` and `iced_wgpu`'s `Settings` pretty-printed over ~1,000 lines *per launch* |
| 1,867 | 297 KB | `ui::update` |
| 485 | 43 KB | `client_core::rooms::room_list` (`room list updated rooms=8`, 482×) |
| 228 | 43 KB | `client_core::rooms::emoji_packs` |

77% of a quiet day's lines are one dependency pretty-printing two structs at
startup. Worth knowing before optimising anything of ours.

---

## 2. Per level, on the real profile

Same account, same rooms, 180 s per arm, release build, each instance closed
cleanly so the non-blocking appender flushed before measuring. The user's own
log file was moved aside first, so each arm started from an empty file and
nothing from the sweep entered the archive.

| Level | Bytes in 180 s | Per hour | × Normal | Events | Process disk writes |
|---|---:|---:|---:|---:|---:|
| **Off** | **0** | **0** | — | 0 | (see §3) |
| Normal (`info`) | 86,127 | ~1.7 MB | 1× | 217 | 692 KB |
| Detailed (`debug`) | 474,161 | ~9.5 MB | **5.5×** | 1,785 | 1.46 MB |
| Everything (`trace`) | 1,768,899 | ~35 MB | **20.5×** | 7,538 | 2.53 MB |

A day left at `trace` would be roughly 850 MB. That is the honest reason the
picker's help text calls it a bug-chasing setting rather than a preference.

Where the extra volume comes from (events per 180 s):

| Level | Loudest sources |
|---|---|
| `info` | `ui` 169, `client_core` 18, `wgpu_hal` 14 — plus 1,503 continuation lines, i.e. 87% of the bytes are the startup struct dumps |
| `debug` | `sync_once` 214, `iced_wgpu` 355, `build` 158, `next_sync_with_lock` 143 — sliding sync and the wgpu setup |
| `trace` | `Connection` 3,185 (508 KB), `live_update_handler` 615, `sync_once` 498 — almost entirely matrix-sdk internals |

At `info` the app's own logging is small; everything above it is the SDK.

---

## 3. "Off" means off

Verified on the real profile: level set to `off`, full launch, 60 s of live
sync, clean shutdown. Every file in the log directory was byte-identical
before and after, and no new file was created — the appender is never built,
so the daily file for today is not even opened.

What still runs at `off` is the render watchdog, and that is deliberate. The
obvious implementation — an `EnvFilter` of `off` — would have disabled it
silently: `Layered::pick_level_hint` takes `cmp::max` of the layers' hints, an
`off` filter pulls the process-wide max level to `OFF`, every callsite is then
disabled statically, and the `iced_winit` present failures the watchdog counts
never reach it. Turning logs off would have turned off recovery from the
standby hang in §1 of the archive findings. Instead `off` installs the watchdog
alone, and the watchdog declares `max_level_hint() == ERROR` so its callsites
stay live. Covered by `render_watchdog::tests::the_layer_counts_with_logging_turned_off`.

---

## 4. What the rate limiter does to a flood

Measured through the exact subscriber stack `logging::init` builds (limiter →
`info` filter → non-ANSI fmt layer), release build, 10 seconds of a callsite
flooding as fast as the machine allows:

| | |
|---|---:|
| Events emitted | 203,838,000 (20.4 M/s) |
| Lines actually written | 260 |
| Bytes actually written | 26,780 |
| Unlimited equivalent (83 B/line, the archive's measured line size) | 16.9 GB |
| Ratio | **~630,000×** |

Sustained bound: 250-line burst plus 1 line/s ≈ **3,850 lines ≈ 390 KB per
hour**, whatever happens. Against the archive: 2026-08-20 wrote 17 GB in a
10.2 h flood; the same event today costs about 3 MB. The 2026-08-29 day
(3.90 GB) would be about 600 KB.

---

## 5. Against Cinny

Both clients logged into the same account, both idle, sampled over the same
10 minutes (19:33–19:43) with per-process I/O counters, WebView2 children
attributed to whichever app spawned them.

| | ThornyChat | Cinny |
|---|---:|---:|
| Disk writes in 10 min | **0.54 MB** | 4.76 MB |
| Write operations | 244 | 4,867 |
| Rate | 0.9 KB/s | 7.7 KB/s |
| Log growth | 1,968 B | **0** |
| App data growth | 76 KB | 1.65 MB |

Steady state, ThornyChat writes ~9× less than Cinny and issues 20× fewer write
operations — Cinny's cost is IndexedDB and LevelDB compaction, which run
whether or not anything happened.

On-disk footprint tells the opposite story about *logs specifically*:

| | ThornyChat | Cinny |
|---|---:|---:|
| Matrix state | 18 MB (sqlite) | 4.5 MB (IndexedDB) |
| Caches | 540 MB media + 45 MB webview | 510 MB WebView2 code cache + 25 MB HTTP cache |
| **Logs on disk** | **10,385 MB** | **0 MB** |

Cinny writes no application log at all — the only `LOG` files under its data
directory are 0-byte LevelDB stubs, and its diagnostics live in the devtools
console, which is memory-only and gone when the window closes. That is one
sane end of the design space: nothing to configure, nothing on disk, and
nothing to send anyone when something goes wrong. ThornyChat sat at the other
end — verbose, permanent, unbounded, no switch.

The settings added here put the choice in the middle: `info` by default with a
flood ceiling, `off` for people who want Cinny's answer, and `debug`/`trace`
for the day something needs chasing — plus "Copy log to clipboard" and "Delete
log files", which are the two things the console-only approach cannot offer.

---

## Method notes

- Per-level arms: 180 s each, `CloseMainWindow` shutdown, size read through an
  open handle (a directory listing can lag behind a file being appended to).
- The flood figure is a direct measurement of bytes reaching the writer, not a
  simulation of the bucket arithmetic; the 83 B/line unlimited baseline is the
  archive's own measured average across the six flood days.
- The Cinny comparison is idle-vs-idle. Neither client was typed into during
  the window, and the same account was signed in on both.
- One thing not measured: how much a `debug`/`trace` day *actually* costs over
  a full day of real use. The 180 s arms include a launch and a first sync,
  which are the busiest part of any session, so the per-hour figures above are
  upper bounds rather than averages.
