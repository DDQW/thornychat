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
/**
 * One collator for every comparison. `a.localeCompare(b, undefined, options)`
 * builds a fresh ICU collator per call: 700 ms to sort a 20 000-member roster,
 * against ~20 ms like this.
 */
const byNameCollator = new Intl.Collator(undefined, { sensitivity: 'base' });

export function groupMembers(members: readonly RoomMember[], tags: readonly PowerLevelTag[]): MemberGroup[] {
  const defined = (tags.length > 0 ? [...tags] : DEFAULT_TAGS).sort((a, b) => b.level - a.level);
  const groups: MemberGroup[] = defined.map((tag) => ({ name: tag.name, color: tag.color, level: tag.level, members: [] }));
  const rest: MemberGroup = { name: 'Members', color: null, level: Number.NEGATIVE_INFINITY, members: [] };

  for (const member of members) {
    const group = groups.find((candidate) => member.power_level >= candidate.level);
    (group ?? rest).members.push(member);
  }

  const byName = (a: RoomMember, b: RoomMember) => byNameCollator.compare(a.display_name || a.user_id, b.display_name || b.user_id);
  const all = [...groups, rest].filter((group) => group.members.length > 0);
  for (const group of all) group.members.sort(byName);
  return all;
}

// --- the panel's list, windowed ---
// Big public rooms have tens of thousands of members; drawing every row took
// 2.2 s and ~150 MB for 20 000. The panel draws only what's on screen, which
// fixed row heights make a matter of arithmetic.

/** Row heights in the members panel, in CSS pixels (MembersPanel.svelte sets them to match). */
export const HEADER_HEIGHT = 32;
export const MEMBER_HEIGHT = 36;

export type RosterRow =
  | { kind: 'header'; key: string; top: number; group: MemberGroup }
  | { kind: 'member'; key: string; top: number; group: MemberGroup; member: RoomMember };

/** The groups flattened into positioned rows, plus the height of the whole list. */
export function layoutRoster(groups: readonly MemberGroup[]): { rows: RosterRow[]; height: number } {
  const rows: RosterRow[] = [];
  let top = 0;
  for (const group of groups) {
    rows.push({ kind: 'header', key: `group:${group.name}:${group.level}`, top, group });
    top += HEADER_HEIGHT;
    for (const member of group.members) {
      rows.push({ kind: 'member', key: member.user_id, top, group, member });
      top += MEMBER_HEIGHT;
    }
  }
  return { rows, height: top };
}

const rowHeight = (row: RosterRow) => (row.kind === 'header' ? HEADER_HEIGHT : MEMBER_HEIGHT);

/** The rows that intersect `[scrollTop - overscan, scrollTop + viewport + overscan]`. */
export function visibleRows(rows: readonly RosterRow[], scrollTop: number, viewport: number, overscan = 300): RosterRow[] {
  const from = scrollTop - overscan;
  const to = scrollTop + viewport + overscan;
  // First row whose bottom edge is below `from`: rows are sorted by `top`.
  let low = 0;
  let high = rows.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    const row = rows[middle]!;
    if (row.top + rowHeight(row) <= from) low = middle + 1;
    else high = middle;
  }
  const visible: RosterRow[] = [];
  for (let index = low; index < rows.length && rows[index]!.top < to; index++) visible.push(rows[index]!);
  return visible;
}

const searchKeys = new WeakMap<readonly RoomMember[], string[]>();

/**
 * The members whose display name or user id contains `query` (case-insensitive),
 * at most `limit` of them. The lowercased search text is built once per roster
 * update rather than once per member per keystroke.
 */
export function searchMembers(members: readonly RoomMember[], query: string, limit = Infinity): RoomMember[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return members.slice(0, limit);
  let keys = searchKeys.get(members);
  if (!keys) {
    keys = members.map((member) => `${member.display_name} ${member.user_id}`.toLowerCase());
    searchKeys.set(members, keys);
  }
  const found: RoomMember[] = [];
  for (let index = 0; index < members.length && found.length < limit; index++) {
    if (keys[index]!.includes(needle)) found.push(members[index]!);
  }
  return found;
}

/** A tag colour that is safe to put in a style attribute, or null. */
export function safeColor(color: string | null): string | null {
  return color && /^#[0-9a-f]{6}$/i.test(color) ? color : null;
}
