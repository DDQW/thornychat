//! The in-app user manual, reached from the "?" button on the Settings tab
//! strip. One long scrolling page grouped by topic — the Settings overlay
//! already wraps the tab body in a `scrollable`, so this just returns a tall
//! `Column`. Everything here is app-authored ASCII/punctuation, so plain
//! `text()` is correct: `remote_text`/advanced shaping is only for
//! server-authored strings that must fall back to bundled fonts (color emoji
//! would still tofu, so icons are described in words, not glyphs). Static and
//! stateless like `room_admin`, so the view is generic over the message type
//! and needs no `State`/`Message`.

use iced::widget::{row, text, Column};
use iced::{Element, Font, Length};

use crate::slash;
use crate::theme;

pub fn view<'a, M: 'a>() -> Element<'a, M> {
    let mut page: Column<'a, M> = Column::new().spacing(18);

    page = page
        .push(text("ThornyChat Manual").size(16).font(theme::SEMIBOLD_FONT))
        .push(note(
            "A guide to everything the app can do. Most actions are buttons, a \
             hover bar on each message, or right-click menus; the few things you \
             type are listed first. Reopen this page any time from the \"?\" \
             button in Settings.",
        ));

    let mut slash_rows = vec![para(
        "Type a command at the very start of a message. Anything \
         unrecognised is sent as normal text (so \"/method\" is safe), and a \
         leading \"//\" escapes to a literal slash. A command clears the box \
         on success; a mistake (a missing or malformed name) shows a hint \
         and keeps your text.",
    )];
    // Generated from the same catalog `slash::parse` matches against, so this
    // list and the parser's actual behaviour can't describe a command
    // differently (they did, three times, before this catalog existed).
    slash_rows.extend(slash::COMMANDS.iter().map(command_row));
    slash_rows.push(note(
        "The room and moderation commands act on the room you're viewing and \
         need the right permissions — if your power level is too low the \
         server refuses and the error shows above the box.",
    ));
    page = page.push(section("Slash commands", slash_rows));

    page = page.push(section(
        "Formatting with Markdown",
        vec![
            para(
                "Type Markdown in the message box and ThornyChat converts it when \
                 you send, so people on other Matrix clients (Element and the rest) \
                 see your messages formatted.",
            ),
            note(
                "Heads-up: ThornyChat's own timeline currently shows message text as \
                 plain text — it does not render formatting yet. So here you will \
                 see the literal characters you typed (the ** around bold, the \
                 backticks around code) even though other clients render them; other \
                 people's formatted messages likewise show as their plain-text \
                 version.",
            ),
            entry("**bold**", "Bold. Double underscores, __like this__, work too."),
            entry("*italic*", "Italic. Single underscores, _like this_, work too."),
            entry("~~strike~~", "Strikethrough."),
            entry("`code`", "Inline monospace, for code or literal text."),
            entry(
                "[text](url)",
                "A titled link. For plain links it is simpler to paste the bare URL \
                 — ThornyChat makes bare URLs clickable and shows a preview; the \
                 [text](url) form only formats on other clients.",
            ),
            note(
                "Headings (#), lists (-) and quotes (>) are Markdown too, but they \
                 need separate lines, and the message box is a single line (Enter \
                 sends), so in practice only the inline styles above are usable.",
            ),
        ],
    ));

    page = page.push(section(
        "Mentions & emoji",
        vec![
            entry(
                "@name",
                "Mention someone: type @ then part of their name and pick from the \
                 list that appears. They get highlighted and notified.",
            ),
            entry(
                ":shortcode:",
                "Insert a custom (pack) emoji by its :shortcode:. Easiest from the \
                 emoji button, which fills in the shortcode for you; the same custom \
                 emoji work in reactions, where they show as the real image.",
            ),
            note("The emoji and sticker pickers themselves are covered under Composer & attachments, below."),
        ],
    ));

    page = page.push(section(
        "Keyboard & mouse",
        vec![
            entry(
                "Enter",
                "Send the message. Also saves an inline edit and submits the \
                 rename, login, and recovery-key fields.",
            ),
            entry(
                "Ctrl+V",
                "Paste an image or file from the clipboard as an attachment. \
                 Plain text pastes into the box as usual.",
            ),
            entry("Esc", "Close the full-screen image viewer; also stops middle-click autoscroll."),
            entry(
                "Backspace",
                "Right after an autocorrect, undoes that correction and restores \
                 what you typed.",
            ),
            entry(
                "Middle-click",
                "In the message list, starts browser-style autoscroll — move the \
                 mouse to steer; any click or key stops it.",
            ),
            entry(
                "Wheel (image)",
                "Over an open image, zooms toward the cursor (1x-20x); drag to \
                 pan when zoomed in.",
            ),
            entry("Wheel (top)", "Scrolling near the top of a room loads older history automatically."),
            entry("Drag & drop", "Drop files onto the window to stage them as attachments."),
            entry("Right-click", "In the message box: Cut, Copy, Paste, Select all."),
            note(
                "The message box is a single line — there is no Shift+Enter \
                 newline, and search is the magnifier icon in the room header \
                 (not Ctrl+F).",
            ),
        ],
    ));

    page = page.push(section(
        "Messages",
        vec![
            para("Hover any message to reveal its action bar in the top-right corner:"),
            bullet("React — opens the emoji picker and adds a reaction."),
            bullet(
                "Reply — quotes the message above the box; the sent reply shows a \
                 clickable quote. Click a quote to jump to the original.",
            ),
            bullet("Edit — your own text messages only; edits inline, Enter saves."),
            bullet("Delete — your own messages only; asks to confirm, then shows \"(message removed)\"."),
            para(
                "Reactions: click React to add one, or click an existing reaction \
                 pill to add or remove yours. Hover a pill to see who reacted. \
                 Custom and :shortcode: emoji reactions show the real image.",
            ),
            para(
                "Reading: a red \"new messages\" divider marks where you left off. \
                 A room is marked read as soon as something arrives while you are \
                 scrolled to the newest message — scroll up to keep messages \
                 unread. Small avatars beside the box show who else is caught up, \
                 and \"... is typing\" appears just below it.",
            ),
            para(
                "Search: click the magnifier in the room header and type to filter \
                 the messages already loaded, with a live match count. Click it \
                 again to close.",
            ),
        ],
    ));

    page = page.push(section(
        "Rooms, spaces & direct messages",
        vec![
            para(
                "The sidebar lists your Direct messages and Rooms — click a row to \
                 open it. Spaces appear as bold headers with their rooms nested \
                 underneath; click a space header to open its explorer and browse \
                 or join child rooms.",
            ),
            bullet(
                "\"+\" on the Direct messages header — search the user directory by \
                 name or @user:server and start a DM.",
            ),
            bullet("\"+\" on the Rooms header — create a fresh private room."),
            bullet("Right-click a room, DM, or space row — Rename, Leave, or Forget it."),
            para(
                "Room header buttons: start or join a call, the notification bell \
                 (Default / All messages / Mentions only / Mute for that room), \
                 search, and the member-list toggle.",
            ),
            para(
                "In the member list: single-click to highlight that person's \
                 messages, double-click to open a DM, or right-click for Direct \
                 message, New room, or Highlight.",
            ),
        ],
    ));

    page = page.push(section(
        "Media & links",
        vec![
            bullet(
                "Images show inline — click one to open the full-screen viewer \
                 (wheel to zoom, drag to pan, a Download button to save; Esc, a \
                 margin click, or a double-click closes it).",
            ),
            bullet(
                "Videos — YouTube, Vimeo, and direct .mp4/.webm links, among \
                 others — show a card with a play button that plays inline, plus a \
                 \"Watch on...\" link.",
            ),
            bullet(
                "Links unfurl into preview cards, with richer cards for tweets and \
                 Steam pages. Link previews are on by default (see Privacy).",
            ),
            bullet("Stickers and animated GIF emotes are supported."),
        ],
    ));

    page = page.push(section(
        "Composer & attachments",
        vec![
            para(
                "Type a message and press Enter. Use the emoji button for the \
                 emoji picker and the sticker button for stickers — a sticker \
                 sends the moment you click it.",
            ),
            para(
                "Spell-check is unobtrusive: instead of underlines it shows a \
                 suggestion bar above the box for the last misspelled word, with \
                 an \"Add to dictionary\" option. Autocorrect (off by default) \
                 silently fixes a word when you end it with a space; Backspace \
                 right after undoes it.",
            ),
            para(
                "Attachments: click the paperclip, paste with Ctrl+V, or drag \
                 files in. They wait as chips above the box — nothing sends until \
                 you press Enter. Whatever you type becomes the caption on the \
                 first file. When replying, a banner shows above the box with a \
                 small x to cancel.",
            ),
        ],
    ));

    page = page.push(section(
        "Encryption & security",
        vec![
            para(
                "Encrypted rooms show a lock icon in the header, and messages can \
                 show a trust shield you can hover for details.",
            ),
            para(
                "The Encryption tab in Settings chooses whether new DMs and new \
                 rooms you create are encrypted — both are off by default.",
            ),
            para(
                "The Security tab in Settings is where you verify a device (by \
                 matching emoji) and set up or unlock key backup / recovery, so \
                 you can read old encrypted messages after signing in somewhere \
                 new. Key backup is opt-in — the app never nags you to turn it on.",
            ),
        ],
    ));

    page = page.push(section(
        "Settings",
        vec![
            para("Open Settings by clicking your profile row at the bottom of the sidebar. The tabs:"),
            bullet(
                "General — account info, Sign out, Start with Windows (off), Check \
                 spelling (on), Autocorrect (off), Show membership changes (on), \
                 and Copy log to clipboard.",
            ),
            bullet(
                "Privacy — Send read receipts (off; a private receipt is sent \
                 instead), Send typing notifications (off), Enable link previews \
                 (on).",
            ),
            bullet("Encryption — encrypt new DMs and new rooms (both off)."),
            bullet(
                "Notifications — your account-wide default for Direct messages and \
                 Group chats (All messages / Mentions only / Mute); per-room \
                 overrides live on the room header bell.",
            ),
            bullet(
                "Appearance — theme presets (Midnight dark by default, or Daylight \
                 light), ten editable colors, the UI font, UI scale, corner \
                 radius, and import / export / reset.",
            ),
            bullet("Security — device verification and key backup (see above)."),
        ],
    ));

    page = page.push(section(
        "Accounts & connection",
        vec![
            para(
                "Sign in by entering your homeserver first; the app then offers \
                 password and/or single sign-on, depending on what your server \
                 supports. Your session is stored securely in Windows Credential \
                 Manager, so you stay logged in between launches.",
            ),
            para(
                "You can run several accounts by passing a profile name as the \
                 first command-line argument — each keeps its own data. Switching \
                 profiles means relaunching the app.",
            ),
        ],
    ));

    page = page.push(section(
        "Not yet available",
        vec![
            note("A few things are not built yet, so you will not find them:"),
            bullet("Desktop notifications and a system-tray icon."),
            bullet(
                "Point-and-click member management and a full room-settings dialog \
                 — you can invite, kick, ban and set the topic with slash commands \
                 (above), but there are no buttons for them yet, and no power-level \
                 editor.",
            ),
            bullet("A threads panel — you will only see a reply count on the original message."),
            bullet("Audio and video in calls — call presence works, but no media yet."),
            bullet(
                "Downloading non-image files, and images, files, or polls inside \
                 encrypted rooms, which show as text placeholders.",
            ),
            bullet("Server-side search — in-room search covers only the messages already loaded."),
        ],
    ));

    page.into()
}

/// A titled block: a semibold heading followed by its rows.
fn section<'a, M: 'a>(title: &'a str, rows: Vec<Element<'a, M>>) -> Element<'a, M> {
    let mut col: Column<'a, M> = Column::new().spacing(6);
    col = col.push(text(title).size(14).font(theme::SEMIBOLD_FONT));
    for r in rows {
        col = col.push(r);
    }
    col.into()
}

/// A body paragraph (fills the width so it wraps rather than overflowing).
fn para<'a, M: 'a>(s: &'a str) -> Element<'a, M> {
    text(s).size(13).width(Length::Fill).into()
}

/// A dimmed secondary paragraph, for asides and defaults.
fn note<'a, M: 'a>(s: &'a str) -> Element<'a, M> {
    text(s).size(12).style(text::secondary).width(Length::Fill).into()
}

/// A hanging-bullet list item.
fn bullet<'a, M: 'a>(s: &'a str) -> Element<'a, M> {
    row![text("•").size(13), text(s).size(13).width(Length::Fill)].spacing(8).into()
}

/// A two-column key/description row: a fixed-width monospace key (a command or
/// shortcut) beside a wrapped description. Takes `impl IntoFragment` (not
/// just `&str`) so callers can pass an owned `String` built from data, like
/// `command_row` below does — the same bound `theme::remote_text` uses.
fn entry<'a, M: 'a>(key: impl text::IntoFragment<'a>, desc: impl text::IntoFragment<'a>) -> Element<'a, M> {
    row![
        text(key).font(Font::MONOSPACE).size(12).width(Length::Fixed(160.0)),
        text(desc).size(13).width(Length::Fill),
    ]
    .spacing(10)
    .into()
}

/// Renders one [`slash::CommandSpec`] as an `entry()` row: the description
/// plus its usage line and any aliases, appended rather than duplicated in
/// `description` itself.
fn command_row<'a, M: 'a>(spec: &'static slash::CommandSpec) -> Element<'a, M> {
    let mut desc = spec.description.to_string();
    if let Some(usage) = spec.usage {
        desc.push(' ');
        desc.push_str(usage);
        desc.push('.');
    }
    if !spec.aliases.is_empty() {
        let aliased: Vec<String> = spec.aliases.iter().map(|a| format!("/{a}")).collect();
        desc.push_str(&format!(" (or {}).", aliased.join(", ")));
    }
    entry(format!("/{}", spec.name), desc)
}
