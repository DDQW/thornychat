# Migration plan: iced → Tauri

Decided 2026-09-29. The UI moves from iced to a Tauri shell with a Svelte
frontend. `client-core` stays as the Rust backend.

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
| `spellcheck.rs` (ISpellChecker) | **Keep for autocorrect only** if WebView2's own typo marking is confirmed in Phase 0. |
| `video_player.rs`, `animated_image.rs`, `synthetic_input.rs`, `spellcheck_highlight.rs`, `lightbox_image.rs`, `theme.rs`, `render_watchdog.rs`, `clipboard_paste.rs` (mostly) | **Delete.** The browser covers these. |
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
