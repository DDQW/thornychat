//! "Explore space" overlay — the standard Matrix space directory (what
//! Cinny calls Explore): browses a space's children via the hierarchy API,
//! including rooms the account hasn't joined. Subspaces drill down one
//! level at a time onto a stack (Back pops, already-fetched levels reshow
//! instantly); joined rooms open in the timeline; unjoined public or
//! restricted ones offer Join. Knock/invite-only rooms are listed but
//! labeled — no knock flow yet.

use std::collections::HashSet;

use client_core::commands::RequestId;
use client_core::events::{SpaceChildSummary, SpaceJoinRule};
use iced::widget::{button, center, column, container, mouse_area, opaque, row, scrollable, text};
use iced::{Element, Length};

/// One level of the drill-down: a space whose children are (being) listed.
#[derive(Debug, Clone)]
pub struct Level {
    pub space_id: String,
    /// Display name for the header (from the sidebar summary or the child
    /// row that was drilled into).
    pub name: String,
    pub children: Vec<SpaceChildSummary>,
    /// Pagination token for the next page (`None` once the last page landed).
    pub next_batch: Option<String>,
    /// In-flight hierarchy fetch (first page or "load more").
    pub pending_request: Option<RequestId>,
    pub error: Option<String>,
}

impl Level {
    pub fn loading(space_id: String, name: String, request_id: RequestId) -> Self {
        Self {
            space_id,
            name,
            children: Vec::new(),
            next_batch: None,
            pending_request: Some(request_id),
            error: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct State {
    /// Drill-down stack; last entry is the visible level. Never empty while
    /// the overlay is open (popping the last level closes it).
    pub stack: Vec<Level>,
    /// The in-flight join, if any: (request, room it was for). One at a
    /// time — join buttons render inert while set.
    pub pending_join: Option<(RequestId, String)>,
    /// Rooms joined from this overlay — flips their rows to "joined"
    /// immediately instead of waiting for a refetch of the level.
    pub joined: HashSet<String>,
    /// The last failed join: (room id, error), rendered inside that room's
    /// row. Cleared when another join starts.
    pub join_error: Option<(String, String)>,
}

impl State {
    /// Opens the explorer at `space_id`, with its first page already
    /// requested under `request_id`.
    pub fn open(space_id: String, name: String, request_id: RequestId) -> Self {
        Self {
            stack: vec![Level::loading(space_id, name, request_id)],
            pending_join: None,
            joined: HashSet::new(),
            join_error: None,
        }
    }

    /// Absorbs a fetched hierarchy page. Pages append (the `from` token
    /// only walks forward); a page whose request no longer matches any
    /// level (level popped, retried, or overlay reopened) is dropped.
    pub fn apply_page(
        &mut self,
        request_id: RequestId,
        space_id: &str,
        children: Vec<SpaceChildSummary>,
        next_batch: Option<String>,
    ) {
        let level = self
            .stack
            .iter_mut()
            .find(|l| l.pending_request == Some(request_id) && l.space_id == space_id);
        if let Some(level) = level {
            level.pending_request = None;
            level.error = None;
            level.children.extend(children);
            level.next_batch = next_batch;
        }
    }

    /// Claims a correlated command success (a join finishing). Returns
    /// false when the request wasn't the explorer's, so the caller keeps
    /// routing it elsewhere.
    pub fn handle_success(&mut self, request_id: RequestId) -> bool {
        if self.pending_join.as_ref().is_some_and(|(id, _)| *id == request_id) {
            if let Some((_, room_id)) = self.pending_join.take() {
                self.joined.insert(room_id);
            }
            return true;
        }
        false
    }

    /// Claims a correlated command failure (a join or a page fetch).
    /// Returns false when the request wasn't the explorer's.
    pub fn handle_failure(&mut self, request_id: RequestId, error: &str) -> bool {
        if self.pending_join.as_ref().is_some_and(|(id, _)| *id == request_id) {
            if let Some((_, room_id)) = self.pending_join.take() {
                self.join_error = Some((room_id, error.to_string()));
            }
            return true;
        }
        if let Some(level) =
            self.stack.iter_mut().find(|l| l.pending_request == Some(request_id))
        {
            level.pending_request = None;
            level.error = Some(error.to_string());
            return true;
        }
        false
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Close,
    /// Pop one level (closes the overlay from the root level).
    Back,
    /// Drill into a subspace listed at the current level.
    EnterSpace { space_id: String, name: String },
    /// Join an unjoined child (room or subspace).
    Join { room_id: String, via: Vec<String> },
    /// Open an already-joined room in the timeline (closes the overlay).
    Open(String),
    /// Fetch the next page of the current level.
    LoadMore,
    /// Refetch the current level from scratch after a failed fetch.
    Retry,
}

pub fn view<'a>(
    state: &'a State,
    media: &'a crate::media_cache::State,
) -> Element<'a, Message> {
    // The stack is never left empty by update(), but don't panic on it.
    let Some(level) = state.stack.last() else {
        return iced::widget::Space::new(0, 0).into();
    };

    let backdrop = |_theme: &iced::Theme| iced::widget::container::Style {
        background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.5).into()),
        ..iced::widget::container::Style::default()
    };

    let mut header = row![].spacing(8).align_y(iced::Center);
    if state.stack.len() > 1 {
        header = header.push(
            button(text(crate::theme::icon::BACK).font(crate::theme::ICON_FONT).size(12))
                .on_press(Message::Back)
                .style(crate::theme::ghost_button)
                .padding([4, 8]),
        );
    }
    header = header.push(
        column![
            text(level.name.clone()).size(16).font(crate::theme::SEMIBOLD_FONT),
            text("Browse this space's rooms — click to open, or join.")
                .size(11)
                .style(text::secondary),
        ]
        .spacing(2)
        .width(Length::Fill),
    );
    header = header.push(
        button(text(crate::theme::icon::CLOSE).font(crate::theme::ICON_FONT).size(12))
            .on_press(Message::Close)
            .style(crate::theme::ghost_button)
            .padding([4, 8]),
    );

    let mut list = column![].spacing(2);
    if let Some(error) = &level.error {
        list = list.push(
            column![
                text(format!("Couldn't load this space: {error}")).size(12).style(text::danger),
                button(text("Retry").size(12)).on_press(Message::Retry).padding([6, 12]),
            ]
            .spacing(10),
        );
    } else {
        for child in &level.children {
            list = list.push(child_row(state, child, media));
        }
        if level.pending_request.is_some() {
            list = list.push(
                container(text("Loading...").size(12).style(text::secondary)).padding([8, 4]),
            );
        } else if level.children.is_empty() {
            list = list.push(
                container(text("This space lists no rooms.").size(12).style(text::secondary))
                    .padding([8, 4]),
            );
        } else if level.next_batch.is_some() {
            list = list.push(
                container(
                    button(text("Load more").size(12))
                        .on_press(Message::LoadMore)
                        .style(crate::theme::ghost_button)
                        .padding([6, 12]),
                )
                .padding([6, 0]),
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
        column![header, body]
            .spacing(12)
            .width(Length::Fixed(560.0))
            .height(Length::Fixed(620.0)),
    )
    .padding(16)
    .style(crate::theme::panel);

    // Backdrop click closes; everything inside the card consumes its own
    // clicks first (same nesting as the settings overlay).
    opaque(mouse_area(center(card).style(backdrop)).on_press(Message::Close))
}

/// A one-line, length-capped rendering of a room topic (topics are often
/// multi-line essays; the row gives them one secondary line).
fn topic_snippet(topic: &str) -> String {
    let one_line = topic.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > 90 {
        let mut cut: String = one_line.chars().take(90).collect();
        cut.push('…');
        cut
    } else {
        one_line
    }
}

fn child_row<'a>(
    state: &'a State,
    child: &'a SpaceChildSummary,
    media: &'a crate::media_cache::State,
) -> Element<'a, Message> {
    let display_name = child
        .name
        .clone()
        .or_else(|| child.canonical_alias.clone())
        .unwrap_or_else(|| child.room_id.clone());
    let joined = child.joined || state.joined.contains(&child.room_id);
    let join_pending =
        state.pending_join.as_ref().is_some_and(|(_, id)| id == &child.room_id);

    let mut name_line = row![text(display_name.clone()).size(13)].spacing(6).align_y(iced::Center);
    if child.is_space {
        name_line = name_line.push(
            container(text("Space").size(10)).padding([1, 6]).style(crate::theme::pill_badge),
        );
    }

    let members = format!(
        "{} member{}",
        child.num_joined_members,
        if child.num_joined_members == 1 { "" } else { "s" }
    );
    // One secondary line: member count, then the topic (or the alias when
    // it isn't already standing in for a missing name).
    let detail = child
        .topic
        .as_deref()
        .map(topic_snippet)
        .filter(|s| !s.is_empty())
        .or_else(|| child.canonical_alias.clone().filter(|alias| *alias != display_name));
    let secondary = match detail {
        Some(detail) => format!("{members} · {detail}"),
        None => members,
    };

    let mut info = column![name_line, text(secondary).size(11).style(text::secondary)]
        .spacing(2)
        .width(Length::Fill);
    if let Some((failed_room, error)) = &state.join_error {
        if failed_room == &child.room_id {
            info = info.push(text(format!("Join failed: {error}")).size(11).style(text::danger));
        }
    }

    let trailing: Element<'a, Message> = if join_pending {
        text("Joining...").size(11).style(text::secondary).into()
    } else if joined {
        text("Joined").size(11).style(text::secondary).into()
    } else {
        match child.join_rule {
            SpaceJoinRule::Public | SpaceJoinRule::Restricted => {
                let join = button(text("Join").size(12)).padding([4, 12]);
                // Inert (but still labeled) while another join is running.
                if state.pending_join.is_none() {
                    join.on_press(Message::Join {
                        room_id: child.room_id.clone(),
                        via: child.via.clone(),
                    })
                    .into()
                } else {
                    join.into()
                }
            }
            SpaceJoinRule::Knock => text("By request").size(11).style(text::secondary).into(),
            SpaceJoinRule::InviteOnly => {
                text("Invite only").size(11).style(text::secondary).into()
            }
        }
    };

    let content = row![
        crate::media_cache::avatar(media, child.avatar_url.as_deref(), &display_name, 30),
        info,
        trailing,
    ]
    .spacing(10)
    .align_y(iced::Center);

    // Row interaction: subspaces drill in (their Join button, when shown,
    // consumes its own click first); joined rooms open; unjoined rooms are
    // inert rows whose only control is the Join button.
    if child.is_space {
        button(content)
            .on_press(Message::EnterSpace {
                space_id: child.room_id.clone(),
                name: display_name,
            })
            .style(crate::theme::ghost_button)
            .width(Length::Fill)
            .padding([6, 8])
            .into()
    } else if joined {
        button(content)
            .on_press(Message::Open(child.room_id.clone()))
            .style(crate::theme::ghost_button)
            .width(Length::Fill)
            .padding([6, 8])
            .into()
    } else {
        container(content).width(Length::Fill).padding([6, 8]).into()
    }
}
