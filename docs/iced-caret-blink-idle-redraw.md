# Upstream report: the caret blink repaints the whole window forever at 2 Hz

Draft of an issue to file against [iced-rs/iced](https://github.com/iced-rs/iced/issues).
Written up while auditing ThornyChat's idle power draw; nothing in this repo
works around it. See the measured numbers at the bottom for why we chose not to
vendor a patched widget over it.

---

## Title

A focused `text_input` keeps the application at a permanent 2 fps, even when
completely idle — no way to opt out

## Summary

`text_input` re-arms a redraw every 500 ms for as long as it is focused, to
blink its caret:

```rust
// iced_widget-0.14.2/src/text_input.rs:1342
let millis_until_redraw = CURSOR_BLINK_INTERVAL_MILLIS
    - (*now - focus.updated_at).as_millis() % CURSOR_BLINK_INTERVAL_MILLIS;

shell.request_redraw_at(
    *now + Duration::from_millis(millis_until_redraw as u64),
);
```

`CURSOR_BLINK_INTERVAL_MILLIS` is `500` and private, and the re-arm is
unconditional while `focus.is_window_focused`. There is no builder method, no
style field, and no runtime setting that turns it off.

Because iced has no partial redraw, "blink the caret" is spelled "rebuild the
entire widget tree, lay it out, encode a frame, and present it". For an app
whose composer is focused by design — a chat client, an editor, a search-first
tool — that is the app's permanent floor: **it never idles below 2 fps as long
as the user is looking at it.**

The rest of iced is careful about this. `ControlFlow::Wait` is the default,
`unconditional-rendering` is opt-in, and every other redraw in the tree is
event-driven. A blinking caret is the one thing that keeps the loop warm.

## Why this matters

- **Laptops.** A 2 Hz wakeup is worse than its CPU-time number suggests: it is
  frequent enough to keep the package out of deep C-states and to keep the GPU
  from dropping to its lowest clocks, but does no useful work between ticks.
- **Hybrid graphics.** Combined with `iced_wgpu`'s hardcoded
  `PowerPreference::HighPerformance`
  (`iced_wgpu-0.14.0/src/window/compositor.rs:85`), an idle iced app can hold a
  discrete GPU awake and repaint it twice a second, indefinitely.
- **Big trees.** The cost is proportional to the whole view, not to the caret.
  An app with an expensive `view()` pays that price 172,800 times a day.

## Environment

| | |
|---|---|
| iced | 0.14.0 (`iced_widget` 0.14.2) |
| wgpu | 27.0.1 |
| winit | 0.30.13 |
| OS | Windows 11 IoT Enterprise LTSC 2024 (10.0.26300) |
| Backend | Vulkan |
| GPU | AMD Radeon RX 7900 XTX |

## Steps to reproduce

1. Run any iced app containing a `text_input`.
2. Focus the input. Do not type.
3. Observe a redraw every 500 ms, forever — e.g. by counting `draw` calls, or
   with an external frame counter.

Expected: an idle application redraws zero times.
Actual: two full frames per second, indefinitely.

## Suggested fixes

In rough order of preference:

1. **Stop blinking once the input goes idle.** After N seconds without input,
   stop re-arming and draw a solid caret. This is what several editors do, it
   is arguably better UX (a steady caret is easier to locate than a blinking
   one), and it takes an idle app to zero frames. It needs no API change.
2. **Honor the platform's caret-blink setting.** Windows exposes
   `GetCaretBlinkTime()`, which returns `INFINITE` when the user has turned
   blinking off in accessibility settings — a preference iced currently
   ignores. Respecting it fixes this for those users and is correct anyway.
3. **Expose it.** A `text_input::Caret` style/behavior knob, or a
   `blink(bool)` builder method, so applications that care can opt out.

(1) and (2) compose well: honor the OS setting, and idle out regardless.

## What we measured

For calibration, on the app that prompted this — ~200 message widgets, a room
list, and a member roster rebuilt per frame:

| | |
|---|---|
| Process CPU, focused and idle | 0.49% of one core |
| UI-thread CPU attributable to the blink | ~1.5 ms/s (~0.15% of one core) |
| GPU engine utilization | below the Windows counter's resolution |

So on a desktop this is small, and we did not vendor a patched widget to avoid
it — 1,700 lines of maintenance for 0.15% of a core is not a trade worth
making. On battery, where the wakeup pattern matters more than the wakeup cost,
we expect it to matter considerably more; that is the case this issue is really
about.
