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

    page = page.push(section(
        "Text commands",
        vec![
            entry(
                "/me <action>",
                "Send an action/emote — \"/me waves\" shows as your name followed \
                 by \"waves\". Must start with \"/me \", including the trailing space.",
            ),
            entry(
                "@name",
                "Mention someone: type @ and part of their name, then pick from \
                 the list. They get highlighted and notified.",
            ),
            entry(
                ":shortcode:",
                "Insert a custom emoji by its :shortcode:. The emoji picker \
                 inserts these for you.",
            ),
            entry("Markdown", "Message text supports Markdown — bold, italics, links, and so on."),
            note(
                "That is the whole set of typed commands — there is no /join, \
                 /leave or /invite. Those actions live as buttons and menus, below.",
            ),
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
                "Member management (invite, kick, ban, power levels) and a full \
                 room-settings dialog — only right-click Rename exists today.",
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
/// shortcut) beside a wrapped description.
fn entry<'a, M: 'a>(key: &'a str, desc: &'a str) -> Element<'a, M> {
    row![
        text(key).font(Font::MONOSPACE).size(12).width(Length::Fixed(160.0)),
        text(desc).size(13).width(Length::Fill),
    ]
    .spacing(10)
    .into()
}
