import { describe, expect, it } from 'vitest';
import type { RoomSummary } from './bindings';
import { buildSidebar, unreadIn } from './sidebar';

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

const list: RoomSummary[] = [
  room('!a', 'alpha'),
  room('!hq', 'HQ', { is_space: true }),
  room('!dm', 'Alice', { is_dm: true }),
  room('!b', 'beta', { unread_count: 2 }),
  room('!sub', 'Sub space', { is_space: true }),
  room('!c', 'gamma', { unread_count: 5 }),
  room('!lonely', 'Lonely'),
];
const children = { '!hq': ['!b', '!sub', '!missing'], '!sub': ['!c'] };

describe('buildSidebar', () => {
  const model = buildSidebar(list, children, '');

  it('nests joined children under their space and ignores ones we are not in', () => {
    expect(model.spaces.map((s) => s.space.name)).toEqual(['HQ']);
    expect(model.spaces[0]!.rooms.map((r) => r.name)).toEqual(['beta']);
    expect(model.spaces[0]!.spaces.map((s) => s.space.name)).toEqual(['Sub space']);
    expect(model.spaces[0]!.spaces[0]!.rooms.map((r) => r.name)).toEqual(['gamma']);
  });

  it('shows a nested space only under its parent', () => {
    expect(model.spaces.some((s) => s.space.room_id === '!sub')).toBe(false);
  });

  it('keeps nested rooms out of the plain sections', () => {
    expect(model.rooms.map((r) => r.name)).toEqual(['alpha', 'Lonely']);
    expect(model.directMessages.map((r) => r.name)).toEqual(['Alice']);
    expect(model.filtered).toBe(false);
  });

  it('sums unread across a whole subtree', () => {
    expect(unreadIn(model.spaces[0]!)).toBe(7);
  });

  it('survives a space listing itself or a cycle', () => {
    const cyclic = buildSidebar(list, { '!hq': ['!hq', '!sub'], '!sub': ['!hq', '!c'] }, '');
    expect(cyclic.spaces).toHaveLength(1);
    expect(cyclic.spaces[0]!.spaces[0]!.rooms.map((r) => r.name)).toEqual(['gamma']);
  });
});

describe('buildSidebar filtering', () => {
  it('flattens, so a match is never hidden inside a space', () => {
    const model = buildSidebar(list, children, 'GAM');
    expect(model.filtered).toBe(true);
    expect(model.rooms.map((r) => r.name)).toEqual(['gamma']);
    expect(model.spaces).toEqual([]);
  });

  it('matches spaces and DMs too', () => {
    const model = buildSidebar(list, children, 'a');
    expect(model.directMessages.map((r) => r.name)).toEqual(['Alice']);
    expect(model.spaces.map((s) => s.space.name)).toContain('Sub space');
  });

  it('treats a blank filter as no filter', () => {
    expect(buildSidebar(list, children, '   ').filtered).toBe(false);
  });
});
