//! Encryption defaults for new conversations. Each toggle writes straight
//! through to the live `EncryptionConfig` (so the next create command reads
//! it immediately) and autosaves to disk. Both ship off by default — see
//! `crate::encryption_config`.

use iced::widget::{column, row, text, toggler};
use iced::{Element, Length, Task};

use crate::encryption_config::EncryptionConfig;

#[derive(Debug, Clone, Copy, Default)]
pub struct State;

#[derive(Debug, Clone)]
pub enum Message {
    DirectMessagesToggled(bool),
    RoomsToggled(bool),
    /// Autosave task finished; nothing to do (mirrors `privacy::Saved`).
    Saved,
}

pub fn update(config: &mut EncryptionConfig, message: Message) -> Task<Message> {
    match message {
        Message::DirectMessagesToggled(on) => config.encrypt_direct_messages = on,
        Message::RoomsToggled(on) => config.encrypt_rooms = on,
        Message::Saved => return Task::none(),
    }
    save_task(*config)
}

fn save_task(config: EncryptionConfig) -> Task<Message> {
    let (Some(path), Some(contents)) = (EncryptionConfig::config_path(), config.to_json_pretty())
    else {
        return Task::none();
    };
    Task::future(async move {
        if let Some(parent) = path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        if let Err(error) = tokio::fs::write(path, contents).await {
            tracing::warn!(%error, "failed to save encryption settings");
        }
        Message::Saved
    })
}

pub fn view(config: &EncryptionConfig) -> Element<'_, Message> {
    let intro = text(
        "Whether new conversations you start are end-to-end encrypted. Encryption is \
         locked in when a room is created, so this only affects ones you make from now \
         on — existing chats are unchanged, and any existing DM is reused as-is.",
    )
    .size(12)
    .style(text::secondary);

    column![
        text("Encryption").size(14).font(crate::theme::SEMIBOLD_FONT),
        intro,
        encryption_toggle(
            "Encrypt new direct messages",
            "When on, a DM you start is end-to-end encrypted. Off (the default) matches \
             the plain DMs most rooms use and avoids key-management friction.",
            config.encrypt_direct_messages,
            Message::DirectMessagesToggled,
        ),
        encryption_toggle(
            "Encrypt new rooms",
            "When on, a room you create is end-to-end encrypted. Off (the default) keeps \
             it open — simpler for bots, bridges and readable history.",
            config.encrypt_rooms,
            Message::RoomsToggled,
        ),
    ]
    .spacing(16)
    .into()
}

/// A titled toggle with an explanatory sub-line — same shape as the privacy
/// tab's rows so the two read consistently.
fn encryption_toggle<'a>(
    title: &'a str,
    description: &'a str,
    value: bool,
    on_toggle: impl Fn(bool) -> Message + 'a,
) -> Element<'a, Message> {
    row![
        column![text(title).size(13), text(description).size(11).style(text::secondary)]
            .spacing(2)
            .width(Length::Fill),
        toggler(value).on_toggle(on_toggle),
    ]
    .spacing(12)
    .align_y(iced::Center)
    .into()
}
