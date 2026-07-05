# Synapse

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

Session data and logs are stored under `%APPDATA%\Synapse\<profile>` (profile defaults to `default`; pass a profile name as the first CLI argument to run multiple accounts side by side).

Note: this scaffold was written without a local Rust toolchain to compile against, so `matrix-sdk`/`matrix-sdk-ui`/`iced` API surface (login builder methods, `SyncService` state enum, `iced::application` builder methods) may need small adjustments to match the exact versions `cargo` resolves on first build — the files most likely to need reconciliation are called out in comments (`session.rs`, `sync.rs`, `main.rs`).
