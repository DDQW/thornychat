//! `ui`: all iced widget/view code. Never depends on `matrix_sdk::*`
//! directly — only on the plain boundary types in
//! `client_core::events`/`client_core::commands`.

pub mod emoji_picker;
pub mod media_cache;
pub mod message;
pub mod screens;
pub mod state;
pub mod steam;
pub mod subscriptions;
pub mod theme;
pub mod tweets;
pub mod twemoji;
pub mod update;
pub mod video_player;
pub mod view;
pub mod widgets;

pub mod platform;

pub use message::Message;
pub use state::{boot, App, Route};
pub use subscriptions::subscription;
pub use update::update;
pub use view::view;
