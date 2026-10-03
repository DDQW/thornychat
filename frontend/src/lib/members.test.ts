import { describe, expect, it } from 'vitest';
import type { PowerLevelTag, RoomMember } from './bindings';
import { groupMembers, HEADER_HEIGHT, layoutRoster, MEMBER_HEIGHT, safeColor, searchMembers, visibleRows } from './members';

const member = (id: string, name: string, power_level: number): RoomMember => ({ user_id: id, display_name: name, avatar_url: null, power_level });

const roster = [member('@z:s', 'zed', 0), member('@a:s', 'Alice', 100), member('@b:s', 'bob', 50), member('@c:s', 'carol', 0), member('@m:s', 'Mid', 25)];

describe('the windowed roster', () => {
  const groups = groupMembers(
    Array.from({ length: 1000 }, (_, i) => member(`@u${i}:s`, `user ${String(i).padStart(4, '0')}`, i < 2 ? 100 : 0)),
    [],
  );
  const { rows, height } = layoutRoster(groups);

  it('lays the groups out as headers followed by their members', () => {
    expect(rows.map((r) => r.kind).slice(0, 4)).toEqual(['header', 'member', 'member', 'header']);
    expect(height).toBe(2 * HEADER_HEIGHT + 1000 * MEMBER_HEIGHT);
    expect(rows.at(-1)!.top).toBe(height - MEMBER_HEIGHT);
  });

  it('draws only the rows near the viewport', () => {
    const visible = visibleRows(rows, 18_000, 600, 0);
    expect(visible.length).toBeLessThanOrEqual(Math.ceil(600 / MEMBER_HEIGHT) + 1);
    expect(visible[0]!.top).toBeLessThanOrEqual(18_000);
    expect(visible.at(-1)!.top + MEMBER_HEIGHT).toBeGreaterThanOrEqual(18_600);
  });

  it('covers the very top and the very bottom', () => {
    expect(visibleRows(rows, 0, 400, 0)[0]).toBe(rows[0]);
    expect(visibleRows(rows, height - 400, 400, 0).at(-1)).toBe(rows.at(-1));
  });

  it('adds the overscan on both sides', () => {
    const tight = visibleRows(rows, 10_000, 500, 0);
    const loose = visibleRows(rows, 10_000, 500, 300);
    expect(loose.length).toBeGreaterThan(tight.length);
    expect(loose[0]!.top).toBeLessThan(tight[0]!.top);
  });
});

describe('searchMembers', () => {
  it('matches display names and user ids, case-insensitively', () => {
    expect(searchMembers(roster, 'ALI').map((m) => m.user_id)).toEqual(['@a:s']);
    expect(searchMembers(roster, '@b:').map((m) => m.user_id)).toEqual(['@b:s']);
    expect(searchMembers(roster, 'nobody')).toEqual([]);
  });

  it('stops at the limit and returns everyone for a blank query', () => {
    expect(searchMembers(roster, 's', 2)).toHaveLength(2);
    expect(searchMembers(roster, '  ')).toHaveLength(roster.length);
  });

  it('follows a new roster rather than a cached one', () => {
    const renamed = roster.map((m) => (m.user_id === '@z:s' ? { ...m, display_name: 'Zora' } : m));
    expect(searchMembers(roster, 'zora')).toEqual([]);
    expect(searchMembers(renamed, 'zora').map((m) => m.user_id)).toEqual(['@z:s']);
  });
});

describe('groupMembers', () => {
  it('uses admins, moderators and members when the room defines no tags', () => {
    const groups = groupMembers(roster, []);
    expect(groups.map((g) => g.name)).toEqual(['Admins', 'Moderators', 'Members']);
    expect(groups[2]!.members.map((m) => m.display_name)).toEqual(['carol', 'Mid', 'zed']);
  });

  it('puts a member in the nearest tag at or below their level', () => {
    const tags: PowerLevelTag[] = [
      { level: 100, name: 'Owners', color: '#ff0000' },
      { level: 20, name: 'Regulars', color: null },
    ];
    const groups = groupMembers(roster, tags);
    expect(groups.map((g) => [g.name, g.members.map((m) => m.display_name)])).toEqual([
      ['Owners', ['Alice']],
      ['Regulars', ['bob', 'Mid']],
      ['Members', ['carol', 'zed']],
    ]);
    expect(groups[0]!.color).toBe('#ff0000');
  });

  it('drops empty groups and orders names case-insensitively', () => {
    const groups = groupMembers([member('@b:s', 'bravo', 0), member('@a:s', 'Alpha', 0)], []);
    expect(groups).toHaveLength(1);
    expect(groups[0]!.members.map((m) => m.display_name)).toEqual(['Alpha', 'bravo']);
  });

  it('falls back to the user id for people with no display name', () => {
    const groups = groupMembers([member('@b:s', '', 0), member('@a:s', '', 0)], []);
    expect(groups[0]!.members.map((m) => m.user_id)).toEqual(['@a:s', '@b:s']);
  });

  it('copes with an empty roster and does not mutate its inputs', () => {
    expect(groupMembers([], [])).toEqual([]);
    const tags: PowerLevelTag[] = [{ level: 0, name: 'Everyone', color: null }, { level: 100, name: 'Top', color: null }];
    const snapshot = JSON.stringify(tags);
    groupMembers(roster, tags);
    expect(JSON.stringify(tags)).toBe(snapshot);
  });
});

describe('safeColor', () => {
  it('only passes plain hex colours', () => {
    expect(safeColor('#a1B2c3')).toBe('#a1B2c3');
    for (const bad of ['red', '#fff', 'url(javascript:1)', '#12345g', null]) expect(safeColor(bad)).toBeNull();
  });
});
