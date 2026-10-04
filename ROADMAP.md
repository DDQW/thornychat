# ThornyChat (Matrix client, Rust + Tauri) — Remaining Work

Windows-first Matrix client. Workspace: `client-core` (matrix-sdk 0.19 wrapper,
no UI), `desktop` (the Tauri 2 shell: bridge, media protocol, settings,
connectors), `frontend` (Svelte 5 + TypeScript UI) and `xtask` (release
builds). Tested against a real account on a private Synapse homeserver (SSO
login). Release builds: `cargo xtask` → frontend build, then generic + znver4 +
znver5 `thornychat.exe` variants, each in its own `target/` subdirectory (see
README "Building").

## Migration from iced (2026-10)

The UI moved from iced to Tauri + Svelte (plan and rationale:
`docs/tauri-migration.md`; the last iced build is the `iced-final` tag). Everything
under "Done" below was ported, with these differences and open items.

**Verified for real** (the built exe, driven over the DevTools protocol): startup
and the login screen, settings carried over from the existing config files, the IPC
commands, the `tcmedia` media routes and their refusals, the CSP, navigation
blocking, homeserver discovery against a live server.

**Not yet verified, because it needs a signed-in account** — treat these as the
first things to try on a real session: sync and the room list, the timeline against
real events (the UI was exercised against a mock worker), Matrix media over
`tcmedia`, attachment upload, SSO, key backup / verification. (The YouTube and
Vimeo embeds were checked in the built exe on 2026-10-03, by loading the same
iframe on the login page; Dailymotion's player answered 403 from this network,
in a plain browser too.)

**Behaviour that changed**
- Spelling is the Windows speller again (`ISpellChecker` on its own STA thread,
  `crates/desktop/src/spellcheck.rs`; marking, suggestion bar, autocorrect and its
  Backspace undo in `frontend/src/lib/spell.ts`). WebView2's own checker was tried
  first and marks nothing in this app (checked on the built exe, 2026-10-03), so the
  native attribute is only a fallback for when the speller is unavailable. New:
  typos are underlined (the iced build recoloured them), and right-clicking a marked
  word offers its suggestions. Autocorrect is a toggle again (Settings → General).
- The lightbox sizes the picture by layout and labels zoom in actual pixels (100% =
  one image pixel per screen pixel); the old transform-scaled image stayed rastered
  at its fitted size, so zooming in showed no more detail than the fit view. Past
  150% it swaps in a Lanczos3 copy made by the media protocol (`?upscale=`, cached
  in `upscale-cache`, 256 MB cap). Pan stops at the image edge, zoom-out at the fit;
  double-click toggles fit/actual pixels; keys `+` `-` `0` `1`.
- Window geometry now comes from `tauri-plugin-window-state`; the old
  `window.json` (incl. its GPU preferences, which no longer apply) is ignored.
- `cargo xtask install-dev` / `toast-test` are gone: the NSIS installer
  (`cargo xtask installer`) writes the Start Menu shortcut Windows needs.
- The font family and UI scale apply live now, not on next launch.

**Dropped as unnecessary** (they only existed to work around iced/wgpu): the render
watchdog, the standby window-close dance, the synthetic-input shim, the GIF decoder,
the WebView2 child-window video player, the Win32 clipboard probe.

## Done (core client)

- Auth & session: password + browser-SSO login w/ server discovery; session
  restore via Windows Credential Manager; logout w/ confirm (Settings →
  General); sliding-sync worker bridged to the page over a Tauri channel,
  timeline updates streamed as incremental diffs (unit-tested on both sides).
- Rooms: room list (DM/room sections, filter, unread badges, computed display
  names, avatars); spaces sidebar section w/ joined rooms nested under their
  parent space; space-explorer overlay (hierarchy API, drill-down w/ back
  stack, join w/ via servers, knock w/ Requested state on knock-rule rooms);
  leave/forget; user-directory DM search; local message/room search.
- Timeline: pagination + scroll autoload, bottom anchoring, day/new-message
  dividers, IRC-style read receipts (marked read only while scrolled to the
  newest message, focus-independent), message grouping, hover action bar,
  jump-to-latest, membership events (toggleable), hash-palette name colors,
  timestamps, "(edited)" tags on edited messages.
- Composer: markdown + preview, @mention pills, attachments + paste-to-attach
  chips (typed text becomes the MSC2530 caption), drag-and-drop, edit/redact,
  typing, reply/quote w/ jump-to-quoted + thumbnails, send retry, slash
  commands (`/me`, `/plain`, `/join`, `/knock`, `/leave`|`/part`, `/invite`,
  `/kick`, `/ban`, `/ignore`|`/unignore`; `//` escapes) + in-app manual,
  right-click cut/copy/paste menu, Windows ISpellChecker typo marking
  in-composer (wavy underline) + suggestion bar + right-click suggestions w/
  opt-in autocorrect (Backspace undoes; autocorrect corpus guarded by a test).
- Big rooms: the members panel is windowed (20 000 members: 2.4 s → ~25 ms to
  open), rosters of closed rooms are released, unchanged timeline rows and room
  summaries keep their objects so one event re-renders one row, and `<video>`
  range requests are served in 4 MB chunks.
- E2EE: cross-signing bootstrap w/ UIAA fallback, SAS verify, opt-in key
  backup/recovery (Settings → Security), trust shields.
- Media & rich content: reactions (no-bg pills, hover attribution, full
  picker; custom-emoji reactions keyed by mxc URL); MSC2545 emoji packs w/
  animated GIF emotes (vendored `animated_image` widget); Twemoji, light
  skin-tone default, persisted frequently-used history; URL previews via
  homeserver OG proxy (privacy-gated) + FxTwitter tweet cards; inline video
  for YouTube/Vimeo/Dailymotion/Rumble/Kick (live channels only, no VOD/clip
  embed) and direct video files, hosted in a WebView2 child window; image
  lightbox w/ cursor-anchored zoom, Lanczos3 upscale past 150% native
  (Real-ESRGAN evaluated and rejected — hallucinates detail), save-to-disk;
  file messages click-to-save w/ the real filename suggested; media/emoji
  disk caches capped (512 MB/64 MB, oldest evicted at startup, unit-tested).
- Settings & platform: per-room notification modes + account-wide DM/group
  defaults (synced both ways); member panel grouped by MSC3949 power tags w/
  click-to-DM and a right-click menu (DM, new room, highlight, ignore),
  visibility persisted; account-wide ignore list (`m.ignored_user_list`,
  watched so other devices' changes land too); theming engine (custom themes,
  dark/light); persisted config files
  (theme/chat/privacy/spellcheck/encryption/connectors/window); window
  size/position/maximized remembered across launches (debounced save,
  off-screen restore guard); autostart (HKCU Run + `--minimized`); app icon
  + version resource embedded; game-activity connectors (Steam/GOG/Epic →
  `m.emote` when the running game changes).
- Repo: public, GPL-3.0-or-later, README + CONTRIBUTING; `cargo xtask`
  release pipeline; GitHub Actions CI on Windows (check + clippy w/
  `-D warnings` + tests).

## Phase 5 — Calls (native WebRTC / MatrixRTC) [highest risk]

Signaling shipped (first pass, still untested against a live call):
- Validation result: matrix-sdk has full MatrixRTC *signaling* (MSC3401
  member events in sliding sync's default `required_state` — unlike other
  custom state — plus `RoomInfo` call tracking, MSC4140 delayed events,
  MSC4075 notify) but **zero media**. Modern calls run through a LiveKit
  focus (SFU), so native media needs a LiveKit protocol client — the raw
  `webrtc` crate alone can't join one. Scoped to signaling-only as planned.
- Done: `client-core/calls` (`CallManager`): live per-room call state via a
  `m.call.member` event handler + startup sweep + on-open snapshot; join
  publishes a session membership (reuses existing call_id/foci, else
  `.well-known` `org.matrix.msc4143.rtc_foci`), with an MSC4140 delayed
  leave scheduled *before* joining + 4s heartbeats (crash cleanup; graceful
  fallback if the HS lacks it, e.g. `M_UNRECOGNIZED`); leave sends the empty
  membership + cancels the delayed one; `m.call.notify` ring/notify when
  starting a fresh call; leave-all on logout/shutdown. UI: accent banner
  under the room header (roster faces w/ tooltips, distinct-user count,
  Join/Leave w/ pending+error states, "signaling only" honesty label),
  header Start-call button when no call, green 📞 in the sidebar row.

Remaining (media):
- Exercise the shipped signaling against a live call from another client.
- Native media = LiveKit client over the `webrtc` crate (JWT from the
  focus' `livekit_service_url`, websocket signaling, SFU tracks): audio
  first, then video — `calls/webrtc_session.rs` is the seam.
- Device pickers (mic/cam), mute/deafen (deliberately not stubbed in the
  UI while they'd do nothing).

## Phase 6 — Admin, spaces, room management

Done: space explorer + sidebar nesting (via `space_children`); join by typed
id/alias, invite, kick, and ban all work as slash commands; leave/forget;
knock flow (explorer "Request to join" button + `/knock`).

Remaining:
- Room settings dialog: name/topic/avatar, join rules, history visibility,
  encryption toggle (`settings/room_admin.rs` is a placeholder stub).
- Member management UI: power-level editor (incl. writing MSC3949 tags),
  per-member profile popover (avatar, id, PL) instead of click=DM only —
  invite/kick/ban still have no buttons, only slash commands. (Ignore is
  the exception: it has a roster right-click entry, since unlike the rest
  it needs no power level.)
- Room creation wizard; invite accept/reject with room preview.

## Phase 7 — Windows platform polish & packaging

Done: autostart (HKCU Run + `--minimized`, toggled from Settings); app icon and
version resource (Tauri's bundler); window size/position/maximized remembered
across launches; tray icon (click to raise the window, Show/Quit menu; closing the
window still quits); single instance per profile (a second launch of the default
profile raises the first; other profiles run side by side); a per-user NSIS
installer (`cargo xtask installer`) whose Start Menu shortcut carries the app
identity `Woelki.ThornyChat` that Windows uses to attribute toasts; message toasts
from the account's push rules (matrix-sdk evaluates them during sync,
`client-core/src/notifications.rs`; room modes and keywords apply, history and
your own messages don't toast, nothing toasts while the window has focus), and
clicking one raises the window and opens the room (`desktop/src/toast.rs`). Exit
now gives the sync worker two seconds to leave any call it is in before the
process goes.

Remaining:
- Toasts are untested against a live account (no way to produce a push-rule
  hit without one). A click from the notification centre after the popup has
  gone relies on Windows relaunching the app through its shortcut (the single
  instance then raises the window, without opening the room): a COM toast
  activator would make that path open the room too.
- Toast actions (buttons, inline reply).
- Tray unread badge.
- Updater: no decision yet (`tauri-plugin-updater` needs signing keys).
- Code signing: the installer is unsigned, so SmartScreen will warn.

## Backlog / known gaps (roughly by value)

- Threads: only reply-count badges; no thread panel view.
- Encrypted-room media: images/files/stickers degrade to text placeholder
  (`MediaSource::Encrypted` unsupported in the media cache path).
- Polls render as placeholders.
- Server-side `/search` (local filter + user-directory DM search only).
- Timeline virtualization: the live list is capped at 200 items (the window is
  reopened when it grows past that) but every loaded row is a real DOM node.
  If big rooms with many animated images get heavy, try `virtua` (the plan's
  first candidate) before anything bespoke.
- Connectors: "now playing" media source (Windows media-transport API) to
  sit beside game detection.
- Repo hygiene: CI runs check/clippy/test, but the planned wiremock-based
  client-core suite and update() logic tests don't exist yet.

## Development notes

- rustup toolchain, target `x86_64-pc-windows-msvc`; build logs pattern:
  `cargo build --release 2>&1 | Out-File $env:TEMP\thornychat_build.log`.
- App data: `%APPDATA%\ThornyChat\ThornyChat\data\<profile>\` (store, logs,
  emoji-cache incl. usage.json); global prefs under `...\config\*.json`.
  Pre-rename installs are migrated from `%APPDATA%\Synapse\Synapse` on first
  launch (see `client-core/src/store.rs`).
- Debug an issue: `$env:RUST_LOG="info,client_core=debug"` then read
  `...\data\default\logs\thornychat.log.<date>`.
- Custom state events (emoji packs, power tags) must be fetched via
  `client.send(get_state_events_for_key)` — sliding sync's required_state
  never includes them; same pattern for any future MSC state.
- UI work without a homeserver: `npm --prefix frontend run dev` and open
  http://localhost:1420 — outside Tauri the page runs against `frontend/src/lib/mock.ts`.
  A real-app check that needs no account: build with
  `cargo build -p thornychat --features custom-protocol`, start the exe with a
  throwaway profile name and `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333`,
  then drive it over the DevTools protocol.
- Server-authored strings are untrusted markup: they reach the page only as
  escaped text or through `sanitizeFormattedBody` (Rust sanitizes first, the page
  again), and the CSP forbids remote images, connections and inline script. Keep
  it that way — the webview has IPC access.
- A mock-driven pane that isn't being painted delivers no `ResizeObserver`
  callbacks or scroll events, so layout behaviour (stick-to-bottom, history
  anchoring) can only be checked with a frame forced between steps.
- The timeline turns browser scroll anchoring off on purpose (`overflow-anchor:
  none`): it does nothing at scrollTop 0, which is exactly where someone who just
  hit the top of history is. Prepending compensates for the added height itself.
