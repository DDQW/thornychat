//! "New direct message" overlay — search the homeserver user directory by
//! name and click someone to open (or create) a DM with them, even if you
//! don't share a room with them yet. Opened by the "+" on the sidebar's
//! Direct messages header. Structurally mirrors the space explorer overlay
//! (backdrop + centered card + scrollable body); the new element here is a
//! search box whose input is debounced before it hits the server.
//!
//! Two independent guards keep the results honest under fast typing: a
//! `generation` counter (only the newest keystroke's debounce timer actually
//! fires a search) and `last_request_id` (only the newest in-flight search's
//! response is accepted, so a slow earlier response can't overwrite a faster
//! later one). All mutation lives in `update.rs::update_dm_search`, like the
//! space explorer.

use client_core::commands::RequestId;
use client_core::events::{friendly_user_id, UserSearchResult};
use iced::widget::{button, center, column, container, mouse_area, opaque, row, scrollable, text, text_input};
use iced::{Element, Length};

#[derive(Debug, Clone, Default)]
pub struct State {
    pub query: String,
    /// Bumped on every keystroke; a debounce timer only fires its search when
    /// its captured generation still matches (i.e. no newer keystroke since).
    pub generation: u64,
    /// The in-flight search's request id — results with a different id are
    /// stale (superseded) and ignored.
    pub last_request_id: Option<RequestId>,
    pub results: Vec<UserSearchResult>,
    pub pending: bool,
    pub error: Option<String>,
    /// The server truncated the result set — hint that a narrower query helps.
    pub limited: bool,
}

impl State {
    /// Claims the correlated `SearchUsers` success. The actual results arrive
    /// via `UserSearchResults` (handled in `update.rs`); this only confirms
    /// the request was ours. Returns false otherwise so the caller keeps
    /// routing the event elsewhere.
    pub fn handle_success(&mut self, request_id: RequestId) -> bool {
        self.last_request_id == Some(request_id)
    }

    /// Claims a correlated `SearchUsers` failure (bad query, network error),
    /// surfacing it in the overlay. Returns false when it wasn't ours.
    pub fn handle_failure(&mut self, request_id: RequestId, error: &str) -> bool {
        if self.last_request_id == Some(request_id) {
            self.pending = false;
            self.error = Some(error.to_string());
            self.results.clear();
            return true;
        }
        false
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Close,
    QueryChanged(String),
    /// A debounce timer elapsed for this generation — fire the search if it's
    /// still the newest one.
    Debounced(u64),
    /// Open (or create) a DM with this user id and close the overlay.
    ResultClicked(String),
}

pub fn view<'a>(state: &'a State, media: &'a crate::media_cache::State) -> Element<'a, Message> {
    let backdrop = |_theme: &iced::Theme| iced::widget::container::Style {
        background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.5).into()),
        ..iced::widget::container::Style::default()
    };

    let header = row![
        column![
            text("New direct message").size(16).font(crate::theme::SEMIBOLD_FONT),
            text("Search for someone by name to start a chat.").size(11).style(text::secondary),
        ]
        .spacing(2)
        .width(Length::Fill),
        button(text(crate::theme::icon::CLOSE).font(crate::theme::ICON_FONT).size(12))
            .on_press(Message::Close)
            .style(crate::theme::ghost_button)
            .padding([4, 8]),
    ]
    .spacing(8)
    .align_y(iced::Center);

    let search_input = text_input("Search by name or @user:server…", &state.query)
        .on_input(Message::QueryChanged)
        .padding(8)
        .size(14);

    let mut list = column![].spacing(2);
    if let Some(error) = &state.error {
        list = list.push(text(format!("Search failed: {error}")).size(12).style(text::danger));
    } else if state.pending {
        list =
            list.push(container(text("Searching…").size(12).style(text::secondary)).padding([8, 4]));
    } else if state.results.is_empty() {
        // Only nag about "no matches" once the user has actually typed —
        // an empty box shouldn't read as a failed search.
        if !state.query.trim().is_empty() {
            list = list.push(
                container(
                    text(
                        "No matches — the directory only covers users your homeserver \
                         knows about.",
                    )
                    .size(12)
                    .style(text::secondary),
                )
                .padding([8, 4]),
            );
        }
    } else {
        for result in &state.results {
            list = list.push(result_row(result, media));
        }
        if state.limited {
            list = list.push(
                container(
                    text("More matches exist — refine your search to narrow them down.")
                        .size(11)
                        .style(text::secondary),
                )
                .padding([8, 4]),
            );
        }
    }

    let body = scrollable(list)
        .height(Length::Fill)
        .direction(iced::widget::scrollable::Direction::Vertical(
            iced::widget::scrollable::Scrollbar::new().width(6).scroller_width(6),
        ))
        .style(crate::theme::thin_scrollbar);

    let card = container(
        column![header, search_input, body]
            .spacing(12)
            .width(Length::Fixed(560.0))
            .height(Length::Fixed(620.0)),
    )
    .padding(16)
    .style(crate::theme::panel);

    // Backdrop click closes; the card consumes its own clicks first (same
    // nesting as the settings / space-explorer overlays).
    opaque(mouse_area(center(card).style(backdrop)).on_press(Message::Close))
}

fn result_row<'a>(
    result: &'a UserSearchResult,
    media: &'a crate::media_cache::State,
) -> Element<'a, Message> {
    // A stranger with no display name still reads as a clean nick rather than
    // a raw mxid (and IRC ghosts lose their `irc_` prefix).
    let name = result
        .display_name
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| friendly_user_id(&result.user_id));
    let avatar = crate::media_cache::avatar(media, result.avatar_url.as_deref(), name, 30);
    let info = column![
        crate::theme::remote_text(name.to_string()).size(13),
        crate::theme::remote_text(result.user_id.clone()).size(11).style(text::secondary),
    ]
    .spacing(2)
    .width(Length::Fill);

    button(row![avatar, info].spacing(10).align_y(iced::Center))
        .on_press(Message::ResultClicked(result.user_id.clone()))
        .width(Length::Fill)
        .padding([6, 8])
        .style(crate::theme::ghost_button)
        .into()
}
