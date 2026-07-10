# ThornyChat

A native Windows [Matrix](https://matrix.org) client written in Rust — [iced](https://github.com/iced-rs/iced) for the interface, [matrix-rust-sdk](https://github.com/matrix-org/matrix-rust-sdk) for the protocol. No Electron, no embedded browser doing the rendering: the whole client is one self-contained executable drawing its own pixels, which is why it starts in a blink and idles light.

Developed by **Dominik Wölki**. Licensed under the **GNU GPL v3**.

## Why another Matrix client?

- **Native, not web.** The UI is immediate-mode Rust (wgpu-rendered), not a packaged website. Cold start is fast, scrolling is smooth, and memory stays flat in long sessions.
- **Privacy by default.** Read receipts are sent as *private* receipts and typing notifications are off unless you opt in. The opt-in game connectors share nothing until you flip them on. There is no telemetry of any kind.
- **IRC-era ergonomics on a modern protocol.** Slash commands for everything, `/me` actions in a configurable color, a single-line composer where Enter sends, and predictable, user-controlled behavior over clever heuristics — the client never guesses at what you meant.
- **Built for real rooms.** Custom emoji packs, shortcode reactions that aggregate with other clients, inline video, and link cards were built against busy production rooms, not demo servers.

## Features

**Timeline.** Incremental diff streaming — only items that actually changed cross from the sync engine to the UI, so a busy room costs almost nothing per tick. Bottom-anchored scrolling with a floating "jump to latest" pill, day and unread dividers, consecutive-message grouping, message search over loaded history, inline edits and deletions, rich replies with click-to-jump quotes, and `/me` emotes rendered inline with the sender's name.

**Custom emoji, stickers, reactions.** Full [MSC2545](https://github.com/matrix-org/matrix-spec-proposals/pull/2545) image-pack support: room packs, personal packs, and globally enabled packs, including animated GIF emotes. Reactions key on `:shortcode:` so they aggregate with Cinny and friends instead of splitting into duplicate pills. A "frequently used" row learns your habits, and stickers you see in rooms are collected for reuse.

**Media.** Inline images reserve their exact footprint before the bytes arrive, so the timeline never jumps as media loads. A fullscreen lightbox offers cursor-anchored wheel zoom up to 20×, drag-to-pan, Lanczos sharpening when you zoom past native resolution, double-click or margin-click to close, and one-click download. Paste an image or file straight into the composer with Ctrl+V, drag-and-drop files onto the window, and caption the first attachment by just typing.

**Inline video.** YouTube, Vimeo, Dailymotion, Rumble, Kick, and direct `.mp4`/`.webm` links play *inside the timeline* (via a WebView2 surface glued over the message card) — no browser tab, and the card never changes size between preview and playback.

**Link previews.** OpenGraph cards through your homeserver's preview proxy, plus richer cards for tweets and Steam store pages. Video cards render with zero third-party fetches; everything else respects the privacy toggle.

**Slash commands.** `/me`, `/plain`, `/shrug` and friends, `/join`, `/leave`, `/roomname`, `/topic`, `/invite`, `/dm`, `/nick`, `/kick`, `/ban`, `/unban` — one command table drives both the parser and the built-in manual, so the docs can't drift from the behavior. Unknown `/words` send as normal text and `//` escapes a literal slash.

**Composer.** Markdown formatting on send, @-mention autocomplete, `:shortcode:` insertion, spell check through the Windows spell checker shown as a quiet suggestion bar (no red squiggles), and opt-in autocorrect where Backspace undoes the last correction. A right-click menu covers Cut/Copy/Paste/Select-all.

**Encryption & security.** E2EE rooms via the Matrix Rust SDK, emoji (SAS) device and user verification, cross-signing bootstrap, per-message trust shields, and key backup/recovery that is strictly opt-in — the client will never nag you about keys.

**Rooms, spaces, DMs.** A Cinny-style sidebar nests each space's rooms under its header, with a space explorer for browsing and joining children. Start DMs from a user-directory search, create rooms, and rename/leave/forget from a right-click. Per-room notification modes (all / mentions / mute) plus account-wide defaults.

**Game connectors (opt-in).** Steam, GOG Galaxy, and Epic watchers that post an IRC-style `* you plays …` action to the room you're viewing when the game you're running changes. Detection is local (registry + process list), off by default, and shares nothing until enabled.

**Reliability.** Failed sends surface a Retry affordance on the message itself; the send queue re-enables automatically a few seconds after a recoverable error and again when the connection comes back. A quiet status line appears above the composer only while the connection is actually unhealthy.

**Theming & polish.** Dark and light presets with ten user-editable colors (including the emote color), adjustable UI scale, corner radius, and font, with theme import/export. Bundled CJK font fallback so nothing renders as tofu. Multiple accounts run side by side via profile arguments. An in-app manual (the "?" in Settings) documents every feature and shortcut.

## Not there yet

Desktop notifications and a tray icon, a threads panel (reply counts show, no panel), audio/video in calls (call presence works), point-and-click member management and power-level editing (the moderation slash commands cover the basics), downloading non-image files, media in encrypted rooms (placeholders for now), and server-side search.

## Building

Windows, stable Rust with the MSVC toolchain:

```
cargo build          # debug
cargo run
```

For release binaries, use the release script instead of a bare `cargo build --release`:

```
powershell -File scripts/build-zen-release.ps1
```

It produces three variants, each in its own `target/` subdirectory:

| Variant | Path | For |
| --- | --- | --- |
| generic | `target/x86_64-pc-windows-msvc/release/` | Any 64-bit CPU — the one to distribute |
| znver4 | `target/znver4/x86_64-pc-windows-msvc/release/` | AMD Zen 4 (AVX-512) |
| znver5 | `target/znver5/x86_64-pc-windows-msvc/release/` | AMD Zen 5 (full-width AVX-512) |

Only ever hand the **generic** build to unknown hardware — the `znverN` variants use instructions older CPUs don't have and will die with an illegal-instruction fault there. Inline video playback uses the WebView2 runtime, which ships with current Windows.

## Data locations

Session data, media caches, and logs live under `%APPDATA%\ThornyChat\ThornyChat\data\<profile>` (profile `default` unless you pass another name as the first CLI argument; each profile is an independent account login). Session secrets are stored in the Windows Credential Manager, not on disk. Global settings (theme, privacy, connectors) live in `%APPDATA%\ThornyChat\ThornyChat\config`.

## Workspace layout

- `crates/client-core` — matrix-sdk wrapper: session, sync worker, per-room timelines. No iced dependency.
- `crates/ui` — every iced view and widget. Talks to the core only through plain command/event types, never `matrix_sdk` types.
- `crates/app` — thin binary: tokio runtime bootstrap plus the iced entrypoint.

## License

ThornyChat is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version. See [LICENSE](LICENSE) for the full text.

Copyright © 2026 Dominik Wölki
