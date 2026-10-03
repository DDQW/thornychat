// Applies the worker's incremental timeline updates (`TimelineDiff`, mirroring
// the SDK's eyeball_im `VectorDiff`) to the page's item list. Indices are in
// the page's list space: the worker has already dropped the SDK's content-less
// start marker and renumbered.

import type { TimelineDiff, TimelineItem } from '../bindings';

/**
 * Returns a new list with every diff of one batch applied in order. The input
 * is never mutated (the store keeps its list as an immutable value), and a
 * diff that points outside the list is clamped instead of throwing: a bad
 * index means the worker and the page have drifted, and the next `Reset`
 * repairs it — crashing the render loop would not.
 */
export function applyDiffs(items: readonly TimelineItem[], diffs: readonly TimelineDiff[]): TimelineItem[] {
  const list = items.slice();
  for (const diff of diffs) {
    switch (diff.type) {
      case 'Append':
        list.push(...diff.data);
        break;
      case 'Clear':
        list.length = 0;
        break;
      case 'PushFront':
        list.unshift(diff.data);
        break;
      case 'PushBack':
        list.push(diff.data);
        break;
      case 'PopFront':
        list.shift();
        break;
      case 'PopBack':
        list.pop();
        break;
      case 'Insert':
        list.splice(Math.min(Math.max(diff.data.index, 0), list.length), 0, diff.data.item);
        break;
      case 'Set':
        if (diff.data.index >= 0 && diff.data.index < list.length) list[diff.data.index] = diff.data.item;
        break;
      case 'Remove':
        if (diff.data.index >= 0 && diff.data.index < list.length) list.splice(diff.data.index, 1);
        break;
      case 'Truncate':
        if (diff.data.length < list.length) list.length = Math.max(diff.data.length, 0);
        break;
      case 'Reset':
        list.length = 0;
        list.push(...diff.data);
        break;
    }
  }
  return list;
}
