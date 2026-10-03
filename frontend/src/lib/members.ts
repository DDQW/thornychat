// Grouping the roster for the members panel. Rooms can define MSC3949
// power-level tags ("Red team", "Purple team", …) that name groups of members;
// without any, the usual Admin / Moderator / Member split applies.

import type { PowerLevelTag, RoomMember } from './bindings';

export interface MemberGroup {
  name: string;
  /** Hex colour the room gave the tag, if any. */
  color: string | null;
  level: number;
  members: RoomMember[];
}

const DEFAULT_TAGS: PowerLevelTag[] = [
  { level: 100, name: 'Admins', color: null },
  { level: 50, name: 'Moderators', color: null },
];

/**
 * Splits `members` into groups. A member belongs to the tag at their power
 * level, falling back to the nearest *lower* defined tag; anyone below every
 * tag lands in a trailing "Members" group. Groups come highest level first and
 * empty ones are dropped. Within a group, people are ordered by display name.
 */
export function groupMembers(members: readonly RoomMember[], tags: readonly PowerLevelTag[]): MemberGroup[] {
  const defined = (tags.length > 0 ? [...tags] : DEFAULT_TAGS).sort((a, b) => b.level - a.level);
  const groups: MemberGroup[] = defined.map((tag) => ({ name: tag.name, color: tag.color, level: tag.level, members: [] }));
  const rest: MemberGroup = { name: 'Members', color: null, level: Number.NEGATIVE_INFINITY, members: [] };

  for (const member of members) {
    const group = groups.find((candidate) => member.power_level >= candidate.level);
    (group ?? rest).members.push(member);
  }

  const byName = (a: RoomMember, b: RoomMember) =>
    (a.display_name || a.user_id).localeCompare(b.display_name || b.user_id, undefined, { sensitivity: 'base' });
  const all = [...groups, rest].filter((group) => group.members.length > 0);
  for (const group of all) group.members.sort(byName);
  return all;
}

/** A tag colour that is safe to put in a style attribute, or null. */
export function safeColor(color: string | null): string | null {
  return color && /^#[0-9a-f]{6}$/i.test(color) ? color : null;
}
