//! Room list, per-room timeline, and spaces wrappers around
//! `matrix-sdk-ui`. Populated in Phase 1 — see `sync::handle_command`'s
//! `OpenRoom`/`CloseRoom` arms for where this plugs into the worker loop.

pub mod emoji_packs;
pub mod power_tags;
pub mod room_list;
pub mod spaces;
pub mod timeline;
