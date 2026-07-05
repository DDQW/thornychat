//! Markdown composer: live-preview toggle, @mention autocomplete, emoji
//! picker (unicode + custom emoji packs), and attachment picking. This
//! module never talks to `client_core::sync`/`mpsc` directly — it only
//! produces `Effect`s, which the root dispatcher (`ui::update`) turns into
//! actual `ClientCommand` sends, generating and tracking the `request_id`
//! needed to correlate the eventual
//! `ClientEvent::CommandSucceeded`/`CommandFailed`.

use std::collections::HashMap;

use client_core::commands::RequestId;
use client_core::events::{EmojiPack, ReplyPreview, RoomMember};
use iced::widget::{button, column, container, markdown, row, text, text_input};
use iced::{Element, Length, Task};

#[derive(Debug, Clone, Default)]
pub struct State {
    pub body: String,
    pub preview_items: Vec<markdown::Item>,
    pub show_preview: bool,
    pub show_emoji_picker: bool,
    pub member_candidates: Vec<RoomMember>,
    /// Lowercased display names, index-parallel to `member_candidates`
    /// (built once per roster update) — the mention filter runs on every
    /// view rebuild while an '@word' ends the draft, and lowercasing the
    /// whole roster per frame allocated thousands of Strings in big rooms.
    pub member_candidates_lower: Vec<String>,
    /// Mentions the user has confirmed by clicking an autocomplete
    /// candidate; attached as `m.mentions` on send, then cleared.
    pub mentioned: Vec<(String, String)>,
    /// Set while composing a reply — shown as a banner above the input and
    /// attached as the rich-reply relation on send.
    pub replying_to: Option<ReplyPreview>,
    pub pending_request: Option<RequestId>,
    /// In-flight attachment upload, tracked separately from text sends: if
    /// it shared `pending_request`, an attachment's CommandSucceeded would
    /// run the SendSucceeded reset and wipe a typed-but-unsent draft.
    pub pending_attachment_request: Option<RequestId>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    BodyChanged(String),
    TogglePreview(bool),
    Send,
    ToggleEmojiPicker,
    EmojiPicked(&'static str),
    CustomEmojiPicked { shortcode: String, mxc_url: String },
    MentionCandidateClicked(String, String),
    PickAttachment,
    AttachmentPicked(Result<(String, Vec<u8>), String>),
    LinkClicked(markdown::Url),

    CancelReply,

    /// Fed back by the root dispatcher once the in-flight command resolves.
    SendSucceeded,
    SendFailed(String),
}

pub enum Effect {
    None,
    Send { body: String, mentioned_user_ids: Vec<String>, reply_to_event_id: Option<String> },
    PickAttachment,
    SendAttachment { filename: String, bytes: Vec<u8>, mime: String },
    Typing(bool),
    EnsureEmojiFetched(Vec<String>),
    /// An emoji was used — the root dispatcher bumps the usage history
    /// that feeds the picker's "Frequently used" section. Key: the glyph
    /// for unicode, the `mxc://` URL for custom emoji.
    EmojiUsed(String),
}

pub fn update(state: &mut State, message: Message) -> (Task<Message>, Effect) {
    match message {
        Message::BodyChanged(body) => {
            let typing = Effect::Typing(!body.trim().is_empty());
            state.body = body;
            // A stale send/attach error shouldn't pin itself above the
            // composer once the user has moved on.
            state.error = None;
            recompute_preview(state);
            (Task::none(), typing)
        }
        Message::TogglePreview(show) => {
            state.show_preview = show;
            (Task::none(), Effect::None)
        }
        Message::Send => {
            // In-flight guard: a second Enter (or Enter + Send click) before
            // CommandSucceeded round-trips would post the message twice —
            // and overwrite pending_request, orphaning the first response.
            // pending_request is always cleared by SendSucceeded/SendFailed,
            // and the composer resets wholesale on room switch, so this
            // can't wedge.
            if state.pending_request.is_some() {
                return (Task::none(), Effect::None);
            }
            let body = state.body.trim().to_string();
            if body.is_empty() {
                return (Task::none(), Effect::None);
            }
            let mentioned_user_ids = state.mentioned.iter().map(|(id, _)| id.clone()).collect();
            let reply_to_event_id = state.replying_to.as_ref().map(|r| r.event_id.clone());
            (Task::none(), Effect::Send { body, mentioned_user_ids, reply_to_event_id })
        }
        Message::CancelReply => {
            state.replying_to = None;
            (Task::none(), Effect::None)
        }
        Message::ToggleEmojiPicker => {
            state.show_emoji_picker = !state.show_emoji_picker;
            let effect = if state.show_emoji_picker {
                Effect::EnsureEmojiFetched(crate::emoji_picker::all_unicode_glyphs())
            } else {
                Effect::None
            };
            (Task::none(), effect)
        }
        Message::EmojiPicked(glyph) => {
            state.body.push_str(glyph);
            recompute_preview(state);
            (Task::none(), Effect::EmojiUsed(glyph.to_string()))
        }
        Message::CustomEmojiPicked { shortcode, mxc_url } => {
            state.body.push_str(&format!(":{shortcode}: "));
            recompute_preview(state);
            (Task::none(), Effect::EmojiUsed(mxc_url))
        }
        Message::MentionCandidateClicked(user_id, display_name) => {
            if let Some(at_pos) = state.body.rfind('@') {
                state.body.truncate(at_pos);
            }
            state.body.push('@');
            state.body.push_str(&display_name);
            state.body.push(' ');
            if !state.mentioned.iter().any(|(id, _)| *id == user_id) {
                state.mentioned.push((user_id, display_name));
            }
            recompute_preview(state);
            (Task::none(), Effect::None)
        }
        Message::PickAttachment => (Task::none(), Effect::PickAttachment),
        Message::AttachmentPicked(Ok((filename, bytes))) => {
            // One upload at a time — a second pick would overwrite
            // pending_attachment_request and orphan the first response.
            if state.pending_attachment_request.is_some() {
                state.error = Some("An attachment is already uploading".into());
                return (Task::none(), Effect::None);
            }
            let mime = mime_guess::from_path(&filename).first_or_octet_stream().to_string();
            (Task::none(), Effect::SendAttachment { filename, bytes, mime })
        }
        Message::AttachmentPicked(Err(reason)) => {
            state.error = Some(reason);
            (Task::none(), Effect::None)
        }
        Message::LinkClicked(url) => {
            let _ = open::that(url.as_str());
            (Task::none(), Effect::None)
        }
        Message::SendSucceeded => {
            state.body.clear();
            state.mentioned.clear();
            state.preview_items.clear();
            state.replying_to = None;
            state.pending_request = None;
            state.error = None;
            (Task::none(), Effect::Typing(false))
        }
        Message::SendFailed(reason) => {
            state.pending_request = None;
            state.error = Some(reason);
            (Task::none(), Effect::None)
        }
    }
}

fn recompute_preview(state: &mut State) {
    state.preview_items = markdown::parse(&state.body).collect();
}

/// The `@partial` word currently being typed at the end of the composer, if
/// any — drives the mention-autocomplete list. Only looks at the trailing
/// word (simple, correct for top-to-bottom typing; editing a mention
/// mid-message won't retrigger the dropdown, an acceptable trade-off here).
fn active_mention_query(body: &str) -> Option<&str> {
    let last_word = body.rsplit(char::is_whitespace).next()?;
    last_word.strip_prefix('@')
}

pub fn view<'a>(
    state: &'a State,
    emoji_usage: &'a HashMap<String, u32>,
    media: &'a crate::media_cache::State,
    packs: &'a [EmojiPack],
    typing: Element<'a, Message>,
    followers: Element<'a, Message>,
) -> Element<'a, Message> {
    // Everything above the input renders inside always-present slots so
    // the input never shifts tree position (which would drop its focus) —
    // see `theme::slot`. The mention list is the critical one: it appears
    // and disappears *while the user is typing*.
    let error_slot = crate::theme::slot(
        state.error.as_ref().map(|error| text(error.clone()).style(text::danger).size(13).into()),
    );

    let reply_slot = crate::theme::slot(state.replying_to.as_ref().map(|reply| {
        let mut banner = row![].spacing(8).align_y(iced::Center);
        banner = banner
            .push(text(format!("↩ Replying to {}", reply.sender)).size(12).style(text::primary));
        if let Some(thumb) = reply
            .image_url
            .as_deref()
            .and_then(|url| crate::media_cache::mxc_visual(media, url, 28, Some(28)))
        {
            banner = banner.push(thumb);
        }
        banner = banner
            .push(text(reply.snippet.clone()).size(12).style(text::secondary).width(Length::Fill))
            .push(
                button(text("×").size(13))
                    .on_press(Message::CancelReply)
                    .style(crate::theme::ghost_button)
                    .padding([0, 6]),
            );
        container(banner).padding([4, 8]).style(crate::theme::panel).into()
    }));

    let mention_slot = crate::theme::slot(active_mention_query(&state.body).and_then(|query| {
        let query_lower = query.to_lowercase();
        let matches: Vec<&RoomMember> = state
            .member_candidates
            .iter()
            .zip(state.member_candidates_lower.iter())
            .filter(|(_, lower)| lower.starts_with(&query_lower))
            .map(|(member, _)| member)
            .take(6)
            .collect();

        if matches.is_empty() {
            return None;
        }
        let mut list = column![].spacing(2);
        for member in matches {
            list = list.push(
                button(text(member.display_name.clone()).size(13))
                    .on_press(Message::MentionCandidateClicked(
                        member.user_id.clone(),
                        member.display_name.clone(),
                    ))
                    .width(Length::Fill)
                    .style(button::text),
            );
        }
        Some(container(list).padding(4).into())
    }));

    let picker_slot = crate::theme::slot(
        state
            .show_emoji_picker
            .then(|| emoji_picker(emoji_usage, media, packs)),
    );

    let mut col = column![error_slot, reply_slot, mention_slot, picker_slot].spacing(6);

    let input: Element<'_, Message> = if state.show_preview {
        container(
            markdown::view(
                &state.preview_items,
                markdown::Settings::default(),
                markdown::Style::from_palette(iced::Theme::Light.palette()),
            )
            .map(Message::LinkClicked),
        )
        .padding(8)
        .width(Length::Fill)
        .into()
    } else {
        text_input("Message... (@mention, markdown supported)", &state.body)
            .on_input(Message::BodyChanged)
            .on_submit(Message::Send)
            .padding(8)
            .into()
    };

    // Send is the one primary action; everything else stays quiet. The
    // typing indicator and follower avatars share this row, clustered on
    // the right next to Send.
    let toolbar = row![
        button(text(if state.show_preview { "Edit" } else { "Preview" }).size(13))
            .on_press(Message::TogglePreview(!state.show_preview))
            .style(crate::theme::ghost_button)
            .padding(6),
        button(text("Emoji").size(13))
            .on_press(Message::ToggleEmojiPicker)
            .style(crate::theme::ghost_button)
            .padding(6),
        button(text("Attach").size(13))
            .on_press(Message::PickAttachment)
            .style(crate::theme::ghost_button)
            .padding(6),
        container(typing).width(Length::Fill).align_x(iced::Right),
        followers,
        button(text("Send").size(13)).on_press(Message::Send).padding([6, 14]),
    ]
    .spacing(6)
    .align_y(iced::Center);

    col = col.push(input);
    col = col.push(toolbar);

    container(col).padding(8).width(Length::Fill).into()
}

fn emoji_picker<'a>(
    emoji_usage: &'a HashMap<String, u32>,
    media: &'a crate::media_cache::State,
    packs: &'a [EmojiPack],
) -> Element<'a, Message> {
    crate::emoji_picker::view(
        emoji_usage,
        media,
        packs,
        Message::EmojiPicked,
        |emoji| Message::CustomEmojiPicked {
            shortcode: emoji.shortcode.clone(),
            mxc_url: emoji.mxc_url.clone(),
        },
    )
}
