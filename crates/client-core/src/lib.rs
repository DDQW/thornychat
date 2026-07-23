//! `client-core`: the matrix-sdk wrapper and session/sync state management
//! layer. This crate has no dependency on `iced` — everything it exposes
//! across the `ui` boundary is a plain type from [`events`] and
//! [`commands`], so the GUI layer never touches `matrix_sdk::*` directly.

// ruma 0.16's deeply-nested event enums push the trait-resolution recursion
// past the default 128 during type-checking; lift it so the crate compiles.
#![recursion_limit = "256"]

pub mod calls;
pub mod client;
pub mod commands;
pub mod error;
pub mod events;
pub mod key_backup;
pub mod media;
pub mod push;
pub mod rooms;
pub mod search;
pub mod session;
pub mod store;
pub mod sync;
pub mod verification;

pub use client::{try_start, start_with_password, start_with_sso, RunningClient};
pub use commands::ClientCommand;
pub use error::{CoreError, CoreResult};
pub use events::ClientEvent;
