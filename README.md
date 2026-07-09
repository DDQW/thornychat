# ThornyChat

A desktop-first, Windows-first Matrix client built in Rust with [iced](https://github.com/iced-rs/iced) and [matrix-rust-sdk](https://github.com/matrix-org/matrix-rust-sdk).

See [`.claude/plans`](.claude) history for the full architecture and phased build plan. Current status: **Phase 0** (scaffold, auth, basic sync) — the workspace, the tokio/iced async bridge, and password/SSO login with session persistence are wired up; room list/timeline rendering lands in Phase 1.

## Workspace layout

- `crates/client-core` — matrix-sdk wrapper, session/sync/state management. No `iced` dependency.
- `crates/ui` — all iced views. Never touches `matrix_sdk::*` types directly, only the plain boundary types in `client_core::events`/`client_core::commands`.
- `crates/app` — thin binary crate: tokio runtime bootstrap + iced entrypoint.

## Building

Requires a stable Rust toolchain (`rustup default stable`, MSVC target on Windows) and a Matrix account/homeserver to log into.

```
cargo build
cargo run
```

Session data and logs are stored under `%APPDATA%\ThornyChat\ThornyChat\data\<profile>` (profile defaults to `default`; pass a profile name as the first CLI argument to run multiple accounts side by side). Installs that predate the rename to ThornyChat are migrated from `%APPDATA%\Synapse` automatically on first launch.

### Release builds (Zen-tuned)

Standard release-build approach: run `scripts/build-zen-release.ps1` instead of a bare `cargo build --release`. It produces three `-C target-cpu` variants side by side, each in its own `target/` subdirectory (`target/znver2`, `target/znver3`, `target/znver4`):

- **znver2** - first full-width 256-bit AVX2 datapath; the `curve25519-dalek` crypto inflection point (Megolm/E2E key ops run at full AVX2 rate instead of Zen 1's half-rate).
- **znver3** - adds VAES/VPCLMULQDQ (vectorized bulk AES for media/attachments) plus the single-thread/IPC uplift that's the one actually perceptible in the iced UI event loop.
- **znver4** - AVX-512 (incl. IFMA) + DDR5 bandwidth; matches this project's dev machine, fastest startup/initial-sync bursts.

Zen 1 and Zen 5 are intentionally skipped (weak AVX2 / not a target machine). Don't ship a `target-cpu=native` or single hand-picked `znverN` binary generally - older CPUs will `SIGILL` on instructions they lack, so pick the matching variant above instead.

Note: this scaffold was written without a local Rust toolchain to compile against, so `matrix-sdk`/`matrix-sdk-ui`/`iced` API surface (login builder methods, `SyncService` state enum, `iced::application` builder methods) may need small adjustments to match the exact versions `cargo` resolves on first build — the files most likely to need reconciliation are called out in comments (`session.rs`, `sync.rs`, `main.rs`).
