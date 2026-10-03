import { describe, expect, it } from 'vitest';
import type { TimelineDiff, TimelineItem } from '../bindings';
import { applyDiffs } from './diff';

function item(id: string): TimelineItem {
  return {
    event_id: id,
    sender: '@a:b',
    sender_display_name: null,
    sender_avatar_url: null,
    timestamp_ms: 0,
    content: { type: 'Text', data: id },
    formatted_body: null,
    shield: null,
    reactions: [],
    thread_root: null,
    thread_reply_count: null,
    read_by: [],
    in_reply_to: null,
    edited: false,
    send_failed: null,
  };
}

const ids = (items: TimelineItem[]) => items.map((i) => i.event_id);
const list = (...names: string[]) => names.map(item);

describe('applyDiffs', () => {
  it('appends, pushes and pops at both ends', () => {
    let items = applyDiffs([], [{ type: 'Append', data: list('b', 'c') }]);
    items = applyDiffs(items, [{ type: 'PushFront', data: item('a') }, { type: 'PushBack', data: item('d') }]);
    expect(ids(items)).toEqual(['a', 'b', 'c', 'd']);
    items = applyDiffs(items, [{ type: 'PopFront' }, { type: 'PopBack' }]);
    expect(ids(items)).toEqual(['b', 'c']);
  });

  it('inserts, replaces and removes by index', () => {
    let items = list('a', 'c');
    items = applyDiffs(items, [{ type: 'Insert', data: { index: 1, item: item('b') } }]);
    expect(ids(items)).toEqual(['a', 'b', 'c']);
    items = applyDiffs(items, [{ type: 'Set', data: { index: 0, item: item('A') } }]);
    expect(ids(items)).toEqual(['A', 'b', 'c']);
    items = applyDiffs(items, [{ type: 'Remove', data: { index: 1 } }]);
    expect(ids(items)).toEqual(['A', 'c']);
  });

  it('truncates, clears and resets', () => {
    let items = list('a', 'b', 'c', 'd');
    items = applyDiffs(items, [{ type: 'Truncate', data: { length: 2 } }]);
    expect(ids(items)).toEqual(['a', 'b']);
    items = applyDiffs(items, [{ type: 'Reset', data: list('x', 'y', 'z') }]);
    expect(ids(items)).toEqual(['x', 'y', 'z']);
    items = applyDiffs(items, [{ type: 'Clear' }]);
    expect(items).toEqual([]);
  });

  it('applies a batch in order, each diff seeing the previous result', () => {
    const diffs: TimelineDiff[] = [
      { type: 'Append', data: list('a', 'b') },
      { type: 'Insert', data: { index: 1, item: item('x') } },
      { type: 'Remove', data: { index: 0 } },
    ];
    expect(ids(applyDiffs([], diffs))).toEqual(['x', 'b']);
  });

  it('never mutates its input', () => {
    const before = list('a', 'b');
    const snapshot = [...before];
    applyDiffs(before, [{ type: 'PopBack' }, { type: 'Clear' }]);
    expect(before).toEqual(snapshot);
  });

  it('clamps out-of-range indices instead of throwing', () => {
    let items = list('a');
    items = applyDiffs(items, [{ type: 'Insert', data: { index: 99, item: item('z') } }]);
    expect(ids(items)).toEqual(['a', 'z']);
    items = applyDiffs(items, [{ type: 'Set', data: { index: 50, item: item('q') } }]);
    items = applyDiffs(items, [{ type: 'Remove', data: { index: -1 } }]);
    items = applyDiffs(items, [{ type: 'Truncate', data: { length: 99 } }]);
    expect(ids(items)).toEqual(['a', 'z']);
  });

  it('handles pops on an empty list', () => {
    expect(applyDiffs([], [{ type: 'PopFront' }, { type: 'PopBack' }])).toEqual([]);
  });
});
