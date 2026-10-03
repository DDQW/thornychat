import { describe, expect, it } from 'vitest';
import { reuseUnchanged, withKey } from './record';

const room = (id: string, unread = 0) => ({ room_id: id, name: id, unread_count: unread, topic: null as string | null });
const key = (r: { room_id: string }) => r.room_id;

describe('reuseUnchanged', () => {
  it('keeps the previous object for every entry that did not change', () => {
    const before = [room('a'), room('b'), room('c')];
    const after = reuseUnchanged([room('a'), room('b', 1), room('c')], before, key);
    expect(after[0]).toBe(before[0]);
    expect(after[1]).not.toBe(before[1]);
    expect(after[1]!.unread_count).toBe(1);
    expect(after[2]).toBe(before[2]);
  });

  it('returns the previous list itself when nothing changed', () => {
    const before = [room('a'), room('b')];
    expect(reuseUnchanged([room('a'), room('b')], before, key)).toBe(before);
  });

  it('notices a reorder, an addition and a removal', () => {
    const before = [room('a'), room('b')];
    const reordered = reuseUnchanged([room('b'), room('a')], before, key);
    expect(reordered).not.toBe(before);
    expect(reordered[0]).toBe(before[1]);
    expect(reuseUnchanged([room('a'), room('b'), room('c')], before, key)).toHaveLength(3);
    expect(reuseUnchanged([room('a')], before, key)).toEqual([before[0]]);
  });

  it('treats null and a value as different', () => {
    const before = [room('a')];
    const after = reuseUnchanged([{ ...room('a'), topic: 'hello' }], before, key);
    expect(after[0]).not.toBe(before[0]);
  });
});

describe('withKey', () => {
  it('sets and removes without touching the original', () => {
    const original = { a: 1 };
    expect(withKey(original, 'b', 2)).toEqual({ a: 1, b: 2 });
    expect(withKey(original, 'a', undefined)).toEqual({});
    expect(original).toEqual({ a: 1 });
  });
});
