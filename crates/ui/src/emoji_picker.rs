//! The full emoji picker: one vertically scrolling list with a section per
//! custom pack and per unicode group, each under its own header — the
//! Element/Discord layout. A "Frequently used" section built from the
//! user's own usage history sits on top. Shared between the composer's
//! Emoji button and the timeline's reaction picker. Generic over the
//! host's message type: the host supplies constructors for "unicode
//! picked" / "custom picked" and this module never produces a message of
//! its own.

use std::collections::HashMap;

use client_core::events::{CustomEmoji, EmojiPack};
use iced::widget::{button, column, container, row, scrollable, text};
use iced::{Element, Length};

/// This client's skin-tone preference, applied everywhere an emoji with
/// tone variants is shown or sent: light, falling back to the neutral
/// yellow for emoji without tone variants.
pub fn preferred_tone(emoji: &'static emojis::Emoji) -> &'static emojis::Emoji {
    emoji.with_skin_tone(emojis::SkinTone::Light).unwrap_or(emoji)
}

/// Tone-resolved emoji per group, computed once — the data is entirely
/// `'static`, and resolving it (~1900 `with_skin_tone` lookups) otherwise
/// re-ran on every view call for as long as the picker was open.
fn toned_groups() -> &'static [(emojis::Group, Vec<&'static emojis::Emoji>)] {
    static GROUPS: std::sync::OnceLock<Vec<(emojis::Group, Vec<&'static emojis::Emoji>)>> =
        std::sync::OnceLock::new();
    GROUPS.get_or_init(|| {
        emojis::Group::iter()
            .map(|group| (group, group.emojis().map(preferred_tone).collect()))
            .collect()
    })
}

/// Every glyph across all groups (tone preference applied) — fetched when
/// the picker opens, since the whole list is visible by scrolling. (Cached
/// on disk after the first time; the font-glyph fallback covers anything
/// still in flight.)
pub fn all_unicode_glyphs() -> Vec<String> {
    toned_groups()
        .iter()
        .flat_map(|(_, emojis)| emojis.iter().map(|e| e.as_str().to_string()))
        .collect()
}

/// Renders a unicode emoji as its fetched Twemoji SVG, falling back to the
/// system emoji font while the fetch is in flight (or if it failed) —
/// instant feedback instead of a blank button.
pub fn emoji_visual<'a, M: 'a>(
    media: &'a crate::media_cache::State,
    emoji: &'a str,
) -> Element<'a, M> {
    match media.emoji.get(emoji) {
        Some(handle) => iced::widget::svg(handle.clone()).width(20).height(20).into(),
        None => text(emoji).size(18).font(crate::theme::EMOJI_FONT).into(),
    }
}

const PER_ROW: usize = 10;
/// Every emoji sits centered in an identical fixed cell so rows and
/// columns line up regardless of the artwork's own size. Sized with
/// several px of slack around the largest visual (24px custom emoji,
/// ~22px font-fallback glyphs) — tight bounds visibly shave glyph edges,
/// since text rendering clips at the widget's box.
const CELL: u16 = 36;

/// How many history entries the "Frequently used" section shows.
const FREQUENT_LIMIT: usize = 20;

pub fn view<'a, M: Clone + 'a>(
    usage: &'a HashMap<String, u32>,
    media: &'a crate::media_cache::State,
    packs: &'a [EmojiPack],
    on_unicode: impl Fn(&'static str) -> M + 'a,
    on_custom: impl Fn(&'a CustomEmoji) -> M + 'a,
) -> Element<'a, M> {
    let cell = |content: Element<'a, M>, message: M| -> Element<'a, M> {
        button(
            container(content)
                .width(Length::Fixed(CELL as f32))
                .height(Length::Fixed(CELL as f32))
                .center_x(Length::Fixed(CELL as f32))
                .center_y(Length::Fixed(CELL as f32)),
        )
        .on_press(message)
        .style(crate::theme::ghost_button)
        .padding(0)
        .into()
    };
    let custom_visual = |emoji: &'a CustomEmoji| -> Element<'a, M> {
        crate::media_cache::mxc_visual(media, &emoji.mxc_url, 24, Some(24))
            .unwrap_or_else(|| text(format!(":{}:", emoji.shortcode)).size(10).into())
    };

    let mut sections = column![].spacing(8);

    // Frequently used: the user's own history, most-used first. Unicode
    // keys resolve back to their 'static glyph via the emojis crate;
    // custom keys are mxc URLs resolved against the loaded packs.
    let mut frequent: Vec<(&String, &u32)> = usage.iter().collect();
    frequent.sort_by(|a, b| b.1.cmp(a.1));
    let mut frequent_cells: Vec<Element<'a, M>> = Vec::new();
    for (key, _count) in frequent {
        if frequent_cells.len() >= FREQUENT_LIMIT {
            break;
        }
        if key.starts_with("mxc://") {
            if let Some(emoji) = packs.iter().flat_map(|p| &p.emojis).find(|e| &e.mxc_url == key) {
                frequent_cells.push(cell(custom_visual(emoji), on_custom(emoji)));
            }
        } else if let Some(emoji) = emojis::get(key) {
            let glyph = emoji.as_str();
            frequent_cells.push(cell(emoji_visual(media, glyph), on_unicode(glyph)));
        }
    }
    if !frequent_cells.is_empty() {
        sections = sections.push(section_header("Frequently used"));
        let mut grid = column![].spacing(2);
        let mut cells = frequent_cells.into_iter();
        loop {
            let chunk: Vec<Element<'a, M>> = cells.by_ref().take(PER_ROW).collect();
            if chunk.is_empty() {
                break;
            }
            let mut grid_row = row![].spacing(2);
            for c in chunk {
                grid_row = grid_row.push(c);
            }
            grid = grid.push(grid_row);
        }
        sections = sections.push(grid);
    }

    // Custom packs, biggest first (puts the room's main pack — HQ — on
    // top, one-off packs after).
    let mut sorted_packs: Vec<&EmojiPack> = packs.iter().collect();
    sorted_packs.sort_by(|a, b| b.emojis.len().cmp(&a.emojis.len()).then(a.name.cmp(&b.name)));
    for pack in sorted_packs {
        sections = sections.push(section_header(&pack.name));
        let mut grid = column![].spacing(2);
        for chunk in pack.emojis.chunks(PER_ROW) {
            let mut grid_row = row![].spacing(2);
            for emoji in chunk {
                grid_row = grid_row.push(cell(custom_visual(emoji), on_custom(emoji)));
            }
            grid = grid.push(grid_row);
        }
        sections = sections.push(grid);
    }

    for (group, group_emojis) in toned_groups() {
        sections = sections.push(section_header(group_label(*group)));
        let mut grid = column![].spacing(2);
        for chunk in group_emojis.chunks(PER_ROW) {
            let mut grid_row = row![].spacing(2);
            for emoji in chunk {
                grid_row = grid_row
                    .push(cell(emoji_visual(media, emoji.as_str()), on_unicode(emoji.as_str())));
            }
            grid = grid.push(grid_row);
        }
        sections = sections.push(grid);
    }

    container(
        scrollable(sections)
            .height(Length::Fixed(320.0))
            .direction(iced::widget::scrollable::Direction::Vertical(
                iced::widget::scrollable::Scrollbar::new().width(6).scroller_width(6),
            ))
            .style(crate::theme::thin_scrollbar),
    )
    .padding(8)
    .style(crate::theme::panel)
    .into()
}

fn section_header<M>(label: &str) -> Element<'_, M> {
    text(label).size(12).font(crate::theme::SEMIBOLD_FONT).into()
}

fn group_label(group: emojis::Group) -> &'static str {
    match group {
        emojis::Group::SmileysAndEmotion => "Smileys",
        emojis::Group::PeopleAndBody => "People",
        emojis::Group::AnimalsAndNature => "Animals",
        emojis::Group::FoodAndDrink => "Food",
        emojis::Group::TravelAndPlaces => "Travel",
        emojis::Group::Activities => "Activities",
        emojis::Group::Objects => "Objects",
        emojis::Group::Symbols => "Symbols",
        emojis::Group::Flags => "Flags",
    }
}
