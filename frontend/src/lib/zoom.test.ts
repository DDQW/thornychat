import { describe, expect, it } from 'vitest';
import { clampScale, MAX_SCALE, MIN_SCALE, zoomAround } from './zoom';

describe('lightbox zoom', () => {
  it('clamps the scale and repairs non-finite values', () => {
    expect(clampScale(100)).toBe(MAX_SCALE);
    expect(clampScale(0.001)).toBe(MIN_SCALE);
    expect(clampScale(2)).toBe(2);
    expect(clampScale(NaN)).toBe(1);
    expect(clampScale(Infinity)).toBe(1);
  });

  it('keeps the point under the cursor fixed while zooming', () => {
    const view = { scale: 1.5, x: 30, y: -20 };
    const cursor = { x: 120, y: 80 };
    const next = 4;
    const moved = zoomAround(view, next, cursor);
    // The image coordinate under the cursor, before and after.
    const before = { x: (cursor.x - view.x) / view.scale, y: (cursor.y - view.y) / view.scale };
    const after = { x: (cursor.x - moved.x) / next, y: (cursor.y - moved.y) / next };
    expect(after.x).toBeCloseTo(before.x);
    expect(after.y).toBeCloseTo(before.y);
  });

  it('zooming about the centre does not move a centred image', () => {
    expect(zoomAround({ scale: 1, x: 0, y: 0 }, 3, { x: 0, y: 0 })).toEqual({ x: 0, y: 0 });
  });

  it('is a no-op at an unchanged scale', () => {
    expect(zoomAround({ scale: 2, x: 10, y: 5 }, 2, { x: 99, y: 99 })).toEqual({ x: 10, y: 5 });
  });
});
