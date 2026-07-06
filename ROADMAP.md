# ThornyChat (Matrix client, Rust + iced) — Remaining Work

Windows-first Matrix client at `C:\Users\Office\thornychat`. Workspace: `client-core`
(matrix-sdk 0.13 wrapper, no iced), `ui` (iced 0.13 views, no matrix-sdk types),
`app` (binary). Tested against a real account on `matrix.org` (SSO login).
Release builds: `cargo build --release` → `target/x86_64-pc-windows-msvc/release/thornychat.exe`.

## Done (phases 0–4.5)
Auth (password + browser SSO w/ server discovery), session restore via Windows
Credential Manager, sliding-sync worker bridged to iced; room list (DM/room
sections, filter, unread badges, computed display names, avatars); timeline
(pagination + scroll autoload, bottom anchoring, day/new-message dividers,
IRC-style read receipts — marked read while scrolled to the newest message,
focus-independent; scrolling up holds messages unread — message grouping,
hover action bar with Segoe Fluent icons, hash-palette name colors,
timestamps); composer
(markdown + preview, @mentions, attachments, edit/redact, typing, reply/quote
with jump-to-quoted + thumbnails); E2EE (cross-signing bootstrap w/ UIAA
fallback, SAS verify, key backup/recovery, trust shields); reactions (no-bg
pills, hover attribution, overlay full picker); custom emoji (MSC2545 packs
fetched direct from server, inline `:shortcode:` rendering, animated GIF emotes
via iced_gif); Twemoji for unicode emoji, light skin tone default, per-user
frequently-used history (persisted); URL previews via homeserver OG proxy +
rich tweet cards via FxTwitter API (quoted tweets, media, engagement); in-app
video playback for YouTube/Vimeo/Dailymotion/Rumble/Kick (platform-tinted
thumbnail card w/ play badge → lightbox overlay hosting the platform's own
iframe player in a WebView2 child window via wry — no browser; "Watch on
{platform}" fallback, resize-synced bounds; Kick is live-channel-only, no
confirmed embed for a specific VOD/clip); image
zoom lightbox; notification modes per room + keyword highlights (synced both
ways); member panel grouped by MSC3949 power-level tags ("Red/Purple team",
full roster fetch) with click-to-DM; local message/room search; dark/light
toggle; custom theme + slim scrollbars.

## Phase 5 — Calls (native WebRTC / MatrixRTC) [highest risk]
Signaling shipped (first pass, untested against a live call yet):
- Validation result: matrix-sdk 0.13 has full MatrixRTC *signaling* (MSC3401
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
- Native media = LiveKit client over the `webrtc` crate (JWT from the
  focus' `livekit_service_url`, websocket signaling, SFU tracks): audio
  first, then video — `calls/webrtc_session.rs` is the seam.
- Device pickers (mic/cam), mute/deafen (deliberately not stubbed in the
  UI while they'd do nothing).

## Phase 6 — Admin, spaces, room management
- Spaces — explorer done: sidebar "Spaces" section (spaces no longer pose as
  plain rooms); clicking a space opens the explore overlay (hierarchy API via
  `rooms/spaces.rs`, depth-1 pages + load-more, drill into subspaces w/ back
  stack, Join with via servers — `JoinRoom` command now implemented — open
  joined rooms, join-rule labels for knock/invite-only). Remaining: nest
  joined rooms under their parent space in the sidebar
  (`RoomSummary.parent_space_id` still never populated); knock flow (knock
  rooms are listed as "By request", no button).
- Room settings dialog: name/topic/avatar, join rules, history visibility,
  encryption toggle.
- Member management: invite / kick / ban, power-level editor (incl. writing
  MSC3949 tags), per-member profile popover (avatar, id, PL) instead of
  click=DM only.
- Room creation wizard; invite accept/reject with room preview; join by
  *typed* id/alias (`JoinRoom` is implemented and used by the space
  explorer, but there's no free-text "join room" input anywhere).
- Leave/forget room.

## Phase 7 — Windows platform polish & packaging
- Push-rule evaluation → `ClientEvent::Notification` (client-core `push.rs`
  watcher only does settings; the Notification event is never emitted).
- WinRT toast notifications (actionable, inline reply); needs AUMID/package
  identity — validate early, affects packaging.
- Tray icon w/ unread badge, minimize-to-tray, single-instance enforcement.
- Autostart (HKCU Run + `--minimized` flag).
- System accent color via `UISettings` (theme.rs has the TODO); remember
  window size/position.
- MSIX packaging (primary) + NSIS/WiX installer fallback; embed icon/version
  resource (`embed-resource`).

## Backlog / known gaps (accumulated trim, roughly by value)
- Persist UI prefs: dark_mode resets every launch; member-panel visibility,
  keyword panel, etc. (small JSON like emoji usage.json).
- No logout button / account menu (LoggedOut event is handled, nothing sends it).
- Threads: only reply-count badges; no thread panel view.
- Encrypted-room media: images/files/stickers degrade to text placeholder
  (MediaSource::Encrypted unsupported in the media cache path).
- File messages: "[file: name]" only — no download/save/open.
- HTML `formatted_body` not rendered (plain body only): no mention pills,
  colored text, spoilers, code blocks from other clients; no "(edited)" tag.
- Polls render as placeholders.
- Server-side `/search` (local filter only today).
- Account-wide default notification mode UI (per-room + keywords exist).
- Animated WebP/APNG emotes render as stills (GIF only).
- Timeline virtualization / incremental diffs (full snapshot per update;
  fine so far, watch with big rooms + many GIFs).
- Media/emoji disk caches have no eviction.
- Round avatar clipping (iced can't clip images; would need CPU pre-rounding).
- Jump-to-quote scroll is index-estimated, not pixel-exact.
- Read receipts for others shown as count ("Read by N") — could become mini
  avatars under own last message like the follower row.
- Repo hygiene: not a git repo yet, no CI, near-zero tests (plan called for
  wiremock-based client-core tests + update() logic tests).

## Environment notes for a fresh chat
- rustup toolchain, target `x86_64-pc-windows-msvc`; build logs pattern:
  `cargo build --release 2>&1 | Out-File $env:TEMP\thornychat_build.log`.
- App data: `%APPDATA%\ThornyChat\ThornyChat\data\<profile>\` (store, logs,
  emoji-cache incl. usage.json). Pre-rename installs are migrated from
  `%APPDATA%\Synapse\Synapse` on first launch (see `client-core/src/store.rs`).
- Debug an issue: `$env:RUST_LOG="info,client_core=debug"` then read
  `...\data\default\logs\thornychat.log.<date>`.
- Custom state events (emoji packs, power tags) must be fetched via
  `client.send(get_state_events_for_key)` — sliding sync's required_state
  never includes them; same pattern for any future MSC state.
- iced gotchas learned: widget state is positional (use `theme::slot` for
  conditional elements near inputs), no `Length::Fill` inside vertical
  scrollables, overlays via always-present `stack!`, container::visible_bounds
  for viewport probing, `anchor_bottom()` for chat scroll semantics.
- Native child windows (WebView2/wry, see `ui/src/video_player.rs`): the
  webview is not `Send` and needs the thread that pumps the parent HWND's
  messages — do ALL create/set_bounds/close inside
  `iced::window::run_with_handle` closures (they run on the winit event-loop
  thread) against a thread_local. A child HWND always composites above the
  wgpu surface, so overlays must be screen-fixed (lightbox), not scrolling
  inline; `video_rect()` is the single geometry source for both the iced
  frame and the native bounds.
