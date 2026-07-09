//! Win11-approximated `iced::Theme`. This is a palette approximation, not a
//! pixel-match — iced doesn't composite with DWM backdrop materials
//! (Mica/acrylic), so this only tunes flat colors to sit close to Windows'
//! light/dark surface and accent conventions.

use std::sync::atomic::{AtomicU32, Ordering};

use iced::border;
use iced::widget::{button, container, text};
use iced::{Color, Element, Font, Theme};

/// iced's default font family (Fira Sans, falling back to whatever generic
/// sans-serif fontdb finds) has no emoji glyphs, so unicode emoji render as
/// tofu boxes. `fontdb` does index all installed system fonts though
/// (confirmed: `cosmic_text::FontSystem::new_with_fonts` calls
/// `db.load_system_fonts()` before adding iced's bundled fonts) — Windows
/// ships "Segoe UI Emoji" by default, so requesting it by name for
/// emoji-only text (reaction pills, emoji pickers) renders real glyphs
/// without needing to bundle a font ourselves.
pub const EMOJI_FONT: Font = Font::with_name("Segoe UI Emoji");

/// Heavier weight for names/titles (ships with Windows; fontdb resolves it
/// from the installed system fonts, falling back silently if absent).
pub const SEMIBOLD_FONT: Font = Font::with_name("Segoe UI Semibold");

/// Windows' system icon font (Windows 11; outline-style UI glyphs at
/// documented PUA codepoints — e.g. U+E76E Emoji2, U+E97A Reply, U+E70F
/// Edit, U+E74D Delete). The right source for monochrome action icons on a
/// Windows-first app; iced's bundled text font has no such glyphs.
pub const ICON_FONT: Font = Font::with_name("Segoe Fluent Icons");

pub mod icon {
    pub const REACT: &str = "\u{E76E}";
    pub const REPLY: &str = "\u{E97A}";
    pub const EDIT: &str = "\u{E70F}";
    pub const DELETE: &str = "\u{E74D}";
    pub const PLAY: &str = "\u{E768}";
    pub const CLOSE: &str = "\u{E8BB}";
    /// U+E717 Phone — call banner / start-call button.
    pub const CALL: &str = "\u{E717}";
    /// U+E723 Attach (paperclip) — composer attachment picker.
    pub const ATTACH: &str = "\u{E723}";
    /// U+E724 Send — composer send button.
    pub const SEND: &str = "\u{E724}";
    /// U+E721 Search — timeline message search.
    pub const SEARCH: &str = "\u{E721}";
    /// U+E716 People — room member panel toggle.
    pub const MEMBERS: &str = "\u{E716}";
    /// U+E7ED Ringer (bell) — room notification-mode menu.
    pub const NOTIFY: &str = "\u{E7ED}";
    /// U+E708 QuietHours (bell + moon) — shown when the room is muted.
    pub const NOTIFY_MUTED: &str = "\u{E708}";
    /// U+F4AA Sticker2 — composer sticker picker.
    pub const STICKER: &str = "\u{F4AA}";
    /// U+E72B Back — space explorer's up-one-level button.
    pub const BACK: &str = "\u{E72B}";
    /// U+E76C ChevronRight — drill-in affordance on space rows.
    pub const CHEVRON_RIGHT: &str = "\u{E76C}";
    /// U+E710 Add (plus) — new-room / new-DM buttons on the sidebar section
    /// headers.
    pub const ADD: &str = "\u{E710}";
    /// U+E896 Download — save the open lightbox image to disk.
    pub const DOWNLOAD: &str = "\u{E896}";
}

/// An [`icon`] glyph rendered in the Windows icon font. Central so callers
/// don't repeat the `.font(ICON_FONT)` wiring at every button. Generic over
/// the borrow so the returned `Text` (invariant in its lifetime) unifies with
/// whatever element lifetime the call site needs.
pub fn icon_text<'a>(glyph: &'a str, size: u16) -> text::Text<'a> {
    text(glyph).font(ICON_FONT).size(f32::from(size))
}

/// `text(...)` for strings the app doesn't author — message bodies, display
/// names, room names, link-preview/tweet/video titles, topics. Uses advanced
/// shaping so cosmic-text's per-glyph font fallback actually runs: iced's
/// default `Shaping::Basic` does *no fallback at all* ("will not try to find
/// missing glyphs in your system fonts"), which renders any script the
/// default font lacks — all of CJK, for one — as tofu boxes, no matter what
/// fonts are installed or bundled (the bundled Noto CJK net in app/main.rs
/// is only reachable through this). Plain `text()` remains right for
/// app-authored ASCII labels and provably-ASCII hot paths.
pub fn remote_text<'a>(fragment: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(fragment).shaping(text::Shaping::Advanced)
}

/// Corner radius for `ghost_button`/`overlay_button`/`selected_ghost_button`,
/// synced once per frame from `view()` via `sync_corner_radius`. iced's
/// `Palette`/`Extended` have no "radius" slot to piggyback on the way colors
/// do, and these functions are referenced as bare function pointers
/// (`.style(theme::ghost_button)`) at many call sites across the app — a
/// signature change would mean touching every one of those for a single
/// float, so this is synced through a small global instead.
static CORNER_RADIUS_BITS: AtomicU32 = AtomicU32::new(6.0f32.to_bits());

pub fn sync_corner_radius(radius: f32) {
    CORNER_RADIUS_BITS.store(radius.to_bits(), Ordering::Relaxed);
}

fn corner_radius() -> f32 {
    f32::from_bits(CORNER_RADIUS_BITS.load(Ordering::Relaxed))
}

/// Emote (`/me` action) text color. Same rationale as `corner_radius` above:
/// the timeline's emote render needs it, but iced's `Palette` has no slot for
/// a bespoke color, so it's synced once per frame from `view()` via
/// `sync_emote_color`. Packed RGBA8 (`0xRRGGBBAA`); seeded to the dark
/// preset's emote color so it reads sanely before the first sync.
static EMOTE_COLOR_BITS: AtomicU32 = AtomicU32::new(0xC3_9B_E8_FF);

pub fn sync_emote_color(color: Color) {
    let [r, g, b, a] = color.into_rgba8();
    EMOTE_COLOR_BITS.store(u32::from_be_bytes([r, g, b, a]), Ordering::Relaxed);
}

pub fn emote_color() -> Color {
    let [r, g, b, a] = EMOTE_COLOR_BITS.load(Ordering::Relaxed).to_be_bytes();
    Color::from_rgba8(r, g, b, f32::from(a) / 255.0)
}

/// Quiet button: no chrome at rest, soft rounded highlight on hover.
/// Replaces iced's `button::primary` default (a solid accent-blue slab)
/// everywhere a control shouldn't shout — room rows, toolbars, pickers.
pub fn ghost_button(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let base = button::Style {
        text_color: palette.background.base.text,
        border: border::rounded(corner_radius()),
        ..button::Style::default()
    };
    match status {
        button::Status::Hovered | button::Status::Pressed => button::Style {
            background: Some(palette.background.weak.color.into()),
            ..base
        },
        button::Status::Disabled => button::Style {
            text_color: palette.background.base.text.scale_alpha(0.4),
            ..base
        },
        button::Status::Active => base,
    }
}

/// Like [`ghost_button`] but always white-on-dark — for controls drawn over
/// the dimmed lightbox/player backdrop, where the light theme's normal text
/// color would vanish.
pub fn overlay_button(_theme: &Theme, status: button::Status) -> button::Style {
    let base = button::Style {
        text_color: Color::WHITE,
        border: border::rounded(corner_radius()),
        ..button::Style::default()
    };
    match status {
        button::Status::Hovered | button::Status::Pressed => button::Style {
            background: Some(Color::from_rgba(1.0, 1.0, 1.0, 0.12).into()),
            ..base
        },
        button::Status::Disabled => button::Style {
            text_color: Color::WHITE.scale_alpha(0.4),
            ..base
        },
        button::Status::Active => base,
    }
}

/// Like [`ghost_button`] but accent-tinted at rest — for the selected item
/// in a list (e.g. the open room's row).
pub fn selected_ghost_button(theme: &Theme, _status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    button::Style {
        text_color: palette.primary.weak.text,
        background: Some(palette.primary.weak.color.into()),
        border: border::rounded(corner_radius()),
        ..button::Style::default()
    }
}

/// Fully-rounded accent badge (unread counts).
pub fn pill_badge(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.primary.strong.color.into()),
        text_color: Some(palette.primary.strong.text),
        border: border::rounded(999),
        ..container::Style::default()
    }
}

/// Slightly-recessed panel background (sidebar, header bar, pickers).
pub fn panel(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.weak.color.into()),
        ..container::Style::default()
    }
}

/// Solid bordered card for layers that float over the timeline (reaction
/// picker, composer emoji picker) — unlike `panel` it needs a border, since
/// it sits on top of other content instead of in the layout flow.
pub fn floating_panel(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.weak.color.into()),
        border: iced::Border {
            color: palette.background.strong.color,
            width: 1.0,
            radius: 10.into(),
        },
        ..container::Style::default()
    }
}

/// Hyperlink-style button: accent-colored text, no chrome, blends into a
/// text row.
pub fn link_button(theme: &Theme, status: button::Status) -> button::Style {
    let palette = theme.extended_palette();
    let color = match status {
        button::Status::Hovered | button::Status::Pressed => palette.primary.base.color,
        _ => palette.primary.strong.color,
    };
    button::Style { text_color: color, ..button::Style::default() }
}

/// Slim rounded scrollbar (iced's default is a bare gray square floating
/// in space — it reads as a mystery button, not a scrollbar).
pub fn thin_scrollbar(
    theme: &Theme,
    status: iced::widget::scrollable::Status,
) -> iced::widget::scrollable::Style {
    use iced::widget::scrollable::{Rail, Scroller, Status, Style};
    let palette = theme.extended_palette();
    let engaged = matches!(status, Status::Hovered { .. } | Status::Dragged { .. });
    let rail = Rail {
        background: Some(palette.background.weak.color.scale_alpha(0.5).into()),
        border: border::rounded(4),
        scroller: Scroller {
            background: if engaged {
                palette.background.strong.color
            } else {
                palette.background.strong.color.scale_alpha(0.7)
            }
            .into(),
            border: border::rounded(4),
        },
    };
    Style {
        container: container::Style::default(),
        vertical_rail: rail,
        horizontal_rail: rail,
        gap: None,
        // The middle-click autoscroll overlay (0.14) — rarely seen; give it the
        // rail's own colors rather than leave it unstyled.
        auto_scroll: iced::widget::scrollable::AutoScroll {
            background: palette.background.strong.color.into(),
            border: border::rounded(4),
            shadow: Default::default(),
            icon: palette.background.strong.text,
        },
    }
}

/// A layout slot that exists whether or not it has content. iced diffs
/// widget state by tree position, so conditionally inserting an element
/// (typing indicator, error line, banner) shifts every later sibling and
/// wipes their state — most painfully, a focused `text_input` loses focus
/// whenever the element above it blinks in or out. Keeping a constant
/// `container` node in the slot makes the change purely internal.
pub fn slot<'a, M: 'a>(content: Option<Element<'a, M>>) -> Element<'a, M> {
    match content {
        Some(content) => container(content).into(),
        None => container(iced::widget::Space::new()).into(),
    }
}
