// Slash-command parsing for the composer.
//
// Matrix/Element-style slash commands are a *client* feature — no Matrix SDK
// defines or parses them — so we recognise a known set here and turn each
// submission into one of: a message to send, an action for the sync worker, or
// a usage error to show. An unknown `/word` is deliberately left alone and
// sent as an ordinary message, so paths and typos like `/method` still go
// through; a leading `//` escapes to a literal single slash.
//
// Pure string logic only. Ported from the iced build's `ui::slash`; the mapping
// from `Action` to a `ClientCommand` lives in `composer-actions.ts`.

export type Parsed =
  /** Send text. `emote` picks `m.emote` (`/me`); `markdown: false` posts the body verbatim (`/plain`). */
  | { kind: 'message'; body: string; emote: boolean; markdown: boolean }
  | { kind: 'action'; action: Action }
  /** A usage error to show in the composer; nothing is sent. */
  | { kind: 'error'; message: string };

export type Action =
  | { type: 'join'; room: string }
  /** Ask to join a knock-rule room (entry happens if a moderator accepts). */
  | { type: 'knock'; room: string }
  | { type: 'leave' }
  | { type: 'invite'; user: string }
  | { type: 'dm'; user: string }
  | { type: 'kick'; user: string; reason: string | null }
  | { type: 'ban'; user: string; reason: string | null }
  | { type: 'unban'; user: string }
  /** Account-wide, unlike kick/ban: the homeserver stops delivering this user's messages everywhere. */
  | { type: 'ignore'; user: string }
  | { type: 'unignore'; user: string }
  | { type: 'topic'; text: string }
  | { type: 'nick'; name: string }
  | { type: 'roomname'; name: string };

export type CommandCategory = 'text' | 'rooms' | 'people' | 'moderation';

/**
 * Static metadata for one slash command — the single source of truth for its
 * name, aliases, usage and description, shared by `parse()`'s usage errors and
 * the in-app manual so the two cannot describe a command differently.
 */
export interface CommandSpec {
  name: string;
  aliases: string[];
  /** `null` for commands that can't fail (`/leave`, the emoticons). */
  usage: string | null;
  description: string;
  category: CommandCategory;
}

const USAGE_ME = 'Usage: /me <action>';
const USAGE_PLAIN = 'Usage: /plain <message>';
const USAGE_JOIN = 'Usage: /join <#room:server or !roomid:server>';
const USAGE_KNOCK = 'Usage: /knock <#room:server or !roomid:server>';
const USAGE_ROOMNAME = 'Usage: /roomname <new name>';
const USAGE_TOPIC = 'Usage: /topic <text>';
const USAGE_INVITE = 'Usage: /invite <@user:server>';
const USAGE_DM = 'Usage: /dm <@user:server>';
const USAGE_NICK = 'Usage: /nick <display name>';
const USAGE_KICK = 'Usage: /kick <@user:server> [reason]';
const USAGE_BAN = 'Usage: /ban <@user:server> [reason]';
const USAGE_UNBAN = 'Usage: /unban <@user:server>';
const USAGE_IGNORE = 'Usage: /ignore <@user:server>';
const USAGE_UNIGNORE = 'Usage: /unignore <@user:server>';

/**
 * Every slash command the composer recognises. This is the canonical list —
 * add a command here *and* to the `switch` in `parse()`; the test
 * "every command in the catalog actually parses" catches a catalog entry with
 * no matching case.
 */
export const COMMANDS: readonly CommandSpec[] = [
  {
    name: 'me',
    aliases: [],
    usage: USAGE_ME,
    description: 'Emote: "/me waves" shows as your name followed by "waves", in the emote colour (Appearance, "Emote/action text").',
    category: 'text',
  },
  { name: 'plain', aliases: [], usage: USAGE_PLAIN, description: 'Send the text verbatim, with no Markdown formatting.', category: 'text' },
  { name: 'shrug', aliases: [], usage: null, description: 'Prepend the classic shrug emoticon, then any text you add after it.', category: 'text' },
  { name: 'tableflip', aliases: [], usage: null, description: 'Prepend a table-flip emoticon, then any text you add after it.', category: 'text' },
  { name: 'unflip', aliases: [], usage: null, description: 'Prepend a put-the-table-back emoticon, then any text you add after it.', category: 'text' },
  { name: 'lenny', aliases: [], usage: null, description: 'Prepend the Lenny face, then any text you add after it.', category: 'text' },
  { name: 'join', aliases: ['j'], usage: USAGE_JOIN, description: 'Join a room by alias or id.', category: 'rooms' },
  { name: 'knock', aliases: [], usage: USAGE_KNOCK, description: 'Ask to join a knock-rule room; you enter if a moderator accepts.', category: 'rooms' },
  { name: 'leave', aliases: ['part'], usage: null, description: "Leave the room you're viewing.", category: 'rooms' },
  { name: 'roomname', aliases: ['rename'], usage: USAGE_ROOMNAME, description: 'Rename the current room.', category: 'rooms' },
  { name: 'topic', aliases: [], usage: USAGE_TOPIC, description: "Set the current room's topic.", category: 'rooms' },
  { name: 'invite', aliases: [], usage: USAGE_INVITE, description: 'Invite someone to this room.', category: 'people' },
  { name: 'dm', aliases: ['msg', 'query'], usage: USAGE_DM, description: 'Open or start a direct message.', category: 'people' },
  { name: 'nick', aliases: [], usage: USAGE_NICK, description: 'Change your display name across your whole account.', category: 'people' },
  { name: 'kick', aliases: [], usage: USAGE_KICK, description: 'Remove someone from the room; they can rejoin.', category: 'moderation' },
  { name: 'ban', aliases: [], usage: USAGE_BAN, description: "Ban someone; they can't rejoin until unbanned.", category: 'moderation' },
  { name: 'unban', aliases: [], usage: USAGE_UNBAN, description: 'Lift a ban.', category: 'moderation' },
  {
    name: 'ignore',
    aliases: [],
    usage: USAGE_IGNORE,
    description:
      "Hide someone everywhere. Unlike a kick or ban this is account-wide and needs no privileges: your homeserver stops sending you their messages in every room, on every device you sign in from.",
    category: 'moderation',
  },
  {
    name: 'unignore',
    aliases: [],
    usage: USAGE_UNIGNORE,
    description: 'Stop ignoring someone. The messages they sent while ignored come back the next time the room loads its history.',
    category: 'moderation',
  },
];

/** Parse one composer submission. */
export function parse(input: string): Parsed {
  const text = input.trim();
  if (!text.startsWith('/')) return message(text);
  const rest = text.slice(1);
  // `//text` → a literal message beginning with a single slash.
  if (rest.startsWith('/')) return message(rest);

  const split = rest.search(/\s/);
  const cmd = split === -1 ? rest : rest.slice(0, split);
  const args = split === -1 ? '' : rest.slice(split).trim();

  switch (cmd.toLowerCase()) {
    // --- text ---
    case 'me': {
      const a = need(args, USAGE_ME);
      return typeof a === 'string' ? { kind: 'message', body: a, emote: true, markdown: true } : a;
    }
    case 'plain': {
      const a = need(args, USAGE_PLAIN);
      return typeof a === 'string' ? { kind: 'message', body: a, emote: false, markdown: false } : a;
    }
    case 'shrug':
      return message(prepend('¯\\_(ツ)_/¯', args));
    case 'tableflip':
      return message(prepend('(╯°□°）╯︵ ┻━┻', args));
    case 'unflip':
      return message(prepend('┬─┬ ノ( ゜-゜ノ)', args));
    case 'lenny':
      return message(prepend('( ͡° ͜ʖ ͡°)', args));

    // --- rooms ---
    case 'join':
    case 'j': {
      const r = requireRoom(args, USAGE_JOIN);
      return typeof r === 'string' ? action({ type: 'join', room: r }) : r;
    }
    case 'knock': {
      const r = requireRoom(args, USAGE_KNOCK);
      return typeof r === 'string' ? action({ type: 'knock', room: r }) : r;
    }
    case 'leave':
    case 'part':
      return action({ type: 'leave' });
    case 'roomname':
    case 'rename': {
      const n = need(args, USAGE_ROOMNAME);
      return typeof n === 'string' ? action({ type: 'roomname', name: n }) : n;
    }
    case 'topic': {
      const t = need(args, USAGE_TOPIC);
      return typeof t === 'string' ? action({ type: 'topic', text: t }) : t;
    }

    // --- people ---
    case 'invite': {
      const u = requireUser(args, USAGE_INVITE);
      return typeof u === 'string' ? action({ type: 'invite', user: u }) : u;
    }
    case 'dm':
    case 'msg':
    case 'query': {
      const u = requireUser(args, USAGE_DM);
      return typeof u === 'string' ? action({ type: 'dm', user: u }) : u;
    }
    case 'nick': {
      const n = need(args, USAGE_NICK);
      return typeof n === 'string' ? action({ type: 'nick', name: n }) : n;
    }

    // --- moderation ---
    case 'kick': {
      const r = requireUserReason(args, USAGE_KICK);
      return 'kind' in r ? r : action({ type: 'kick', user: r.user, reason: r.reason });
    }
    case 'ban': {
      const r = requireUserReason(args, USAGE_BAN);
      return 'kind' in r ? r : action({ type: 'ban', user: r.user, reason: r.reason });
    }
    case 'unban': {
      const u = requireUser(args, USAGE_UNBAN);
      return typeof u === 'string' ? action({ type: 'unban', user: u }) : u;
    }
    case 'ignore': {
      const u = requireUser(args, USAGE_IGNORE);
      return typeof u === 'string' ? action({ type: 'ignore', user: u }) : u;
    }
    case 'unignore': {
      const u = requireUser(args, USAGE_UNIGNORE);
      return typeof u === 'string' ? action({ type: 'unignore', user: u }) : u;
    }

    // Unknown command: leave it alone and send the original text verbatim.
    default:
      return message(text);
  }
}

function message(body: string): Parsed {
  return { kind: 'message', body, emote: false, markdown: true };
}

function action(a: Action): Parsed {
  return { kind: 'action', action: a };
}

function error(message: string): Parsed {
  return { kind: 'error', message };
}

/** A text emoticon command sends the emoticon, optionally followed by the rest of the line. */
function prepend(prefix: string, args: string): string {
  return args === '' ? prefix : `${prefix} ${args}`;
}

function need(args: string, usage: string): string | Parsed {
  return args === '' ? error(usage) : args;
}

function firstToken(text: string): string {
  return text.split(/\s+/)[0] ?? '';
}

/** First whitespace-delimited token, validated as an `@user:server` id. */
function requireUser(args: string, usage: string): string | Parsed {
  const required = need(args, usage);
  if (typeof required !== 'string') return required;
  const first = firstToken(required);
  return looksLikeUser(first) ? first : error(`${usage} — user ids look like @name:server`);
}

/** First token as a room id/alias, validated as `#alias:server` or `!id:server`. */
function requireRoom(args: string, usage: string): string | Parsed {
  const required = need(args, usage);
  if (typeof required !== 'string') return required;
  const first = firstToken(required);
  return looksLikeRoom(first) ? first : error(usage);
}

/** `<@user:server> [reason]` — the user is the first token, the reason everything after it (or null). */
function requireUserReason(args: string, usage: string): { user: string; reason: string | null } | Parsed {
  const required = need(args, usage);
  if (typeof required !== 'string') return required;
  const split = required.search(/\s/);
  const user = split === -1 ? required : required.slice(0, split);
  const reason = split === -1 ? '' : required.slice(split).trim();
  return looksLikeUser(user) ? { user, reason: reason === '' ? null : reason } : error(`${usage} — user ids look like @name:server`);
}

export function looksLikeUser(s: string): boolean {
  return s.startsWith('@') && s.includes(':');
}

export function looksLikeRoom(s: string): boolean {
  return (s.startsWith('#') || s.startsWith('!')) && s.includes(':');
}
