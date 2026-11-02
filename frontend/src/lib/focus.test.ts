import { describe, expect, it } from 'vitest';
import { arrowTarget } from './focus';

describe('arrowTarget', () => {
  it('moves through a vertical list and wraps', () => {
    expect(arrowTarget('ArrowDown', 0, 3)).toBe(1);
    expect(arrowTarget('ArrowDown', 2, 3)).toBe(0);
    expect(arrowTarget('ArrowUp', 0, 3)).toBe(2);
    expect(arrowTarget('Home', 2, 3)).toBe(0);
    expect(arrowTarget('End', 0, 3)).toBe(2);
    expect(arrowTarget('ArrowRight', 0, 3)).toBeNull();
    expect(arrowTarget('a', 0, 3)).toBeNull();
  });

  it('can stop at the ends instead of wrapping', () => {
    expect(arrowTarget('ArrowDown', 2, 3, { wrap: false })).toBe(2);
    expect(arrowTarget('ArrowUp', 0, 3, { wrap: false })).toBe(0);
  });

  it('moves a row at a time in a grid', () => {
    expect(arrowTarget('ArrowDown', 1, 10, { columns: 4 })).toBe(5);
    expect(arrowTarget('ArrowDown', 8, 10, { columns: 4 })).toBe(9);
    expect(arrowTarget('ArrowUp', 1, 10, { columns: 4 })).toBe(0);
    expect(arrowTarget('ArrowRight', 9, 10, { columns: 4 })).toBe(0);
  });

  it('uses Left and Right for a horizontal list', () => {
    expect(arrowTarget('ArrowRight', 0, 2, { horizontal: true })).toBe(1);
    expect(arrowTarget('ArrowDown', 0, 2, { horizontal: true })).toBeNull();
  });

  it('does nothing in an empty list', () => {
    expect(arrowTarget('ArrowDown', 0, 0)).toBeNull();
  });
});
