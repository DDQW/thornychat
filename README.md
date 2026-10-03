# ThornyChat

Windows first Matrix Client with lots of preview features and connectors to steam and other stuff to autosend messages about your activities.

This is basically tailormade for myself but maybe someone else has fun with this too

Kind Regards

Wölki

## How it's built

A [Tauri 2](https://tauri.app) app. The Rust side owns everything that matters
to a Matrix client — the session, end-to-end encryption, sync, the media cache,
the Windows integration — and the window is a Svelte 5 page talking to it:

| Part | What it is |
| ---- | ---------- |
| `crates/client-core` | The matrix-sdk wrapper. Knows nothing about any UI: it speaks only in the plain `ClientCommand` / `ClientEvent` types. |
| `crates/desktop` | The Tauri shell: the bridge to `client-core`, the `tcmedia` media protocol, settings, game-activity connectors, tray, autostart, logging. |
| `frontend` | The UI: Svelte 5 + TypeScript + Vite. The protocol types it imports are generated from the Rust ones. |
| `crates/xtask` | `cargo xtask`, the release build. |

The page has no network access of its own (see the CSP in
`crates/desktop/tauri.conf.json`): Matrix media, Twemoji and link-card images all
arrive through the Rust side, which restricts what it will fetch.

## Building

Windows-only workspace; everything targets `x86_64-pc-windows-msvc`.

Prerequisites:

- **Rust stable.** `rust-toolchain.toml` pins the channel and pulls in the
  `x86_64-pc-windows-msvc` target plus `clippy`, so rustup provisions it on the
  first build.
- **MSVC build tools** — Visual Studio (or the standalone Build Tools) with the
  "Desktop development with C++" workload. `libsqlite3-sys` compiles SQLite from
  source (matrix-sdk-sqlite's `bundled` feature) and `aws-lc-sys` needs it too.
- **CMake** — also for `aws-lc-sys`.
- **Node.js 22.12 or newer** (with npm) for the frontend.
- **The Tauri CLI**, for `cargo tauri dev` and the installer:
  `cargo install tauri-cli --version "^2" --locked`.
- **WebView2.** Part of Windows 11; the installer fetches it on Windows 10.

### Development

```
npm --prefix frontend ci      # once
cargo tauri dev               # Vite dev server + the app, with hot reload
```

Pass an argument after `--` to run a separate profile (its own account and data,
side by side with your real one): `cargo tauri dev -- -- my-test-profile`.

Open `http://localhost:1420` in a plain browser (`npm --prefix frontend run dev`)
to work on the UI without a homeserver: outside Tauri the page talks to a mock of
the sync worker (`frontend/src/lib/mock.ts`).

Checks, the same ones CI runs:

```
npm --prefix frontend run check     # svelte-check
npm --prefix frontend test          # vitest
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The protocol types in `frontend/src/lib/bindings` are generated from
`client-core`. After changing a command or event, run `cargo xtask bindings` and
commit the result (CI fails if they drift). A *new* type also needs a line in
`frontend/src/lib/bindings/index.ts`.

### Release

`cargo xtask` is the standard release build — it builds the frontend, then three
binaries, each in its own `target/` subdirectory so they don't overwrite each
other. A bare `cargo build --release` is **not** equivalent: without the
`custom-protocol` feature the exe expects the dev server instead of carrying its
frontend.

| Variant  | Path                                                  | What it is |
| -------- | ----------------------------------------------------- | ---------- |
| generic  | `target/x86_64-pc-windows-msvc/release/thornychat.exe` | Baseline x86-64: runs on any 64-bit CPU. The one to hand to someone else or ship as a download. |
| znver4   | `target/znver4/x86_64-pc-windows-msvc/release/thornychat.exe` | AVX-512 (incl. IFMA) + DDR5, on Zen 4's double-pumped 256-bit units; the dev machine's native CPU. |
| znver5   | `target/znver5/x86_64-pc-windows-msvc/release/thornychat.exe` | Zen 5: a native full-width 512-bit AVX-512 datapath plus wider dispatch — the newest generation. |

**Never ship a znverN (or `target-cpu=native`) binary to unknown hardware.** A CPU
without those instructions dies with an illegal-instruction fault. The generic
build is the one that's safe everywhere. (The CPU level only matters for
matrix-sdk's crypto; the webview does the rest.)

`cargo xtask installer` builds the per-user NSIS installer (generic CPU), which also
creates the Start Menu shortcut Windows needs to attribute toast notifications to
ThornyChat.

## License

ThornyChat is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version. See [LICENSE](LICENSE) for the full text.

Copyright © 2026 Dominik Wölki
