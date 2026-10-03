// A stand-in for the Rust shell, used only when the page runs in a plain
// browser (`npm run dev`), so the UI can be built and inspected without a
// homeserver. It plays the part of the sync worker: it answers commands with
// the same events the real one would, over data that exercises every message
// kind the timeline renders. Nothing in the app imports this directly.
//
// Two URL parameters make rooms big, for performance work: `?history=N` opens
// every room with N synthetic messages of mixed kinds, and `?members=N` gives
// it an N-member roster.

import type { Backend } from './api';
import type {
  ClientCommand,
  ClientEvent,
  RoomMember,
  RoomSummary,
  TimelineDiff,
  TimelineItem,
  TimelineItemContent,
} from './bindings';
import type { Settings, ThemeConfig } from './types';

const ME = '@you:thorny.chat';
const HOUR = 3_600_000;

const dark: ThemeConfig = {
  name: 'ThornyChat Dark',
  dark: true,
  background: '#1A1A1A',
  surface: '#262626',
  surface_strong: '#333333',
  text: '#F2F2F2',
  muted_text: '#999999',
  accent: '#BDB6EC',
  accent_text: '#2C2843',
  success: '#85E0BA',
  danger: '#E69D9D',
  emote: '#C9A9E0',
  font_family: null,
  ui_scale: 1,
  corner_radius: 10,
};
const light: ThemeConfig = {
  ...dark,
  name: 'ThornyChat Light',
  dark: false,
  background: '#F2F2F2',
  surface: '#FFFFFF',
  surface_strong: '#E5E5E5',
  text: '#000000',
  muted_text: '#5C5C5C',
  accent: '#1858D5',
  accent_text: '#FFFFFF',
  success: '#00844C',
  danger: '#C40E0E',
  emote: '#6B4FA0',
};

const MEMBERS: RoomMember[] = [
  { user_id: ME, display_name: 'You', avatar_url: null, power_level: 100 },
  { user_id: '@alice:thorny.chat', display_name: 'Alice', avatar_url: null, power_level: 100 },
  { user_id: '@bob:thorny.chat', display_name: 'Bob Builder', avatar_url: null, power_level: 50 },
  { user_id: '@carol:thorny.chat', display_name: 'Carol', avatar_url: null, power_level: 0 },
  { user_id: '@irc_dave:thorny.chat', display_name: '', avatar_url: null, power_level: 0 },
];

const room = (id: string, name: string, extra: Partial<RoomSummary> = {}): RoomSummary => ({
  room_id: id,
  name,
  topic: null,
  avatar_url: null,
  unread_count: 0,
  is_encrypted: false,
  is_space: false,
  is_dm: false,
  last_message_preview: null,
  ...extra,
});

const ROOMS: RoomSummary[] = [
  room('!hq:thorny.chat', 'ThornyChat HQ', { is_space: true, topic: 'Where the thorns live' }),
  room('!general:thorny.chat', 'general', { topic: 'Everything and nothing. Be kind.', unread_count: 3 }),
  room('!dev:thorny.chat', 'dev', { topic: 'Rust, Svelte, and regrets', is_encrypted: true }),
  room('!random:thorny.chat', 'random', { unread_count: 12 }),
  room('!alice:thorny.chat', 'Alice', { is_dm: true, unread_count: 1 }),
  room('!bob:thorny.chat', 'Bob Builder', { is_dm: true }),
  room('!lonely:thorny.chat', 'Lonely room'),
];

let counter = 0;
const nextId = () => `$mock${++counter}:thorny.chat`;

function item(sender: string, content: TimelineItemContent, at: number, extra: Partial<TimelineItem> = {}): TimelineItem {
  const member = MEMBERS.find((m) => m.user_id === sender);
  return {
    event_id: nextId(),
    sender,
    sender_display_name: member ? member.display_name || null : null,
    sender_avatar_url: null,
    timestamp_ms: at,
    content,
    formatted_body: null,
    shield: null,
    reactions: [],
    thread_root: null,
    thread_reply_count: null,
    read_by: [],
    in_reply_to: null,
    edited: false,
    send_failed: null,
    ...extra,
  };
}

const text = (data: string): TimelineItemContent => ({ type: 'Text', data });

function sampleHistory(now: number): TimelineItem[] {
  const t = (hoursAgo: number, minutes = 0) => now - hoursAgo * HOUR + minutes * 60_000;
  const items: TimelineItem[] = [
    item('', { type: 'DateDivider', data: 'Yesterday' }, t(30)),
    item('@alice:thorny.chat', text('Morning! The tauri migration branch is green.'), t(29)),
    item('@alice:thorny.chat', text('Still need to port the composer though.'), t(29, 1)),
    item('@bob:thorny.chat', text('Nice. Did the timeline survive the move?'), t(28)),
    item('', { type: 'MembershipChange', data: 'Carol joined the room' }, t(27)),
    item('', { type: 'DateDivider', data: 'Today' }, t(6)),
    item('@carol:thorny.chat', text('Hi all, glad to be here.'), t(5)),
    item('@irc_dave:thorny.chat', { type: 'Emote', data: 'waves from the IRC bridge' }, t(5, 2)),
    item('@bob:thorny.chat', text('Welcome Carol! Check the **pinned** message for the rules.'), t(4), {
      formatted_body: '<p>Welcome <a href="https://matrix.to/#/@carol:thorny.chat">Carol</a>! Check the <strong>pinned</strong> message for the <em>rules</em>.</p>',
      reactions: [
        { key: '👍', count: 2, reacted_by_me: true, senders: [ME, '@alice:thorny.chat'] },
        { key: '🎉', count: 1, reacted_by_me: false, senders: ['@carol:thorny.chat'] },
      ],
    }),
    item('@alice:thorny.chat', { type: 'Image', data: { url: 'mxc://mock/sunset', caption: 'Sunset from the office', width: 640, height: 360 } }, t(3)),
    item('@alice:thorny.chat', { type: 'Sticker', data: { url: 'mxc://mock/sticker1', body: 'thorn-wave', width: 128, height: 128 } }, t(3, 1)),
    item(
      '@carol:thorny.chat',
      text('Thanks! That looks lovely.'),
      t(3, 3),
      { in_reply_to: { event_id: '$img', sender: 'Alice', snippet: '[image: Sunset from the office]', image_url: 'mxc://mock/sunset' } },
    ),
    item('@bob:thorny.chat', { type: 'File', data: { url: 'mxc://mock/report', filename: 'quarterly-report.pdf', caption: null } }, t(2)),
    item('@bob:thorny.chat', { type: 'Video', data: { url: 'mxc://mock/clip', filename: 'demo.mp4', caption: 'Demo of the new build', mimetype: 'video/mp4', thumbnail_url: 'mxc://mock/clipthumb' } }, t(2, 1)),
    item('@alice:thorny.chat', text('Related reading: https://www.youtube.com/watch?v=dQw4w9WgXcQ and https://x.com/rustlang/status/1234567890'), t(1)),
    item(
      '@bob:thorny.chat',
      text('```rust\nfn main() {\n    println!("hello thorns");\n}\n```'),
      t(1, 5),
      {
        formatted_body:
          '<pre><code class="language-rust">fn main() {\n    println!("hello thorns");\n}\n</code></pre><p>And a <span data-mx-spoiler>spoiler</span>, plus <font data-mx-color="#ff8844">colour</font>.</p>',
        edited: true,
      },
    ),
    item('@carol:thorny.chat', { type: 'Redacted' }, t(0, 40)),
    item(ME, text('Looks great to me, shipping it.'), t(0, 20), { read_by: ['@alice:thorny.chat', '@bob:thorny.chat'] }),
    item('', { type: 'NewMessagesDivider' }, t(0, 10)),
    item('@alice:thorny.chat', text('One more thing: remember the standup at 10.'), t(0, 5)),
  ];
  return items;
}

/** `count` messages of mixed kinds (text, emoji, links, markup, images, reactions, replies). */
function syntheticHistory(count: number, now: number): TimelineItem[] {
  const senders = MEMBERS.map((member) => member.user_id);
  const lines = [
    'Morning! The build is green again 🎉',
    'Did anyone look at https://github.com/DDQW/thornychat/issues/12 yet?',
    'lgtm',
    'I think the timeline should stay capped at a couple of hundred items; past that the DOM grows for days.',
    'haha 😂😂',
    'Reminder: standup at 10',
  ];
  return Array.from({ length: count }, (_, i) => {
    const at = now - (count - i) * 45_000;
    const sender = senders[i % senders.length]!;
    switch (i % 10) {
      case 3:
        return item(sender, text('Use **bold** and `code`'), at, { formatted_body: '<p>Use <strong>bold</strong> and <code>code</code>, see <a href="https://matrix.org">matrix.org</a></p>' });
      case 6:
        return item(sender, { type: 'Image', data: { url: 'mxc://mock/sunset', caption: null, width: 640, height: 360 } }, at);
      case 8:
        return item(sender, text(lines[i % lines.length]!), at, { reactions: [{ key: '👍', count: 2, reacted_by_me: false, senders: senders.slice(0, 2) }], read_by: [senders[2]!] });
      default:
        return item(sender, text(lines[i % lines.length]!), at, i % 10 === 5 ? { in_reply_to: { event_id: '$x', sender: 'Alice', snippet: 'an earlier message', image_url: null } } : {});
    }
  });
}

/** A roster of `count` people, for `?members=N`. */
function syntheticMembers(count: number): RoomMember[] {
  return [
    ...MEMBERS,
    ...Array.from({ length: count }, (_, i) => ({ user_id: `@user${i}:example.org`, display_name: `Person ${String(i).padStart(5, '0')}`, avatar_url: null, power_level: 0 })),
  ];
}

function olderHistory(before: number, page: number): TimelineItem[] {
  return Array.from({ length: 12 }, (_, i) =>
    item(i % 3 === 0 ? '@bob:thorny.chat' : '@alice:thorny.chat', text(`Older message #${page * 12 + i + 1}: lorem ipsum dolor sit amet.`), before - (12 - i) * 5 * 60_000 - page * 90 * 60_000),
  );
}

/** The mock's speller: a few classic typos and what it would offer for them. */
const TYPOS: Record<string, string[]> = {
  teh: ['the', 'ten', 'tech'],
  recieve: ['receive'],
  recieved: ['received'],
  definately: ['definitely', 'defiantly'],
  seperate: ['separate'],
  fiel: ['file', 'feel', 'fuel'],
  wierd: ['weird', 'wired'],
  adress: ['address'],
  untill: ['until'],
  thier: ['their'],
  smyth: ['smith'],
};

function svgFor(label: string, color: string, width = 640, height = 360): string {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="${color}"/><stop offset="1" stop-color="#222"/></linearGradient></defs><rect width="100%" height="100%" fill="url(#g)"/><text x="50%" y="50%" fill="white" font-family="sans-serif" font-size="${Math.round(height / 9)}" text-anchor="middle" dominant-baseline="middle">${label}</text></svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}

export function createMockBackend(): Backend {
  let onEvent: (event: ClientEvent) => void = () => {};
  let settings: Settings = {
    theme: dark,
    chat: { show_membership_events: true, hide_members: false },
    privacy: { send_read_receipts: false, send_typing_notifications: false, enable_link_previews: true },
    encryption: { encrypt_direct_messages: false, encrypt_rooms: false },
    connectors: { steam_enabled: false, gog_enabled: false, epic_enabled: false, announce_stop: false, poll_interval_secs: 30 },
    spellcheck: { enabled: true, autocorrect: false },
    log: { level: 'info' },
  };
  const profileFiles = new Map<string, string>();
  const timelines = new Map<string, TimelineItem[]>();
  const params = new URLSearchParams(location.search);
  let loggedIn = params.get('login') === null;
  const bigHistory = Number(params.get('history')) || 0;
  const bigRoster = Number(params.get('members')) || 0;
  const dictionary = new Set<string>();
  const typo = (word: string) => (dictionary.has(word.toLowerCase()) ? undefined : TYPOS[word.toLowerCase()]);
  let autostart = false;

  const emit = (event: ClientEvent) => queueMicrotask(() => onEvent(event));
  const diffs = (room_id: string, list: TimelineDiff[]) => emit({ type: 'TimelineDiffs', data: { room_id, diffs: list } });
  const ok = (request_id: string, delay = 120) => setTimeout(() => emit({ type: 'CommandSucceeded', data: { request_id } }), delay);
  const fail = (request_id: string, error: string) => setTimeout(() => emit({ type: 'CommandFailed', data: { request_id, error } }), 120);

  function startSession() {
    loggedIn = true;
    setTimeout(() => {
      emit({ type: 'SyncStateChanged', data: { type: 'Syncing' } });
      emit({ type: 'RoomListUpdated', data: ROOMS });
      emit({ type: 'SpaceChildrenFetched', data: { space_id: '!hq:thorny.chat', children: ['!general:thorny.chat', '!dev:thorny.chat', '!random:thorny.chat'] } });
      emit({ type: 'RoomNotificationModesUpdated', data: [['!random:thorny.chat', 'Mute']] });
      emit({ type: 'DefaultNotificationModesUpdated', data: { direct_messages: 'AllMessages', group_chats: 'MentionsAndKeywordsOnly' } });
      emit({ type: 'IgnoredUsersUpdated', data: [] });
      emit({
        type: 'CustomEmojiPacksUpdated',
        data: [{ name: 'Thorny pack', emojis: [{ shortcode: 'thorn_wave', mxc_url: 'mxc://mock/emoji1', is_emoticon: true, is_sticker: true, width: 128, height: 128 }, { shortcode: 'thorn_heart', mxc_url: 'mxc://mock/emoji2', is_emoticon: true, is_sticker: false, width: null, height: null }] }],
      });
    }, 150);
  }

  function handle(command: ClientCommand) {
    switch (command.type) {
      case 'OpenRoom': {
        const id = command.data.room_id;
        const list = id === '!lonely:thorny.chat' ? [] : bigHistory > 0 ? syntheticHistory(bigHistory, Date.now()) : sampleHistory(Date.now());
        timelines.set(id, list);
        setTimeout(() => {
          diffs(id, [{ type: 'Reset', data: list }]);
          emit({ type: 'RoomMembersUpdated', data: { room_id: id, members: bigRoster > 0 ? syntheticMembers(bigRoster) : MEMBERS } });
          emit({
            type: 'PowerLevelTagsUpdated',
            data: { room_id: id, tags: [{ level: 100, name: 'Admins', color: '#e5a3ff' }, { level: 50, name: 'Moderators', color: null }] },
          });
          if (id === '!dev:thorny.chat') {
            emit({ type: 'CallStateUpdated', data: { room_id: id, joined: false, participants: [{ user_id: '@alice:thorny.chat', device_id: 'A' }, { user_id: '@bob:thorny.chat', device_id: 'B' }] } });
          }
          if (id === '!general:thorny.chat') emit({ type: 'TypingUpdated', data: { room_id: id, user_ids: ['@carol:thorny.chat'] } });
        }, 120);
        break;
      }
      case 'CloseRoom':
        timelines.delete(command.data.room_id);
        break;
      case 'PaginateBackwards': {
        const { room_id, request_id } = command.data;
        const current = timelines.get(room_id) ?? [];
        const first = current.find((i) => i.timestamp_ms > 0);
        const page = Math.floor(current.length / 12);
        setTimeout(() => {
          if (page >= 4) {
            emit({ type: 'TimelineStartReached', data: { room_id } });
          } else {
            const older = olderHistory(first?.timestamp_ms ?? Date.now(), page);
            timelines.set(room_id, [...older, ...current]);
            diffs(room_id, older.slice().reverse().map((data): TimelineDiff => ({ type: 'PushFront', data })));
          }
          emit({ type: 'CommandSucceeded', data: { request_id } });
        }, 350);
        break;
      }
      case 'SendMessage': {
        const { room_id, body, request_id, emote, reply_to_event_id } = command.data;
        const echo = item(ME, emote ? { type: 'Emote', data: body } : text(body), Date.now(), {
          event_id: null,
          in_reply_to: reply_to_event_id ? { event_id: reply_to_event_id, sender: 'Someone', snippet: 'a message', image_url: null } : null,
        });
        const list = timelines.get(room_id) ?? [];
        timelines.set(room_id, [...list, echo]);
        diffs(room_id, [{ type: 'PushBack', data: echo }]);
        setTimeout(() => {
          const sent = { ...echo, event_id: nextId() };
          const current = timelines.get(room_id) ?? [];
          const index = current.indexOf(echo);
          if (index >= 0) {
            current[index] = sent;
            diffs(room_id, [{ type: 'Set', data: { index, item: sent } }]);
          }
          emit({ type: 'CommandSucceeded', data: { request_id } });
        }, 500);
        break;
      }
      case 'EditMessage': {
        const { room_id, event_id, new_body, request_id } = command.data;
        const list = timelines.get(room_id) ?? [];
        const index = list.findIndex((i) => i.event_id === event_id);
        if (index >= 0) {
          list[index] = { ...list[index]!, content: text(new_body), formatted_body: null, edited: true };
          diffs(room_id, [{ type: 'Set', data: { index, item: list[index]! } }]);
        }
        ok(request_id);
        break;
      }
      case 'RedactEvent': {
        const { room_id, event_id, request_id } = command.data;
        const list = timelines.get(room_id) ?? [];
        const index = list.findIndex((i) => i.event_id === event_id);
        if (index >= 0) {
          list[index] = { ...list[index]!, content: { type: 'Redacted' }, formatted_body: null, reactions: [] };
          diffs(room_id, [{ type: 'Set', data: { index, item: list[index]! } }]);
        }
        ok(request_id);
        break;
      }
      case 'ToggleReaction': {
        const { room_id, event_id, key, request_id } = command.data;
        const list = timelines.get(room_id) ?? [];
        const index = list.findIndex((i) => i.event_id === event_id);
        if (index >= 0) {
          const target = list[index]!;
          const existing = target.reactions.find((r) => r.key === key);
          let reactions = target.reactions;
          if (!existing) reactions = [...reactions, { key, count: 1, reacted_by_me: true, senders: [ME] }];
          else if (existing.reacted_by_me) reactions = reactions.flatMap((r) => (r.key !== key ? [r] : r.count > 1 ? [{ ...r, count: r.count - 1, reacted_by_me: false, senders: r.senders.filter((s) => s !== ME) }] : []));
          else reactions = reactions.map((r) => (r.key === key ? { ...r, count: r.count + 1, reacted_by_me: true, senders: [...r.senders, ME] } : r));
          list[index] = { ...target, reactions };
          diffs(room_id, [{ type: 'Set', data: { index, item: list[index]! } }]);
        }
        ok(request_id);
        break;
      }
      case 'SearchUsers':
        setTimeout(
          () =>
            emit({
              type: 'UserSearchResults',
              data: {
                request_id: command.data.request_id,
                limited: false,
                results: MEMBERS.filter((m) => `${m.display_name}${m.user_id}`.toLowerCase().includes(command.data.query.toLowerCase())).map((m) => ({ user_id: m.user_id, display_name: m.display_name || null, avatar_url: null })),
              },
            }),
          200,
        );
        break;
      case 'FetchSpaceHierarchy':
        setTimeout(
          () =>
            emit({
              type: 'SpaceHierarchyFetched',
              data: {
                request_id: command.data.request_id,
                space_id: command.data.space_id,
                next_batch: null,
                children: [
                  { room_id: '!general:thorny.chat', name: 'general', topic: 'Everything and nothing', canonical_alias: '#general:thorny.chat', avatar_url: null, num_joined_members: 42, is_space: false, joined: true, join_rule: 'Public', via: ['thorny.chat'] },
                  { room_id: '!announce:thorny.chat', name: 'announcements', topic: 'Release notes', canonical_alias: null, avatar_url: null, num_joined_members: 128, is_space: false, joined: false, join_rule: 'Public', via: ['thorny.chat'] },
                  { room_id: '!staff:thorny.chat', name: 'staff', topic: null, canonical_alias: null, avatar_url: null, num_joined_members: 6, is_space: false, joined: false, join_rule: 'Knock', via: ['thorny.chat'] },
                ],
              },
            }),
          250,
        );
        break;
      case 'FetchUrlPreview':
        setTimeout(
          () =>
            emit({
              type: 'UrlPreviewFetched',
              data: { url: command.data.url, title: 'Rick Astley - Never Gonna Give You Up', description: 'The official video.', site_name: 'YouTube', image_mxc: 'mxc://mock/preview', image_width: 480, image_height: 270 },
            }),
          300,
        );
        break;
      case 'Logout':
        setTimeout(() => {
          loggedIn = false;
          emit({ type: 'LoggedOut' });
        }, 200);
        break;
      case 'StartVerification':
        emit({ type: 'VerificationStateChanged', data: { type: 'RequestSent' } });
        setTimeout(() => emit({ type: 'VerificationStateChanged', data: { type: 'EmojisReady', data: [['🐶', 'Dog'], ['🔥', 'Fire'], ['🎩', 'Hat'], ['🍕', 'Pizza'], ['🎸', 'Guitar'], ['🚀', 'Rocket'], ['📌', 'Pin']] } }), 600);
        break;
      case 'ConfirmSasMatch':
        emit({ type: 'VerificationStateChanged', data: { type: 'Done' } });
        break;
      case 'EnableRecovery':
        emit({ type: 'RecoveryEnableProgress', data: 'CreatingBackup' });
        setTimeout(() => emit({ type: 'RecoveryEnabled', data: { recovery_key: 'EsTc 9Ya4 q7Kc Zx2p LmN3 8vFd Hb5W Ts1R' } }), 700);
        ok(command.data.request_id, 800);
        break;
      case 'RestoreFromBackup':
        if (command.data.recovery_key.length < 8) fail(command.data.request_id, 'That recovery key is not valid.');
        else {
          ok(command.data.request_id, 400);
          setTimeout(() => emit({ type: 'KeyBackupRestored' }), 400);
        }
        break;
      case 'SendAttachment':
      case 'SetRoomName':
      case 'LeaveRoom':
      case 'ForgetRoom':
      case 'InviteUser':
      case 'JoinRoom':
      case 'KnockRoom':
      case 'KickUser':
      case 'BanUser':
      case 'UnbanUser':
      case 'IgnoreUser':
      case 'UnignoreUser':
      case 'SetRoomTopic':
      case 'SetDisplayName':
      case 'SetRoomNotificationMode':
      case 'ClearRoomNotificationMode':
      case 'SetDefaultNotificationMode':
      case 'JoinCall':
      case 'LeaveCall':
      case 'SendSticker':
        ok(command.data.request_id);
        break;
      case 'OpenDirectMessage':
        setTimeout(() => emit({ type: 'DirectMessageReady', data: { room_id: '!alice:thorny.chat' } }), 200);
        break;
      case 'CreateRoom':
      case 'CreateRoomWith':
        setTimeout(() => emit({ type: 'RoomCreated', data: { room_id: '!lonely:thorny.chat' } }), 200);
        break;
      default:
        break;
    }
  }

  return {
    async attachEvents(handler) {
      onEvent = handler;
    },
    async sessionRestore() {
      if (!loggedIn) return { status: 'none' };
      startSession();
      return { status: 'started', session: { user_id: ME, device_id: 'MOCKDEVICE', homeserver: 'https://thorny.chat/' } };
    },
    async discoverHomeserver(homeserver) {
      await new Promise((r) => setTimeout(r, 250));
      if (/fail/i.test(homeserver)) throw 'could not reach homeserver: connection refused';
      return { supports_password: true, supports_sso: true, sso_providers: [{ id: 'gitlab', name: 'GitLab' }, { id: 'forum', name: 'Forum Account' }] };
    },
    async loginPassword(_homeserver, _username, password) {
      await new Promise((r) => setTimeout(r, 400));
      if (password === 'wrong') throw 'invalid username or password';
      startSession();
      return { user_id: ME, device_id: 'MOCKDEVICE', homeserver: 'https://thorny.chat/' };
    },
    async loginSso() {
      await new Promise((r) => setTimeout(r, 600));
      startSession();
      return { user_id: ME, device_id: 'MOCKDEVICE', homeserver: 'https://thorny.chat/' };
    },
    async dispatch(command) {
      handle(command);
    },
    async ownProfile() {
      return { display_name: 'You', avatar_url: null };
    },
    async sendAttachment(file, meta) {
      const echo = item(ME, { type: 'File', data: { url: 'mxc://mock/upload', filename: file.name, caption: meta.caption } }, Date.now(), { event_id: null });
      diffs(meta.roomId, [{ type: 'PushBack', data: echo }]);
      setTimeout(() => emit({ type: 'CommandSucceeded', data: { request_id: meta.requestId } }), 700);
    },

    async appInfo() {
      return { version: '0.1.0-mock', profile: 'default', logging_enabled: true };
    },
    async getSettings() {
      return structuredClone(settings);
    },
    async setSettings(next) {
      settings = structuredClone(next);
      return structuredClone(settings);
    },
    async themePresets() {
      return [dark, light];
    },
    async fetchTweet() {
      await new Promise((r) => setTimeout(r, 300));
      return {
        text: 'Rust 1.98 is out! Check the release notes for what is new.',
        author: { name: 'Rust Language', screen_name: 'rustlang', avatar_url: null, verification: { verified: true } },
        replies: 120,
        retweets: 3400,
        likes: 15200,
        views: 410000,
        created_timestamp: Math.floor(Date.now() / 1000) - 7200,
        media: null,
        quote: null,
      };
    },
    async fetchSteamApp() {
      return {
        name: 'Half-Life 2',
        short_description: 'The sequel to the critically acclaimed Half-Life.',
        header_image: null,
        is_free: false,
        platforms: { windows: true, mac: true, linux: true },
        price_overview: { discount_percent: 75, initial_formatted: '€9.99', final_formatted: '€2.49' },
        release_date: { coming_soon: false, date: '16 Nov, 2004' },
      };
    },
    async openExternal(url) {
      window.open(url, '_blank', 'noopener');
    },
    async saveMedia() {
      return true;
    },
    async loadProfileFile(file) {
      return profileFiles.get(file) ?? null;
    },
    async saveProfileFile(file, contents) {
      profileFiles.set(file, contents);
    },
    async autostartEnabled() {
      return autostart;
    },
    async setAutostart(enabled) {
      autostart = enabled;
    },
    async readLog() {
      return '2026-10-03T12:00:00Z  INFO mock: nothing to see here\n';
    },
    async clearLogs() {
      return 'Deleted 3 log files, emptied today\'s — 2.0 KB freed.';
    },
    async quit() {},
    async spellStatus() {
      return { available: true, language: 'en-US' };
    },
    async spellCheck(words) {
      return words.map((word) => typo(word) !== undefined);
    },
    async spellSuggest(word) {
      return typo(word) ?? [];
    },
    async spellCorrection(word) {
      return typo(word)?.[0] ?? null;
    },
    async spellAdd(word) {
      dictionary.add(word.toLowerCase());
    },
    async onConnectorEmote() {
      return () => {};
    },

    mediaUrl(mxcUrl) {
      const name = mxcUrl.split('/').pop() ?? 'media';
      const colors: Record<string, string> = { sunset: '#d9622b', sticker1: '#7a5cff', clipthumb: '#2b8ad9', preview: '#c4302b', emoji1: '#2bd98a', emoji2: '#d92b6e' };
      const small = ['sticker1', 'emoji1', 'emoji2'].includes(name);
      return svgFor(name, colors[name] ?? '#3b6', small ? 128 : 640, small ? 128 : 360);
    },
    // The mock's pictures are vector art; there is nothing to upscale.
    upscaledUrl: () => null,
    twemojiUrl: (codepoints) => `https://cdn.jsdelivr.net/gh/jdecked/twemoji@latest/assets/svg/${codepoints}.svg`,
    webImageUrl: (url) => url,
  };
}
