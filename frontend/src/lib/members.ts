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

/** The tag a member at `level` belongs to: the highest one at or below it, else null. */
export function tagFor(level: number, tags: readonly PowerLevelTag[]): PowerLevelTag | null {
  let best: PowerLevelTag | null = null;
  for (const tag of tags) if (level >= tag.level && (!best || tag.level > best.level)) best = tag;
  return best;
}

/**
 * A tag colour as it is drawn, in the roster and on names in the timeline:
 * the room's own hue and saturation, so a red team reads as red and a purple
 * team as purple, with only the lightness moved as far as text needs to stay
 * readable on the theme (a pure `#0000ff` is 2.4:1 on the dark theme). A grey
 * tag stays grey. Null for a missing or unsafe colour.
 */
export function tagColor(color: string | null, dark = true): string | null {
  const hex = safeColor(color);
  if (!hex) return null;
  const [r, g, b] = [1, 3, 5].map((at) => parseInt(hex.slice(at, at + 2), 16) / 255) as [number, number, number];
  const [lightness, rawChroma, rawHue] = toOklch(r, g, b);
  const [chroma, hue] = rawChroma < 0.03 ? [0, 0] : [rawChroma, rawHue];
  // Lighter (dark theme) or darker (light theme) in small steps until it
  // clears 4.5:1 on the theme's background, where names sit; a colour that
  // already does stays exactly as the room chose it. (Held to the hover grey
  // instead, a red team came out salmon.)
  const ground = dark ? 0.0103 : 0.8879; // #1A1A1A, #F2F2F2
  const step = dark ? 0.01 : -0.01;
  let l = lightness;
  while (l > 0 && l < 1 && contrast(relativeLuminance(oklchToSrgb(l, chroma, hue)), ground) < 4.5) l += step;
  l = Math.min(1, Math.max(0, l));
  return chroma === 0 ? `oklch(${Math.round(l * 100)}% 0 0)` : `oklch(${Math.round(l * 100)}% ${chroma.toFixed(3)} ${Math.round(hue)})`;
}

function contrast(a: number, b: number): number {
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

/** WCAG relative luminance of 0-1 sRGB channels. */
function relativeLuminance([r, g, b]: [number, number, number]): number {
  const linear = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  return 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
}

/** OKLCH to 0-1 sRGB, clipped into gamut per channel (as Chromium draws an out-of-gamut oklch()). */
export function oklchToSrgb(lightness: number, chroma: number, hue: number): [number, number, number] {
  const a = chroma * Math.cos((hue * Math.PI) / 180);
  const b = chroma * Math.sin((hue * Math.PI) / 180);
  const l = (lightness + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (lightness - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (lightness - 0.0894841775 * a - 1.291485548 * b) ** 3;
  const linear = [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
  const gamma = (c: number) => (c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055);
  return linear.map((c) => Math.min(1, Math.max(0, gamma(Math.min(1, Math.max(0, c)))))) as [number, number, number];
}

/** OKLCH lightness (0-1), chroma and hue (0-360) of an sRGB colour with 0-1 channels. */
export function toOklch(r: number, g: number, b: number): [number, number, number] {
  const linear = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  const [lr, lg, lb] = [linear(r), linear(g), linear(b)];
  const l = Math.cbrt(0.4122214708 * lr + 0.5363325363 * lg + 0.0514459929 * lb);
  const m = Math.cbrt(0.2119034982 * lr + 0.6806995451 * lg + 0.1073969566 * lb);
  const s = Math.cbrt(0.0883024619 * lr + 0.2817188376 * lg + 0.6299787005 * lb);
  const lightness = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
  const a = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
  const bb = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
  return [lightness, Math.hypot(a, bb), ((Math.atan2(bb, a) * 180) / Math.PI + 360) % 360];
}
