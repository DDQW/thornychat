# Log archive findings — 2026-07-14 → 2026-09-03

Everything worth keeping from the 39 GB of daily logs under
`%APPDATA%\ThornyChat\ThornyChat\data\default\logs`. **Once this document is
reviewed, those files can be deleted** — nothing else in them is of value, and
every finding below is reproduced here with its evidence.

Method: all 34 daily files scanned end to end; every `WARN`/`ERROR` line
extracted (32,576 of them), the render-hang flood counted separately, and the
whole archive searched for panics, aborts and assertion failures.

## Headline numbers

| | |
|---|---|
| Files / span | 34 daily logs, 2026-07-14 → 2026-09-03 |
| Total size | 39 GB |
| Render-hang flood lines | **498,901,936** — 99.99% of the archive |
| All other WARN/ERROR lines | 32,576 |
| **Rust panics / aborts / assertion failures** | **0** |

The app has never once crashed. Every problem below is either a hang, a
transient network failure, or a cosmetic warning.

---

## 1. Standby render hang — 15 occurrences, never self-recovers

Full analysis and the upstream issue draft are in
[`iced-surface-error-other-hang.md`](iced-surface-error-other-hang.md). What the
archive adds:

| Day | Flood lines | Window | Duration | Rate |
|---|---:|---|---|---:|
| 2026-08-17 | 16,342,574 | 14:57–18:21 | 3.4 h | 1,331/s |
| 2026-08-18 | 1 | 20:41 | — | — |
| 2026-08-19 | 53,938,125 | 11:12–21:24 | 10.2 h | 1,468/s |
| 2026-08-20 | 209,587,843 | 09:33–19:45 | 10.2 h | 5,707/s |
| 2026-08-21 | 61,119,160 | 09:50–23:56 | 14.1 h | 1,203/s |
| 2026-08-22 | 26,785,800 | 00:21–23:06 | 22.7 h | 327/s |
| 2026-08-24 | 16,334,861 | 14:21–22:07 | 7.8 h | 583/s |
| 2026-08-25 | 5,944,174 | 08:21–19:52 | 11.5 h | 143/s |
| 2026-08-27 | 24,174 | 22:30–22:31 | 2 s | 12,087/s |
| 2026-08-28 | 26,639,986 | 07:34–19:23 | 11.8 h | 626/s |
| 2026-08-29 | 46,951,786 | 11:21–13:15 | 1.9 h | 6,858/s |
| 2026-08-30 | 23,356,713 | 14:36–15:11 | 35 min | 10,950/s |
| 2026-08-31 | 2,010 | 15:53 | <1 s | — |
| 2026-09-01 | 11,345,524 | 07:43–07:53 | 9 min | 20,116/s |
| 2026-09-03 | 529,205 | 13:47–16:14 | 2.5 h | 59/s |

Three things the raw error message doesn't tell you:

**It never recovers on its own.** In all 15 events the flood runs until the
process ends. Where the log continues afterwards it resumes with
`INFO thornychat: starting ThornyChat` — a restart, e.g. 2026-09-01 floods
07:43→07:53 and the next line is a fresh start at 13:30. There is no case of a
present succeeding again.

**The app is not fully dead — only the renderer is.** Other subsystems keep
running throughout. During the 2026-08-17 hang the timeline was still
processing sync:

```
18:05:02.476166 ERROR iced_winit: Error Other when presenting surface.
18:05:02.476624  INFO ui::update: timeline window reset by a sync gap — snapping to live edge len=19
18:05:02.478080 ERROR wgpu::backend::wgpu_core: Handling wgpu errors as fatal by default
18:05:02.483049 ERROR iced_winit: Error Other when presenting surface.
```

So it is a pure render/present failure: state advances, nothing is ever drawn.

**Rate varies 100×** (59/s to 20,116/s) depending on what else contends for the
loop — which is why one day cost 17 GB and another 44 MB.

Onset always follows a gap in logging consistent with standby (2026-08-17: last
line 14:18, flood at 14:57; 2026-08-27: last line 22:08, flood at 22:30), or is
the first line of a new day's file after an overnight suspend (2026-08-19).

`ERROR wgpu::backend::wgpu_core: Handling wgpu errors as fatal by default`
appears 15 times, always *inside* a flood window, never before one — it is a
consequence, not the trigger.

**Status:** upstream bug, no fix available (0.14.0 is the newest iced), so the
app now handles it itself:

- `crates/ui/src/watchdog.rs` — a zero-size widget counts redraw arrivals. Held
  above 150/s for 10 s, it notifies via the tray, relaunches, and exits. It
  publishes through the event loop's own message vec rather than a subscription,
  because the subscription channel is demonstrably full during a hang.
- `crates/ui/src/platform/relaunch.rs` — the restart, with a
  `--restarted-after-hang` guard so a still-wedged GPU can't produce an endless
  respawn.
- `crates/app/src/log_limit.rs` + retention cap — the same flood now costs
  ~36k lines instead of 209M, whether or not the watchdog catches it.

Still open: the threshold may miss the slowest observed hangs (see the rate
column). The watchdog logs a calibration sample whenever the rate exceeds 60/s,
so the next real occurrence will supply a measured number.

---

## 2. Session and crypto store destroyed by a *network* error at startup

**This is a real bug in our code, and it cost a re-login on 2026-08-24.**

```
2026-08-24T10:11:20.672949Z  WARN client_core::session: state store unusable,
    falling back to interactive login error=client build error: error sending request
```

[`session.rs:164`](../crates/client-core/src/session.rs:164) asserts that past
that point a failure can only be a store problem:

> From here on, failures are store problems (meta.homeserver is a full URL, so
> `build_client` does no network discovery; `restore_session` is local store
> activation)

The log disproves it: the error is `error sending request` — a network failure.
`Client::builder().build()` does reach the network even when given a full
homeserver URL. On that path the code then runs:

```rust
let _ = std::fs::remove_file(&meta_path);
discard_state_store(paths);
return Ok(None);          // → interactive login
```

So **a transient network failure at launch deletes `session.json` and the whole
sqlite state store**, including the crypto store bound to that device — losing
the session, the local cache, and that device's E2EE identity. Launching before
the network is up (autostart at boot, or right after a resume) is exactly when
this fires.

**Fixed.** `CoreError::is_unusable_store` (`crates/client-core/src/error.rs`) is
a strict whitelist — only `ClientBuildError::SqliteStore` may discard anything.
Every other build failure now keeps the session and returns an error, and
`CoreError::is_transient_transport` marks the network cases so `update.rs`
retries with backoff (2s → 30s, six attempts) behind a "Can't reach the
homeserver — retrying…" status instead of dumping the user on a login form.
`restore_session` gets the mirror-image treatment: it still self-heals on local
faults, but never on transport.

A companion warning (`failed to remove stale state store`) fired the same day,
so the deletion didn't even fully succeed.

---

## 3. Layout-invalidation thrash — July only, unexplained, possibly latent

3,203 × `WARN iced_winit: More than 3 consecutive RedrawRequested events
produced layout invalidation`, meaning the view kept resizing itself every
frame.

| Day | Count |
|---|---:|
| 2026-07-23 | 1 |
| 2026-07-24 | 2 |
| 2026-07-25 | 2,072 |
| 2026-07-30 | 47 |
| 2026-07-31 | 1,081 |

Stops dead after 2026-07-31 and never appears in August or September. No commit
lands in that window (the last commit is 2026-07-24), so this was **not fixed by
a code change** — it stopped on its own, which means it is probably
content- or room-dependent and still latent. If the composer or timeline ever
starts feeling sluggish, this warning is the thing to grep for.

Left alone deliberately: there is nothing to fix without a reproduction. The
rate limiter keeps it visible if it returns without letting it flood.

---

## 4. Dropped UI events during hangs (symptom, not a cause)

`WARN iced_futures::subscription::tracker: Error sending event to subscription:
TrySendError { kind: Full }` — iced's bounded subscription channel overflowing,
so events were discarded.

Every occurrence falls inside a render-hang window (e.g. the 2026-08-19 cluster
at 19:42–20:22 sits inside that day's 11:12–21:24 flood). The render loop starves
the update loop, the channel backs up, events are dropped. Expected to disappear
entirely once the hang is fixed; not worth chasing separately.

---

## 5. Network outages — handled correctly

7,552 × `dns error` (`Os { code: 11001 }` = `WSAHOST_NOT_FOUND`), surfacing as
`ERROR supervisor …`, `matrix_sdk_ui::sync_service: Error while processing
encryption/room list in sync service`, and failed read-receipt sends.

Worst day 2026-08-17 with 4,158, arriving at a steady 7–8 second interval
(~8/min) for roughly 8.5 hours — a long DNS outage, not a retry storm. The sync
supervisor reconnected every time; **no session was lost and no messages were
missed as a result.**

One observation: that 7–8 s interval is flat, so the SDK's offline-mode retry
does not back off during a prolonged outage. It is polite enough not to matter,
but it does mean a multi-hour outage writes thousands of ERROR lines.

Related, all transient and all recovered:

| Count | What |
|---:|---|
| 21 | `client_core::sync: failed to send read receipt` / `fully-read marker` |
| 17 | `mark_as_read … send_single_receipt` failures |
| 5 | `send_queue: Recoverable error when sending request` |
| 5 | `supervisor task: unable to stop room list service: SlidingSync's internal channel is blocked` |

---

## 6. Reply previews that silently fail — 74 occurrences

`ERROR fetch_details_for_event{…}:fetch_in_reply_to_details{…}:send{… status=404}`
— the homeserver 404s on
`GET /_matrix/client/v3/rooms/{room}/event/{event}` for the quoted event.

Spread over 2026-07-31 → 2026-08-17, all in one room. The quoted events are
redacted or purged server-side, so the reply preview can never load.

**Fixed.** `convert_reply_preview` collapsed every non-`Ready` state to `"…"`,
making a permanent failure look identical to a fetch still in flight;
`TimelineDetails::Error` now renders "Message unavailable".

---

## 7. Decryption retries — 51 occurrences

`matrix_sdk::event_cache::redecryptor: Failed to redecrypt an event` plus the
`decrypt_room_event` warnings behind them, across the
`retry_decryption_for_event_cache_updates` and `retry_decryption` paths. These
are the SDK retrying events whose keys hadn't arrived yet; expected traffic for
an encrypted room, and low volume.

---

## 8. Cosmetic / by design

| Count | What | Verdict |
|---:|---|---|
| 355 | `matrix_sdk_ui::timeline::tasks: No avatar changes to update` | SDK noise — now filtered to `error` in `logging.rs` |
| 64 | `ui::animated_image: widget slot reused for a different gif — resetting` | handled; expected on fast scroll |
| 3 | `ui::update: media is AVIF/HEIC — no decoder compiled in, falling back` | by design (codecs trimmed in `5397911`) |
| 1 | `ui::update: media fetch failed, blacklisting for this session` | handled |

---

## What is *not* in the archive

Worth stating explicitly, since it bounds what these logs could still be useful for:

- no panics, aborts, or assertion failures
- no out-of-memory or allocation failures
- no authentication rejections (`M_UNKNOWN_TOKEN`, 401, `M_FORBIDDEN`) — the
  only session loss was the self-inflicted one in §2
- no store corruption or `MismatchedAccount`
- no evidence of lost or duplicated messages

## What was done about it

| § | Finding | Outcome |
|---|---|---|
| 1 | Standby render hang | Watchdog + guarded auto-restart; flood bounded by the rate limiter. Upstream issue drafted. |
| 2 | Session/crypto store destroyed by a network error | Fixed — only a sqlite fault may discard; transport errors retry with backoff. |
| 3 | Layout-invalidation thrash | Deferred, no reproduction. Kept visible, no longer able to flood. |
| 4 | Dropped UI events | No action — a symptom of §1. |
| 5 | DNS outages | No action — handled correctly already. |
| 6 | Reply previews that 404 | Fixed — now says "Message unavailable". |
| 7 | Decryption retries | No action — normal for an encrypted room. |
| 8 | Cosmetic noise | SDK avatar spam filtered out. |

Everything above is reproduced in this document, so **the log files themselves
are no longer needed**.
