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

/// Renders a unicode emoji as its fetched Twemoji SVG at `size`px, falling
/// back to the system emoji font while the fetch is in flight (or if it
/// failed) — instant feedback instead of a blank button.
pub fn emoji_visual<'a, M: 'a>(
    media: &'a crate::media_cache::State,
    emoji: &'a str,
    size: u16,
) -> Element<'a, M> {
    match media.emoji.get(emoji) {
        Some(handle) => iced::widget::svg(handle.clone()).width(size).height(size).into(),
        None => text(emoji).size(size.saturating_sub(2)).font(crate::theme::EMOJI_FONT).into(),
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
    // One pass over the packs instead of a linear scan per frequent mxc key
    // per view rebuild (this runs every frame while the picker is open).
    // entry().or_insert keeps the FIRST match on duplicate urls across
    // packs, matching the old find() semantics.
    let mut custom_by_url: HashMap<&str, &CustomEmoji> = HashMap::new();
    for e in packs.iter().flat_map(|p| &p.emojis).filter(|e| e.is_emoticon) {
        custom_by_url.entry(e.mxc_url.as_str()).or_insert(e);
    }
    let mut frequent_cells: Vec<Element<'a, M>> = Vec::new();
    for (key, _count) in frequent {
        if frequent_cells.len() >= FREQUENT_LIMIT {
            break;
        }
        if key.starts_with("mxc://") {
            if let Some(&emoji) = custom_by_url.get(key.as_str()) {
                frequent_cells.push(cell(custom_visual(emoji), on_custom(emoji)));
            }
        } else if let Some(emoji) = emojis::get(key) {
            let glyph = emoji.as_str();
            frequent_cells.push(cell(emoji_visual(media, glyph, 20), on_unicode(glyph)));
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
        // Only the emoticon-usage images belong in the emoji picker; a
        // sticker-only pack (e.g. HQ's stickers) contributes nothing here and
        // shouldn't render an empty header.
        let emoticons: Vec<&CustomEmoji> = pack.emojis.iter().filter(|e| e.is_emoticon).collect();
        if emoticons.is_empty() {
            continue;
        }
        sections = sections.push(section_header(&pack.name));
        let mut grid = column![].spacing(2);
        for chunk in emoticons.chunks(PER_ROW) {
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
                    .push(cell(emoji_visual(media, emoji.as_str(), 20), on_unicode(emoji.as_str())));
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

/// Sticker cells are bigger than emoji ones (stickers are pictures, not
/// glyphs): the image fits within `STICKER_IMG`, centered in a slightly
/// larger square button so the grid stays aligned.
const STICKER_CELL: u16 = 76;
const STICKER_IMG: u16 = 64;
const STICKERS_PER_ROW: usize = 5;

/// Scales `(w, h)` to fill a `max`×`max` box preserving aspect ratio
/// (upscaling small stickers so they don't sit tiny in a big cell). Falls
/// back to a square when a dimension is missing or zero.
fn fit_within(w: u32, h: u32, max: u16) -> (u16, u16) {
    if w == 0 || h == 0 {
        return (max, max);
    }
    let scale = (max as f32 / w as f32).min(max as f32 / h as f32);
    (
        ((w as f32 * scale).round() as u16).clamp(8, max),
        ((h as f32 * scale).round() as u16).clamp(8, max),
    )
}

/// The Sticker tab: a grid of sendable stickers — the grow-with-use
/// "Collected" set (harvested from `m.sticker` events seen in rooms), most
/// recent first, then each sticker pack the account has (HQ-style room packs
/// on top). Each cell fires `on_pick(url, body, width, height)`; the host
/// turns that into an `m.sticker` send.
pub fn sticker_view<'a, M: Clone + 'a>(
    media: &'a crate::media_cache::State,
    packs: &'a [EmojiPack],
    collection: &'a [crate::state::CollectedSticker],
    on_pick: impl Fn(&str, &str, Option<u32>, Option<u32>) -> M + 'a,
) -> Element<'a, M> {
    let cell = |content: Element<'a, M>, message: M| -> Element<'a, M> {
        button(
            container(content)
                .width(Length::Fixed(STICKER_CELL as f32))
                .height(Length::Fixed(STICKER_CELL as f32))
                .center_x(Length::Fixed(STICKER_CELL as f32))
                .center_y(Length::Fixed(STICKER_CELL as f32)),
        )
        .on_press(message)
        .style(crate::theme::ghost_button)
        .padding(0)
        .into()
    };
    let visual = |url: &str, w: Option<u32>, h: Option<u32>| -> Element<'a, M> {
        let (dw, dh) = match (w, h) {
            (Some(w), Some(h)) => fit_within(w, h, STICKER_IMG),
            _ => (STICKER_IMG, STICKER_IMG),
        };
        crate::media_cache::mxc_visual(media, url, dw, Some(dh)).unwrap_or_else(|| {
            // Reserve the box while the fetch is in flight so the grid doesn't
            // jump as images land.
            iced::widget::Space::new(Length::Fixed(dw as f32), Length::Fixed(dh as f32)).into()
        })
    };
    let grid = |cells: Vec<Element<'a, M>>| -> Element<'a, M> {
        let mut grid = column![].spacing(2);
        let mut it = cells.into_iter();
        loop {
            let chunk: Vec<Element<'a, M>> = it.by_ref().take(STICKERS_PER_ROW).collect();
            if chunk.is_empty() {
                break;
            }
            let mut r = row![].spacing(2);
            for c in chunk {
                r = r.push(c);
            }
            grid = grid.push(r);
        }
        grid.into()
    };

    let mut sections = column![].spacing(8);
    let mut any = false;

    if !collection.is_empty() {
        any = true;
        let cells: Vec<Element<'a, M>> = collection
            .iter()
            .map(|s| {
                cell(visual(&s.url, s.width, s.height), on_pick(&s.url, &s.body, s.width, s.height))
            })
            .collect();
        sections = sections.push(section_header("Collected"));
        sections = sections.push(grid(cells));
    }

    let mut sorted_packs: Vec<&EmojiPack> = packs.iter().collect();
    sorted_packs.sort_by(|a, b| b.emojis.len().cmp(&a.emojis.len()).then(a.name.cmp(&b.name)));
    for pack in sorted_packs {
        let cells: Vec<Element<'a, M>> = pack
            .emojis
            .iter()
            .filter(|e| e.is_sticker)
            .map(|e| {
                cell(
                    visual(&e.mxc_url, e.width, e.height),
                    on_pick(&e.mxc_url, &e.shortcode, e.width, e.height),
                )
            })
            .collect();
        if cells.is_empty() {
            continue;
        }
        any = true;
        sections = sections.push(section_header(&pack.name));
        sections = sections.push(grid(cells));
    }

    if !any {
        sections = sections.push(
            text("No stickers yet — stickers people send in your rooms show up here to reuse.")
                .size(12)
                .style(text::secondary),
        );
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
