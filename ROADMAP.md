# ThornyChat (Matrix client, Rust + iced) — Remaining Work

Windows-first Matrix client at `C:\Users\Office\thornychat`. Workspace: `client-core`
(matrix-sdk 0.18 wrapper, no iced), `ui` (iced 0.14 views, no matrix-sdk types),
`app` (binary), `xtask` (release builds). Tested against a real account on a
private Synapse homeserver (SSO login). Release builds: `cargo xtask` → generic +
znver4 + znver5 `thornychat.exe` variants, each in its own `target/`
subdirectory (a bare `cargo build --release` yields only the generic one —
see README "Building").

## Done (core client)

- Auth & session: password + browser-SSO login w/ server discovery; session
  restore via Windows Credential Manager; logout w/ confirm (Settings →
  General); sliding-sync worker bridged to iced, timeline updates streamed
  as incremental diffs (unit-tested) w/ media placeholders coalesced to keep
  startup reflow down.
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
  `/kick`, `/ban`; `//` escapes) + in-app manual, right-click cut/copy/paste
  menu, Windows ISpellChecker suggestion bar w/ opt-in autocorrect.
- E2EE: cross-signing bootstrap w/ UIAA fallback, SAS verify, opt-in key
  backup/recovery (Settings → Security), trust shields.
- Media & rich content: reactions (no-bg pills, hover attribution, full
  picker; custom-emoji reactions keyed by mxc URL); MSC2545 emoji packs w/
  animated GIF emotes (vendored `animated_image` widget); Twemoji, light
  skin-tone default, persisted frequently-used history; URL previews via
  homeserver OG proxy (privacy-gated) + FxTwitter tweet cards; inline video
  for YouTube/Vimeo/Dailymotion/Rumble/Kick (live channels only, no VOD/clip
  embed) and direct video files, hosted in a WebView2 child window; image
  lightbox w/ cursor-anchored zoom, Lanczos3 upscale past ~300% native
  (Real-ESRGAN evaluated and rejected — hallucinates detail), save-to-disk;
  file messages click-to-save w/ the real filename suggested; media/emoji
  disk caches capped (512 MB/64 MB, oldest evicted at startup, unit-tested).
- Settings & platform: per-room notification modes + account-wide DM/group
  defaults (synced both ways); member panel grouped by MSC3949 power tags w/
  click-to-DM, visibility persisted; theming engine (custom themes,
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
  invite/kick/ban currently have no buttons, only slash commands.
- Room creation wizard; invite accept/reject with room preview.

## Phase 7 — Windows platform polish & packaging

Done: autostart (HKCU Run + `--minimized`, toggled from Settings); icon +
version resource embedded via `app.rc`/`embed-resource`; window
size/position/maximized remembered across launches. (System accent color
via `UISettings` was considered and dropped — the theming engine's custom
accents cover it.)

Remaining:
- Push-rule evaluation → `ClientEvent::Notification` (client-core `push.rs`
  still only reads/writes notification *settings*; the Notification event
  is never emitted).
- WinRT toast notifications (actionable, inline reply) —
  `platform/notifications.rs` is a stub; needs AUMID/package identity —
  validate early, affects packaging.
- Tray icon w/ unread badge, minimize-to-tray, single-instance enforcement
  (`platform/tray.rs` is a stub).
- MSIX packaging (primary) + NSIS/WiX installer fallback.

## Backlog / known gaps (roughly by value)

- Threads: only reply-count badges; no thread panel view.
- Encrypted-room media: images/files/stickers degrade to text placeholder
  (`MediaSource::Encrypted` unsupported in the media cache path).
- HTML `formatted_body` not rendered (plain body only): no mention pills,
  colored text, spoilers, code blocks from other clients.
- Polls render as placeholders.
- Server-side `/search` (local filter + user-directory DM search only).
- Animated WebP/APNG emotes render as stills (`animated_image` is GIF-only).
- Timeline virtualization (diffs stream now, but every row still renders;
  watch big rooms + many GIFs).
- Round avatar clipping (iced can't clip images; would need CPU pre-rounding).
- Jump-to-quote scroll is index-estimated, not pixel-exact.
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
- Server-authored strings must render through `theme::remote_text` (Advanced
  shaping + bundled Noto CJK fallback) — iced's default Basic shaping never
  falls back, so plain `text()` on remote content shows tofu for CJK.
- iced gotchas learned: widget state is positional (use `theme::slot` for
  conditional elements near inputs), no `Length::Fill` inside vertical
  scrollables, overlays via always-present `stack!`, container::visible_bounds
  for viewport probing, `anchor_bottom()` for chat scroll semantics;
  `Task::perform`'s mapper is `FnOnce` since iced 0.14.
- Native child windows (WebView2/wry, see `ui/src/video_player.rs`): the
  webview is not `Send` and needs the thread that pumps the parent HWND's
  messages — do ALL create/set_bounds/close inside
  `iced::window::run_with_handle` closures (they run on the winit event-loop
  thread) against a thread_local. A child HWND always composites above the
  wgpu surface, so overlays must be screen-fixed (lightbox), not scrolling
  inline; `video_rect()` is the single geometry source for both the iced
  frame and the native bounds.
