// Turns the worker's flat item list into what the timeline draws: date and
// "new messages" dividers, compact system lines, and messages grouped under one
// header when the same person sends several in a row.

import type { TimelineItem } from '../bindings';

export type Row =
  | { kind: 'date'; key: string; label: string }
  | { kind: 'unread'; key: string }
  | { kind: 'system'; key: string; item: TimelineItem; text: string }
  | { kind: 'message'; key: string; item: TimelineItem; continuation: boolean };

export interface RowOptions {
  /** Show joins/leaves/kicks as system lines (Settings → Chat). */
  showMembership: boolean;
  /** The user has caught up since the divider was placed; hide it. */
  hideUnreadDivider: boolean;
}

/** Messages from one sender closer together than this share a header. */
export const GROUP_WINDOW_MS = 5 * 60 * 1000;

export function buildRows(items: readonly TimelineItem[], options: RowOptions): Row[] {
  const rows: Row[] = [];
  const seen = new Map<string, number>();
  const uniqueKey = (base: string): string => {
    // Svelte's keyed each throws on a duplicate; the worker shouldn't send two
    // items with one event id, but a render crash is the wrong failure mode.
    const count = seen.get(base) ?? 0;
    seen.set(base, count + 1);
    return count === 0 ? base : `${base}#${count}`;
  };

  let previous: TimelineItem | null = null;
  items.forEach((item, index) => {
    const content = item.content;
    switch (content.type) {
      case 'DateDivider':
        rows.push({ kind: 'date', key: uniqueKey(`date:${item.timestamp_ms}:${content.data}`), label: content.data });
        previous = null;
        return;
      case 'NewMessagesDivider':
        if (!options.hideUnreadDivider) rows.push({ kind: 'unread', key: uniqueKey('unread') });
        previous = null;
        return;
      case 'MembershipChange':
        if (options.showMembership) {
          rows.push({ kind: 'system', key: uniqueKey(item.event_id ?? `system:${index}`), item, text: content.data });
        }
        // A membership line splits a run of messages visually, so it ends the group.
        if (options.showMembership) previous = null;
        return;
      default: {
        const continuation = previous !== null && continues(previous, item);
        rows.push({ kind: 'message', key: uniqueKey(item.event_id ?? `local:${index}`), item, continuation });
        previous = item;
      }
    }
  });

  // A divider with nothing after it marks nothing.
  const last = rows.at(-1);
  if (last?.kind === 'unread') rows.pop();
  return rows;
}

function continues(previous: TimelineItem, current: TimelineItem): boolean {
  if (previous.sender !== current.sender) return false;
  if (current.in_reply_to) return false;
  // `/me` lines carry the sender's name inline, so they never share a header.
  if (previous.content.type === 'Emote' || current.content.type === 'Emote') return false;
  const gap = current.timestamp_ms - previous.timestamp_ms;
  return gap >= 0 && gap < GROUP_WINDOW_MS;
}
