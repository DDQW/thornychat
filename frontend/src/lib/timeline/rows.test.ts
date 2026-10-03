import { describe, expect, it } from 'vitest';
import type { TimelineItem, TimelineItemContent } from '../bindings';
import { buildRows, GROUP_WINDOW_MS, reuseRows, type RowOptions } from './rows';

const options: RowOptions = { showMembership: true, hideUnreadDivider: false };

function item(id: string | null, sender: string, content: TimelineItemContent, timestamp_ms = 0, extra: Partial<TimelineItem> = {}): TimelineItem {
  return {
    event_id: id,
    sender,
    sender_display_name: null,
    sender_avatar_url: null,
    timestamp_ms,
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
const text = (id: string, sender: string, at = 0, extra: Partial<TimelineItem> = {}) =>
  item(id, sender, { type: 'Text', data: id }, at, extra);
const kinds = (rows: ReturnType<typeof buildRows>) => rows.map((r) => r.kind);
const continuations = (rows: ReturnType<typeof buildRows>) =>
  rows.filter((r) => r.kind === 'message').map((r) => (r.kind === 'message' ? r.continuation : null));

describe('reuseRows', () => {
  const a = text('a', '@x:s', 0);
  const b = text('b', '@x:s', 1000);
  const c = text('c', '@y:s', 2000);

  it('hands back the previous object for every unchanged row', () => {
    const before = buildRows([a, b], options);
    const after = reuseRows(buildRows([a, b, c], options), before);
    expect(after[0]).toBe(before[0]);
    expect(after[1]).toBe(before[1]);
    expect(after[2]).not.toBe(before[1]);
    expect(after).toHaveLength(3);
  });

  it('replaces a row whose item changed (a reaction, an edit)', () => {
    const before = buildRows([a, b], options);
    const edited = { ...b, edited: true };
    const after = reuseRows(buildRows([a, edited], options), before);
    expect(after[0]).toBe(before[0]);
    expect(after[1]).not.toBe(before[1]);
    expect(after[1]?.kind === 'message' && after[1].item).toBe(edited);
  });

  it('replaces a row whose grouping changed', () => {
    // "b" continues "a"'s group until a different sender's message lands between them.
    const before = buildRows([a, b], options);
    const after = reuseRows(buildRows([a, c, b], options), before);
    const row = after.find((r) => r.key === before[1]!.key);
    expect(row).not.toBe(before[1]);
    expect(row?.kind === 'message' && row.continuation).toBe(false);
  });

  it('keeps dividers that did not move', () => {
    const divider = item('d', '', { type: 'DateDivider', data: 'Today' }, 5);
    const before = buildRows([divider, a], options);
    expect(reuseRows(buildRows([divider, a, b], options), before)[0]).toBe(before[0]);
  });
});

describe('buildRows grouping', () => {
  it('groups consecutive messages from one sender', () => {
    const rows = buildRows([text('a', '@x:s', 0), text('b', '@x:s', 1000), text('c', '@x:s', 2000)], options);
    expect(continuations(rows)).toEqual([false, true, true]);
  });

  it('starts a new group for a different sender', () => {
    const rows = buildRows([text('a', '@x:s'), text('b', '@y:s', 1000), text('c', '@x:s', 2000)], options);
    expect(continuations(rows)).toEqual([false, false, false]);
  });

  it('starts a new group after the window elapses', () => {
    const rows = buildRows([text('a', '@x:s', 0), text('b', '@x:s', GROUP_WINDOW_MS - 1), text('c', '@x:s', GROUP_WINDOW_MS * 3)], options);
    expect(continuations(rows)).toEqual([false, true, false]);
  });

  it('a reply always gets its own header', () => {
    const reply = text('b', '@x:s', 1000, { in_reply_to: { event_id: 'a', sender: 'x', snippet: 'hi', image_url: null } });
    expect(continuations(buildRows([text('a', '@x:s'), reply], options))).toEqual([false, false]);
  });

  it('emotes carry the name inline and never share a header', () => {
    const emote = item('b', '@x:s', { type: 'Emote', data: 'waves' }, 1000);
    expect(continuations(buildRows([text('a', '@x:s'), emote, text('c', '@x:s', 2000)], options))).toEqual([false, false, false]);
  });

  it('a date divider ends a group', () => {
    const divider = item(null, '', { type: 'DateDivider', data: 'Monday' }, 500);
    const rows = buildRows([text('a', '@x:s', 0), divider, text('b', '@x:s', 1000)], options);
    expect(kinds(rows)).toEqual(['message', 'date', 'message']);
    expect(continuations(rows)).toEqual([false, false]);
  });
});

describe('buildRows membership and dividers', () => {
  const join = item('m', '@y:s', { type: 'MembershipChange', data: 'y joined the room' });

  it('shows membership lines as system rows and splits the group', () => {
    const rows = buildRows([text('a', '@x:s', 0), join, text('b', '@x:s', 1000)], options);
    expect(kinds(rows)).toEqual(['message', 'system', 'message']);
    expect(continuations(rows)).toEqual([false, false]);
  });

  it('hides membership lines entirely when turned off, without splitting the group', () => {
    const rows = buildRows([text('a', '@x:s', 0), join, text('b', '@x:s', 1000)], { ...options, showMembership: false });
    expect(kinds(rows)).toEqual(['message', 'message']);
    expect(continuations(rows)).toEqual([false, true]);
  });

  it('keeps the unread divider only when something follows it', () => {
    const unread = item(null, '', { type: 'NewMessagesDivider' });
    expect(kinds(buildRows([text('a', '@x:s'), unread, text('b', '@y:s')], options))).toEqual(['message', 'unread', 'message']);
    expect(kinds(buildRows([text('a', '@x:s'), unread], options))).toEqual(['message']);
  });

  it('hides the unread divider once the user has caught up', () => {
    const unread = item(null, '', { type: 'NewMessagesDivider' });
    const rows = buildRows([text('a', '@x:s'), unread, text('b', '@y:s')], { ...options, hideUnreadDivider: true });
    expect(kinds(rows)).toEqual(['message', 'message']);
  });
});

describe('buildRows keys', () => {
  it('uses the event id and falls back to position for local echoes', () => {
    const rows = buildRows([text('a', '@x:s'), item(null, '@x:s', { type: 'Text', data: 'pending' }, 10)], options);
    expect(rows.map((r) => r.key)).toEqual(['a', 'local:1']);
  });

  it('never repeats a key, even when the worker repeats an event id', () => {
    const rows = buildRows([text('a', '@x:s'), text('a', '@x:s', 5)], options);
    expect(new Set(rows.map((r) => r.key)).size).toBe(2);
  });
});
