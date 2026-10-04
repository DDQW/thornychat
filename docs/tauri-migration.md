# Migration plan: iced → Tauri

Decided 2026-09-29. The UI moves from iced to a Tauri shell with a Svelte
frontend. `client-core` stays as the Rust backend.

## Status (2026-10-03)

The iced UI is **gone**: `crates/ui` and `crates/app` were deleted in the same
change that added `crates/desktop` and `frontend/`, instead of the originally
planned build-alongside. That was the explicit call at the time; the last iced
build is the `iced-final` tag and git history has everything else. The
"Strategy" row below is kept as the original intent.

| Phase | State |
|---|---|
| 0 Spike | Folded into the build (see "Spike findings"). |
| 1 Prepare | Done: serde derives and generated TypeScript types (`ts-rs`, not `tauri-specta` — it is still release-candidate); sanitized `formatted_body` on timeline items; non-UI Rust moved into `crates/desktop`. |
| 2 Shell and bridge | Done: single dispatcher + event channel, `tcmedia` protocol with Range support, settings, single instance, logging, shutdown deadline, strict CSP, navigation blocking, links opened externally. |
| 3 Frontend foundation | Done: Svelte 5 runes stores, diff application, theme → CSS variables. |
| 4 Screens | Done: login, room list + spaces + explorer, timeline, composer, reactions, edit/redact/reply, emoji picker (+custom packs, stickers), lightbox, link/tweet/Steam/video cards, members panel, settings (all tabs), verification + recovery, call banner. |
| 5 Platform | Done: tray, autostart, window state, game connectors, toast on push (the event itself is still never emitted — see ROADMAP). Not done: toast click-through. |
| 6 Packaging | NSIS installer via `cargo xtask installer`; the three CPU variants kept; CI builds the frontend first. Updater and signing undecided. |
| 7 Cutover | Done in one step (no dogfood period). The "Done" list in `ROADMAP.md` is the parity checklist; its migration section lists what differs. |

### Spike findings

1. **Timeline.** No virtualization: the worker already windows the list and the page
   reopens it past 200 items, so rows are plain DOM. Two things the browser does by
   itself were wrong for chat and are done by hand: scroll anchoring is off (it does
   nothing at scrollTop 0), and prepending history compensates for the added height.
   Verified against a mock worker, including the scrollTop-0 case — which turned out
   not to be enough (2026-10-04): the first version detected a prepend by the old
   first row reappearing further down, but the SDK replaces the top date divider on
   every same-day back-pagination, so on a real account nothing was compensated and
   history loaded page after page under a reader parked at the top. It now anchors
   on the message at the top of the view and keeps it in place on any resize, and the
   mock replaces its top divider the way the SDK does. Sticking to the bottom also
   lost out to the composer's `height: auto` autosize (one forced layout per line
   clamped the timeline's scrollTop); the composer uses `field-sizing: content` now,
   and only an upward scroll lets go of the bottom.
2. **Media.** A `tcmedia` custom protocol backed by the existing disk cache; no bytes
   cross IPC. Verified in the built exe over `https://tcmedia.localhost`, including
   that traversal, unknown hosts and out-of-allow-list URLs are refused.
3. **Embeds.** The window is created with `use_https_scheme` so the page's origin is
   `https://tauri.localhost` — YouTube refuses embedders without a real https
   origin. The player still stayed black at first: on Windows, wry runs every
   initialization script in every frame (Tauri's "main frame only" flag is
   ignored), so `freezePrototype` froze `Object.prototype` inside YouTube's
   iframe too, and its player script throws on that. The freeze is now our own
   script, applied to the top-level document only (`main.rs`). Tauri's IPC
   bootstrap reaches those frames the same way; its ACL rejects their calls
   (checked: `invoke` from the YouTube frame → "not allowed by ACL"). Player
   pop-ups ("Watch on YouTube") go to the system browser through `on_new_window`.
   Verified in the built exe, 2026-10-03.
4. **Spellcheck.** WebView2's built-in checker marks **nothing** in this app — not
   in a `textarea`, not in `contenteditable`, with `spellcheck` on and the Windows
   language list including English (checked on the built exe, 2026-10-03). The
   `ISpellChecker` engine was therefore ported (`crates/desktop/src/spellcheck.rs`,
   one STA thread behind Tauri commands) with the marking, suggestion bar and
   autocorrect rebuilt in the page (`frontend/src/lib/spell.ts`).
5. **`formatted_body`.** Sanitized in Rust (Matrix spec allow-list, reply fallback
   removed), then again in the page (DOMPurify with the same list, no remote images,
   no author classes/ids/styles), then constrained by the CSP. Covered by tests on
   both sides; the CSP and navigation guard verified in the built exe.
6. **Resources.** **Not measured** (needs a signed-in session to be a fair
   comparison with the iced build). Idle login screen: ~34 MB in the main process,
   plus the WebView2 processes.

### What nobody has run yet

Everything that needs a real account: sync, a real timeline, Matrix media through
`tcmedia`, uploads, SSO, key backup and verification. The page was exercised
against a mock of the sync worker and the Rust side against unit tests and the
no-account paths of the real exe.

## Decisions

| Question | Decision |
|---|---|
| Frontend | Svelte 5 + TypeScript + Vite |
| `formatted_body` (rich message HTML) | **In scope for parity**, not deferred. Sanitized in Rust *and* in the frontend (defense in depth). |
| Emoji | Keep Twemoji (consistent look), fetched and cached by the backend as today |
| Resource ceiling | None fixed. The move is accepted if the app genuinely works better; Phase 0 still records idle RAM/CPU so the trade-off is known, not a gate. |
| Strategy | Build alongside; the iced app stays releasable until parity. Tag `iced-final` before deleting it. No history rewrites. |

## Why the migration is cheaper than the line count suggests

- `client-core` has no iced dependency. It talks to the UI only through
  `ClientCommand` (~60 variants) and `ClientEvent` (~40). Timeline updates are
  already incremental `TimelineDiff`s. Both map almost one-to-one onto Tauri
  commands and events.
- Neither enum derives `Serialize`/`Deserialize` yet. That is the first code change.
- Much of `crates/ui` is not UI: Win32/COM code, connectors, tweet/Steam
  fetchers, config files, the slash-command parser.
- Several files exist only to work around iced (`render_watchdog`,
  `synthetic_input`, `spellcheck_highlight`, `video_player`, `animated_image`)
  and should disappear.
- Today the timeline carries plain-text bodies only. Rendering `formatted_body`
  is new work, not a port.

## What happens to each module

| Current | Fate |
|---|---|
| `client-core` | **Keep.** Add serde derives, TS type generation (`tauri-specta`), and a sanitized `formatted` field on text-like timeline content. |
| `ui/platform/*` (toasts, AUMID, tray, autostart, power, relaunch, file dialog) | **Move** to a non-iced crate. Toasts and tray keep custom code if click-to-open-room needs it; autostart and dialogs may become Tauri plugins. |
| `connectors/*`, `tweets.rs`, `steam.rs`, `twemoji.rs` fetching, `*_config.rs` | **Move** to the backend crate; expose as commands / the media protocol. |
| `slash.rs` | **Port to TS**, with its tests. |
| `spellcheck.rs` (ISpellChecker) | **Kept** (WebView2's own marking does nothing here): `crates/desktop/src/spellcheck.rs`, with the highlight/autocorrect logic ported to `frontend/src/lib/spell.ts`. |
| `video_player.rs`, `animated_image.rs`, `synthetic_input.rs`, `lightbox_image.rs`, `theme.rs`, `render_watchdog.rs`, `clipboard_paste.rs` (mostly) | **Delete.** The browser covers these (`upscale.rs` lives on as `crates/desktop/src/upscale.rs` behind the media protocol). |
| `update.rs`, `state.rs`, `view.rs`, `screens/*`, `emoji_picker.rs`, `media_cache.rs` | **Rewrite** in the frontend. This is the bulk of the work. |
| `xtask` | **Adapt** to drive `cargo tauri build` for the three CPU variants. |

## Phases

### Phase 0: Spike (go/no-go)

Throwaway Tauri app on `client-core`. Validate:

1. **Timeline**: variable-height messages, bottom anchoring, prepending history
   without scroll jumps, images loading in. Try `virtua` first, TanStack Virtual
   as fallback.
2. **Media**: custom `mxc://` URI protocol backed by the existing disk cache and
   size caps. No bytes over IPC.
3. **Embeds**: YouTube/Vimeo/etc. iframes from a Tauri origin (known to be
   finicky with app origins).
4. **Spellcheck**: does WebView2 mark typos in `textarea`/`contenteditable` using
   the Windows dictionary?
5. **`formatted_body`**: sanitize in Rust, render in Svelte, confirm no script
   or remote-content path reaches the IPC-enabled context.
6. **Resource numbers**: idle RAM/CPU vs the iced build (method in
   `docs/log-volume-2026-09.md`). Recorded, not a gate.

Exit: items 1-5 pass, or the specific gaps are accepted.

### Phase 1: Prepare in place (iced app unchanged)

- Commit or stash the working tree first.
- Extract non-UI Rust out of `ui` into a new crate that `ui` depends on.
- Add serde/specta derives to commands and events. Keep `Zeroizing<String>`
  fields out of generated types and logs.
- Add sanitized `formatted` content to the timeline item types.

Exit: the iced app builds and behaves as before.

### Phase 2: Tauri shell and bridge

- New `crates/desktop` Tauri crate.
- Profile argument, logging and log limits, shutdown deadline.
- Single command dispatcher; event pump into a Tauri `Channel`.
- Media protocol, window geometry, single-instance, config get/set commands.
- Security: strict CSP, minimal capabilities, no remote content in the
  IPC-enabled context, links opened externally.

Exit: login, sync, and one room's event stream reach a stub frontend.

### Phase 3: Frontend foundation

- Svelte 5 + TS + Vite; vitest.
- Stores for session, rooms, timelines; `TimelineDiff` applied in TS.
- `theme_config` mapped to CSS variables; design tokens.

### Phase 4: Screens, in dependency order

1. Login (password, SSO), server discovery
2. Room list, spaces sidebar, space explorer
3. Timeline, read-only (dividers, grouping, receipts, jump-to-latest,
   `formatted_body`)
4. Composer (markdown, mention pills, attachments, replies, edit/redact,
   typing, slash commands). Editor choice for mention pills (`contenteditable`
   vs Lexical/Tiptap) is decided in Phase 0 or early Phase 4.
5. Reactions, threads, emoji picker with custom emoji packs, Twemoji
6. Rich media: lightbox, file save, video embeds, URL/tweet/Steam cards
7. Members panel, DM search, context menus
8. Settings (all tabs), room admin
9. Verification, key backup and recovery
10. Call banner

### Phase 5: Platform parity

Toast activation, tray, autostart, sleep/resume, paste-to-attach,
drag-and-drop, game-activity connectors.

### Phase 6: Packaging

- NSIS installer with a real Start Menu shortcut and AUMID (should retire
  `cargo xtask install-dev`).
- Keep the three CPU variants (they matter only for matrix-sdk crypto).
- WebView2 bootstrapper policy for Windows 10.
- CI: add Node + Tauri build.
- Updater decision.

### Phase 7: Cutover

- Walk the "Done" list in `ROADMAP.md` as the parity checklist.
- Dogfood, then make the Tauri binary the default.
- Remove `crates/ui`, the iced dependencies, and the iced workaround docs.
- Update README and ROADMAP.

## Risks

| Risk | Mitigation |
|---|---|
| Timeline scroll behaviour worse than iced | Spike first; keep iced until parity |
| XSS/injection in a webview with IPC access | Sanitize `formatted_body` in Rust and DOMPurify; strict CSP; minimal capabilities |
| Embeds refuse the Tauri origin | Phase 0 test; fall back to opening externally |
| Autocorrect / toast click-through don't fit a plugin | Keep custom Rust behind commands |
| Two half-finished UIs | iced stays shippable throughout; per-phase exit criteria |
| Idle memory/CPU regress | Measured in Phase 0 and re-measured before cutover, reported not gated |
