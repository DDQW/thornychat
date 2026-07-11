//! Slash-command parsing for the composer.
//!
//! Matrix/Element-style slash commands are a *client* feature — no Matrix SDK
//! defines or parses them — so we recognise a known set here and turn each
//! submission into one of: a message to send, an action for the sync worker, or
//! a usage error to show. An unknown `/word` is deliberately left alone and
//! sent as an ordinary message, so paths and typos like `/method` still go
//! through; a leading `//` escapes to a literal single slash.
//!
//! Pure string logic only (no `matrix_sdk`/`client_core` types) — the mapping
//! from [`Action`] to a `ClientCommand` lives at the call site in `update.rs`.

/// What a composer submission parses into.
#[derive(Debug, Clone, PartialEq)]
pub enum Parsed {
    /// Send text. `emote` picks `m.emote` (`/me`); `markdown` false posts the
    /// body verbatim as plain text (`/plain`) instead of the usual render.
    Message { body: String, emote: bool, markdown: bool },
    /// A room/moderation action to dispatch to the sync worker.
    Action(Action),
    /// A usage error to show in the composer; nothing is sent.
    Error(String),
}

/// A room or moderation action requested by a command.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Join(String),
    /// Ask to join a knock-rule room (sends the knock; entry happens if a
    /// moderator accepts).
    Knock(String),
    Leave,
    Invite(String),
    Dm(String),
    Kick { user: String, reason: Option<String> },
    Ban { user: String, reason: Option<String> },
    Unban(String),
    Topic(String),
    Nick(String),
    RoomName(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandCategory {
    Text,
    Rooms,
    People,
    Moderation,
}

/// Static metadata for one slash command — the single source of truth for
/// its name/aliases/usage/description, shared by `parse()`'s usage-error
/// text (via the `USAGE_*` consts below) and the in-app manual
/// (`screens::settings::manual::command_row`), so the two cannot describe a
/// command differently. `slash.rs` and `manual.rs` disagreed on three of
/// these before this table existed (join's missing `!roomid:server` form,
/// "new name" vs "name" for /roomname, "display name" vs "name" for /nick)
/// — proof this drifts in practice, not just in theory.
#[derive(Debug)]
pub struct CommandSpec {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    /// `None` for commands that can't fail (`/leave`, the emoticons).
    pub usage: Option<&'static str>,
    /// One or two sentences for the manual. Does not restate usage or
    /// aliases — the manual appends those from `usage`/`aliases` itself.
    pub description: &'static str,
    pub category: CommandCategory,
}

const USAGE_ME: &str = "Usage: /me <action>";
const USAGE_PLAIN: &str = "Usage: /plain <message>";
const USAGE_JOIN: &str = "Usage: /join <#room:server or !roomid:server>";
const USAGE_KNOCK: &str = "Usage: /knock <#room:server or !roomid:server>";
const USAGE_ROOMNAME: &str = "Usage: /roomname <new name>";
const USAGE_TOPIC: &str = "Usage: /topic <text>";
const USAGE_INVITE: &str = "Usage: /invite <@user:server>";
const USAGE_DM: &str = "Usage: /dm <@user:server>";
const USAGE_NICK: &str = "Usage: /nick <display name>";
const USAGE_KICK: &str = "Usage: /kick <@user:server> [reason]";
const USAGE_BAN: &str = "Usage: /ban <@user:server> [reason]";
const USAGE_UNBAN: &str = "Usage: /unban <@user:server>";

/// Every slash command the composer recognises, in the same order the
/// `parse()` match below groups them (text / rooms / people / moderation).
/// This is the canonical list — add a command here *and* to the `match` in
/// `parse()`; the `every_command_in_the_catalog_actually_parses` test below
/// catches a `COMMANDS` entry with no matching match arm.
pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "me",
        aliases: &[],
        usage: Some(USAGE_ME),
        description: "Emote: \"/me waves\" shows as your name followed by \"waves\", in the emote colour (Appearance, \"Emote/action text\").",
        category: CommandCategory::Text,
    },
    CommandSpec {
        name: "plain",
        aliases: &[],
        usage: Some(USAGE_PLAIN),
        description: "Send the text verbatim, with no Markdown formatting.",
        category: CommandCategory::Text,
    },
    CommandSpec {
        name: "shrug",
        aliases: &[],
        usage: None,
        description: "Prepend the classic shrug emoticon, then any text you add after it.",
        category: CommandCategory::Text,
    },
    CommandSpec {
        name: "tableflip",
        aliases: &[],
        usage: None,
        description: "Prepend a table-flip emoticon, then any text you add after it.",
        category: CommandCategory::Text,
    },
    CommandSpec {
        name: "unflip",
        aliases: &[],
        usage: None,
        description: "Prepend a put-the-table-back emoticon, then any text you add after it.",
        category: CommandCategory::Text,
    },
    CommandSpec {
        name: "lenny",
        aliases: &[],
        usage: None,
        description: "Prepend the Lenny face, then any text you add after it.",
        category: CommandCategory::Text,
    },
    CommandSpec {
        name: "join",
        aliases: &["j"],
        usage: Some(USAGE_JOIN),
        description: "Join a room by alias or id.",
        category: CommandCategory::Rooms,
    },
    CommandSpec {
        name: "knock",
        aliases: &[],
        usage: Some(USAGE_KNOCK),
        description: "Ask to join a knock-rule room; you enter if a moderator accepts.",
        category: CommandCategory::Rooms,
    },
    CommandSpec {
        name: "leave",
        aliases: &["part"],
        usage: None,
        description: "Leave the room you're viewing.",
        category: CommandCategory::Rooms,
    },
    CommandSpec {
        name: "roomname",
        aliases: &["rename"],
        usage: Some(USAGE_ROOMNAME),
        description: "Rename the current room.",
        category: CommandCategory::Rooms,
    },
    CommandSpec {
        name: "topic",
        aliases: &[],
        usage: Some(USAGE_TOPIC),
        description: "Set the current room's topic. Other clients show it; ThornyChat doesn't display topics yet.",
        category: CommandCategory::Rooms,
    },
    CommandSpec {
        name: "invite",
        aliases: &[],
        usage: Some(USAGE_INVITE),
        description: "Invite someone to this room.",
        category: CommandCategory::People,
    },
    CommandSpec {
        name: "dm",
        aliases: &["msg", "query"],
        usage: Some(USAGE_DM),
        description: "Open or start a direct message.",
        category: CommandCategory::People,
    },
    CommandSpec {
        name: "nick",
        aliases: &[],
        usage: Some(USAGE_NICK),
        description: "Change your display name across your whole account.",
        category: CommandCategory::People,
    },
    CommandSpec {
        name: "kick",
        aliases: &[],
        usage: Some(USAGE_KICK),
        description: "Remove someone from the room; they can rejoin.",
        category: CommandCategory::Moderation,
    },
    CommandSpec {
        name: "ban",
        aliases: &[],
        usage: Some(USAGE_BAN),
        description: "Ban someone; they can't rejoin until unbanned.",
        category: CommandCategory::Moderation,
    },
    CommandSpec {
        name: "unban",
        aliases: &[],
        usage: Some(USAGE_UNBAN),
        description: "Lift a ban.",
        category: CommandCategory::Moderation,
    },
];

/// Parse one composer submission.
pub fn parse(input: &str) -> Parsed {
    let text = input.trim();
    let Some(rest) = text.strip_prefix('/') else {
        return message(text);
    };
    // `//text` → a literal message beginning with a single slash.
    if let Some(escaped) = rest.strip_prefix('/') {
        return message(&format!("/{escaped}"));
    }

    let (cmd, args) = match rest.split_once(char::is_whitespace) {
        Some((c, a)) => (c, a.trim()),
        None => (rest, ""),
    };

    match cmd.to_ascii_lowercase().as_str() {
        // --- text ---
        "me" => match require(args, USAGE_ME) {
            Ok(a) => Parsed::Message { body: a.to_string(), emote: true, markdown: true },
            Err(e) => e,
        },
        "plain" => match require(args, USAGE_PLAIN) {
            Ok(a) => Parsed::Message { body: a.to_string(), emote: false, markdown: false },
            Err(e) => e,
        },
        "shrug" => message(&prepend("¯\\_(ツ)_/¯", args)),
        "tableflip" => message(&prepend("(╯°□°）╯︵ ┻━┻", args)),
        "unflip" => message(&prepend("┬─┬ ノ( ゜-゜ノ)", args)),
        "lenny" => message(&prepend("( ͡° ͜ʖ ͡°)", args)),

        // --- rooms ---
        "join" | "j" => match require_room(args, USAGE_JOIN) {
            Ok(r) => Parsed::Action(Action::Join(r)),
            Err(e) => e,
        },
        "knock" => match require_room(args, USAGE_KNOCK) {
            Ok(r) => Parsed::Action(Action::Knock(r)),
            Err(e) => e,
        },
        "leave" | "part" => Parsed::Action(Action::Leave),
        "roomname" | "rename" => match require(args, USAGE_ROOMNAME) {
            Ok(n) => Parsed::Action(Action::RoomName(n.to_string())),
            Err(e) => e,
        },
        "topic" => match require(args, USAGE_TOPIC) {
            Ok(t) => Parsed::Action(Action::Topic(t.to_string())),
            Err(e) => e,
        },

        // --- people ---
        "invite" => match require_user(args, USAGE_INVITE) {
            Ok(u) => Parsed::Action(Action::Invite(u)),
            Err(e) => e,
        },
        "dm" | "msg" | "query" => match require_user(args, USAGE_DM) {
            Ok(u) => Parsed::Action(Action::Dm(u)),
            Err(e) => e,
        },
        "nick" => match require(args, USAGE_NICK) {
            Ok(n) => Parsed::Action(Action::Nick(n.to_string())),
            Err(e) => e,
        },

        // --- moderation ---
        "kick" => match require_user_reason(args, USAGE_KICK) {
            Ok((user, reason)) => Parsed::Action(Action::Kick { user, reason }),
            Err(e) => e,
        },
        "ban" => match require_user_reason(args, USAGE_BAN) {
            Ok((user, reason)) => Parsed::Action(Action::Ban { user, reason }),
            Err(e) => e,
        },
        "unban" => match require_user(args, USAGE_UNBAN) {
            Ok(u) => Parsed::Action(Action::Unban(u)),
            Err(e) => e,
        },

        // Unknown command: leave it alone and send the original text verbatim.
        _ => message(text),
    }
}

fn message(body: &str) -> Parsed {
    Parsed::Message { body: body.to_string(), emote: false, markdown: true }
}

/// A text emoticon command sends the emoticon, optionally followed by the rest
/// of the line (matching Element's `/shrug`, `/tableflip`, …).
fn prepend(prefix: &str, args: &str) -> String {
    if args.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix} {args}")
    }
}

fn require<'a>(args: &'a str, usage: &str) -> Result<&'a str, Parsed> {
    if args.is_empty() {
        Err(Parsed::Error(usage.to_string()))
    } else {
        Ok(args)
    }
}

/// First whitespace-delimited token, validated as an `@user:server` id.
fn require_user(args: &str, usage: &str) -> Result<String, Parsed> {
    let first = require(args, usage)?.split_whitespace().next().unwrap_or_default();
    if looks_like_user(first) {
        Ok(first.to_string())
    } else {
        Err(Parsed::Error(format!("{usage} — user ids look like @name:server")))
    }
}

/// First token as a room id/alias, validated as `#alias:server` or `!id:server`.
fn require_room(args: &str, usage: &str) -> Result<String, Parsed> {
    let first = require(args, usage)?.split_whitespace().next().unwrap_or_default();
    if looks_like_room(first) {
        Ok(first.to_string())
    } else {
        Err(Parsed::Error(usage.to_string()))
    }
}

/// `<@user:server> [reason]` — the user is the first token, the reason is
/// everything after it (or `None`).
fn require_user_reason(args: &str, usage: &str) -> Result<(String, Option<String>), Parsed> {
    require(args, usage)?;
    let (user, reason) = match args.split_once(char::is_whitespace) {
        Some((u, r)) => {
            let r = r.trim();
            (u, if r.is_empty() { None } else { Some(r.to_string()) })
        }
        None => (args, None),
    };
    if looks_like_user(user) {
        Ok((user.to_string(), reason))
    } else {
        Err(Parsed::Error(format!("{usage} — user ids look like @name:server")))
    }
}

fn looks_like_user(s: &str) -> bool {
    s.starts_with('@') && s.contains(':')
}

fn looks_like_room(s: &str) -> bool {
    (s.starts_with('#') || s.starts_with('!')) && s.contains(':')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(body: &str) -> Parsed {
        Parsed::Message { body: body.to_string(), emote: false, markdown: true }
    }

    #[test]
    fn plain_text_passes_through() {
        assert_eq!(parse("hello world"), msg("hello world"));
    }

    #[test]
    fn me_is_an_emote() {
        assert_eq!(
            parse("/me waves"),
            Parsed::Message { body: "waves".into(), emote: true, markdown: true }
        );
    }

    #[test]
    fn bare_me_is_a_usage_error() {
        assert!(matches!(parse("/me"), Parsed::Error(_)));
    }

    #[test]
    fn method_is_not_a_command() {
        // A word that merely starts with /me still sends as text.
        assert_eq!(parse("/method foo"), msg("/method foo"));
    }

    #[test]
    fn plain_skips_markdown() {
        assert_eq!(
            parse("/plain **bold**"),
            Parsed::Message { body: "**bold**".into(), emote: false, markdown: false }
        );
    }

    #[test]
    fn shrug_with_and_without_text() {
        assert_eq!(parse("/shrug"), msg("¯\\_(ツ)_/¯"));
        assert_eq!(parse("/shrug hi"), msg("¯\\_(ツ)_/¯ hi"));
    }

    #[test]
    fn double_slash_escapes() {
        assert_eq!(parse("//me literally"), msg("/me literally"));
    }

    #[test]
    fn room_and_people_actions() {
        assert_eq!(parse("/join #a:b.com"), Parsed::Action(Action::Join("#a:b.com".into())));
        assert_eq!(parse("/knock #a:b.com"), Parsed::Action(Action::Knock("#a:b.com".into())));
        assert_eq!(parse("/leave"), Parsed::Action(Action::Leave));
        assert_eq!(parse("/invite @u:b.com"), Parsed::Action(Action::Invite("@u:b.com".into())));
        assert_eq!(parse("/dm @u:b.com"), Parsed::Action(Action::Dm("@u:b.com".into())));
    }

    #[test]
    fn kick_splits_user_and_reason() {
        assert_eq!(
            parse("/kick @u:b.com being rude"),
            Parsed::Action(Action::Kick { user: "@u:b.com".into(), reason: Some("being rude".into()) })
        );
        assert_eq!(
            parse("/ban @u:b.com"),
            Parsed::Action(Action::Ban { user: "@u:b.com".into(), reason: None })
        );
    }

    #[test]
    fn bad_arguments_are_usage_errors() {
        assert!(matches!(parse("/kick"), Parsed::Error(_)));
        assert!(matches!(parse("/kick notauser"), Parsed::Error(_)));
        assert!(matches!(parse("/join notaroom"), Parsed::Error(_)));
        assert!(matches!(parse("/knock notaroom"), Parsed::Error(_)));
    }

    #[test]
    fn unknown_command_sends_as_text() {
        assert_eq!(parse("/wobble on"), msg("/wobble on"));
    }

    #[test]
    fn every_command_in_the_catalog_actually_parses() {
        // Catches a COMMANDS entry with no matching arm in parse()'s match:
        // an unmatched name falls through to the unknown-command case, which
        // echoes the input back verbatim as a Message. A real match arm for
        // that name never produces that exact echo (it errors, acts, or
        // sends a transformed body), so this is a reliable tripwire — not
        // the reverse direction (a match arm with no COMMANDS entry).
        for spec in COMMANDS {
            let echoed_unknown = msg(&format!("/{}", spec.name));
            assert_ne!(
                parse(&format!("/{}", spec.name)),
                echoed_unknown,
                "\"/{}\" is in COMMANDS but parse() doesn't recognise it",
                spec.name
            );
        }
    }
}
