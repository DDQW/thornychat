import { describe, expect, it } from 'vitest';
import type { PowerLevelTag, RoomMember } from './bindings';
import { groupMembers, safeColor } from './members';

const member = (id: string, name: string, power_level: number): RoomMember => ({ user_id: id, display_name: name, avatar_url: null, power_level });

const roster = [member('@z:s', 'zed', 0), member('@a:s', 'Alice', 100), member('@b:s', 'bob', 50), member('@c:s', 'carol', 0), member('@m:s', 'Mid', 25)];

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
