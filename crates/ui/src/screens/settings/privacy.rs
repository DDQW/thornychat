//! Privacy controls: what your activity reveals to other users and to
//! third-party servers. Each toggle writes straight through to the live
//! `PrivacyConfig` (so `update.rs` gates the next command with it
//! immediately) and autosaves to disk. Every option ships disabled by
//! default — the most private stance; see `crate::privacy_config`.

use iced::widget::{column, row, text, toggler};
use iced::{Element, Length, Task};

use crate::privacy_config::PrivacyConfig;

#[derive(Debug, Clone, Copy, Default)]
pub struct State;

#[derive(Debug, Clone)]
pub enum Message {
    ReadReceiptsToggled(bool),
    TypingToggled(bool),
    LinkPreviewsToggled(bool),
    /// Autosave task finished; nothing to do (mirrors `appearance::Saved`).
    Saved,
}

pub fn update(privacy: &mut PrivacyConfig, message: Message) -> Task<Message> {
    match message {
        Message::ReadReceiptsToggled(on) => privacy.send_read_receipts = on,
        Message::TypingToggled(on) => privacy.send_typing_notifications = on,
        Message::LinkPreviewsToggled(on) => privacy.enable_link_previews = on,
        Message::Saved => return Task::none(),
    }
    save_task(*privacy)
}

fn save_task(privacy: PrivacyConfig) -> Task<Message> {
    let (Some(path), Some(contents)) = (PrivacyConfig::config_path(), privacy.to_json_pretty())
    else {
        return Task::none();
    };
    Task::future(async move {
        if let Some(parent) = path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        if let Err(error) = tokio::fs::write(path, contents).await {
            tracing::warn!(%error, "failed to save privacy settings");
        }
        Message::Saved
    })
}

pub fn view<'a>(privacy: &'a PrivacyConfig) -> Element<'a, Message> {
    let intro = text(
        "ThornyChat defaults to a high-privacy stance: until you turn these on, your \
         activity isn't shared with other people or third-party servers.",
    )
    .size(12)
    .style(text::secondary);

    column![
        text("Privacy").size(14).font(crate::theme::SEMIBOLD_FONT),
        intro,
        privacy_toggle(
            "Send read receipts",
            "Let others see which messages you've read, and when. When off, your read \
             position is tracked privately — your own unread markers still clear, but \
             nobody else can see it.",
            privacy.send_read_receipts,
            Message::ReadReceiptsToggled,
        ),
        privacy_toggle(
            "Send typing notifications",
            "Show others a \"typing...\" indicator while you write a message.",
            privacy.send_typing_notifications,
            Message::TypingToggled,
        ),
        privacy_toggle(
            "Enable link previews",
            "Unfurl links in messages into preview cards. This contacts your homeserver \
             and, for some links, third-party sites (Twitter, Steam) — which can reveal \
             your IP address and what you're reading.",
            privacy.enable_link_previews,
            Message::LinkPreviewsToggled,
        ),
    ]
    .spacing(16)
    .into()
}

/// A titled toggle with an explanatory sub-line, so each option's privacy
/// trade-off is legible at a glance rather than hidden behind a bare label.
fn privacy_toggle<'a>(
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
