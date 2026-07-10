//! Activity connectors: auto-post what you're playing as an IRC-style emote
//! into the room you're currently viewing. Each toggle writes straight through
//! to the live `ConnectorsConfig` (so the poll subscription in
//! `subscriptions.rs` picks it up on the next frame) and autosaves to disk.
//! Everything ships disabled; see `crate::connectors_config`.

use iced::widget::{column, row, text, toggler};
use iced::{Element, Length, Task};

use crate::connectors_config::ConnectorsConfig;

#[derive(Debug, Clone, Copy, Default)]
pub struct State;

#[derive(Debug, Clone)]
pub enum Message {
    SteamToggled(bool),
    GogToggled(bool),
    EpicToggled(bool),
    AnnounceStopToggled(bool),
    /// Autosave task finished; nothing to do (mirrors `privacy::Saved`).
    Saved,
}

pub fn update(connectors: &mut ConnectorsConfig, message: Message) -> Task<Message> {
    match message {
        Message::SteamToggled(on) => connectors.steam_enabled = on,
        Message::GogToggled(on) => connectors.gog_enabled = on,
        Message::EpicToggled(on) => connectors.epic_enabled = on,
        Message::AnnounceStopToggled(on) => connectors.announce_stop = on,
        Message::Saved => return Task::none(),
    }
    save_task(*connectors)
}

fn save_task(connectors: ConnectorsConfig) -> Task<Message> {
    let (Some(path), Some(contents)) =
        (ConnectorsConfig::config_path(), connectors.to_json_pretty())
    else {
        return Task::none();
    };
    Task::future(async move {
        if let Some(parent) = path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        if let Err(error) = tokio::fs::write(path, contents).await {
            tracing::warn!(%error, "failed to save connector settings");
        }
        Message::Saved
    })
}

pub fn view(connectors: &ConnectorsConfig) -> Element<'_, Message> {
    let intro = text(
        "Auto-announce what you're playing as an action message (like /me plays …) \
         in the room you're currently viewing. The line is posted only when the game \
         starts or changes — nothing is shared until you turn on a launcher below.",
    )
    .size(12)
    .style(text::secondary);

    column![
        text("Connectors").size(14).font(crate::theme::SEMIBOLD_FONT),
        intro,
        connector_toggle(
            "Steam",
            "Announce the Steam game you're running (read from Steam's own \
             registry key — no login or API key needed).",
            connectors.steam_enabled,
            Message::SteamToggled,
        ),
        connector_toggle(
            "GOG Galaxy",
            "Announce installed GOG games when you launch them (matches running \
             programs against your installed GOG library).",
            connectors.gog_enabled,
            Message::GogToggled,
        ),
        connector_toggle(
            "Epic Games",
            "Announce installed Epic games when you launch them (matches running \
             programs against your installed Epic library).",
            connectors.epic_enabled,
            Message::EpicToggled,
        ),
        connector_toggle(
            "Announce when you stop",
            "Also post a \"stopped playing …\" line when you quit a game. Off by \
             default — usually just the start/switch line is wanted.",
            connectors.announce_stop,
            Message::AnnounceStopToggled,
        ),
    ]
    .spacing(16)
    .into()
}

/// A titled toggle with an explanatory sub-line — same shape as the Privacy
/// tab's `privacy_toggle`, so the two activity-sharing tabs read alike.
fn connector_toggle<'a>(
    title: &'a str,
    description: &'a str,
    value: bool,
    on_toggle: impl Fn(bool) -> Message + 'a,
) -> Element<'a, Message> {
    row![
        column![
            text(title).size(13),
            text(description).size(11).style(text::secondary),
        ]
        .spacing(2)
        .width(Length::Fill),
        toggler(value).on_toggle(on_toggle),
    ]
    .spacing(12)
    .align_y(iced::Center)
    .into()
}
