import { describe, expect, it } from 'vitest';
import {
  clampOffset,
  clampScale,
  FIT_MARGIN,
  fitSize,
  magnification,
  placement,
  scaleFor,
  upscaleEdge,
  zoomAround,
} from './zoom';

describe('lightbox zoom', () => {
  it('clamps the scale between the rest view and the maximum, repairing non-finite values', () => {
    expect(clampScale(100, 16)).toBe(16);
    expect(clampScale(0.25, 16)).toBe(1);
    expect(clampScale(2, 16)).toBe(2);
    expect(clampScale(NaN, 16)).toBe(1);
    expect(clampScale(Infinity, 16)).toBe(1);
    // A maximum below the rest view (a picture already shown magnified) pins it there.
    expect(clampScale(3, 0.5)).toBe(1);
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

describe('fitting', () => {
  const stage = { width: 1000, height: 700 };

  it('contain-fits a large picture inside the margin', () => {
    const fit = fitSize({ width: 4000, height: 2000 }, stage);
    expect(fit.width).toBeCloseTo(1000 * FIT_MARGIN);
    expect(fit.height).toBeCloseTo(500 * FIT_MARGIN);
    // A tall one is bound by the height instead.
    expect(fitSize({ width: 1000, height: 4000 }, stage).height).toBeCloseTo(700 * FIT_MARGIN);
  });

  it('never enlarges a small picture at rest', () => {
    expect(fitSize({ width: 120, height: 80 }, stage)).toEqual({ width: 120, height: 80 });
  });

  it('has no size until both the picture and the stage do', () => {
    expect(fitSize({ width: 0, height: 0 }, stage)).toEqual({ width: 0, height: 0 });
    expect(fitSize({ width: 100, height: 100 }, { width: 0, height: 0 })).toEqual({ width: 0, height: 0 });
  });
});

describe('the % label', () => {
  const natural = { width: 4000, height: 2000 };
  const fit = { width: 1000, height: 500 };

  it('counts screen pixels per image pixel, not the size relative to the fit', () => {
    // The fitted 4000 px photo is shown at a quarter of its resolution...
    expect(magnification(fit, natural, 1, 1)).toBeCloseTo(0.25);
    // ...and four times the fit is actual pixels.
    expect(magnification(fit, natural, 4, 1)).toBeCloseTo(1);
  });

  it('accounts for display scaling', () => {
    // At 150% Windows scaling a CSS pixel is 1.5 screen pixels.
    expect(magnification({ width: 100, height: 100 }, { width: 100, height: 100 }, 1, 1.5)).toBeCloseTo(1.5);
    expect(scaleFor(1, { width: 100, height: 100 }, { width: 100, height: 100 }, 1.5)).toBeCloseTo(2 / 3);
  });

  it('inverts: the scale for a magnification gives that magnification back', () => {
    const scale = scaleFor(3, fit, natural, 1.25);
    expect(magnification(fit, natural, scale, 1.25)).toBeCloseTo(3);
  });
});

describe('panning', () => {
  const stage = { width: 1000, height: 700 };

  it('does not pan a picture that fits', () => {
    expect(clampOffset({ x: 300, y: -200 }, { width: 800, height: 600 }, stage)).toEqual({ x: 0, y: 0 });
  });

  it('pans only within the overflow', () => {
    // 400 px wider and 100 px taller than the stage: ±200 and ±50.
    expect(clampOffset({ x: 5000, y: -5000 }, { width: 1400, height: 800 }, stage)).toEqual({ x: 200, y: -50 });
    expect(clampOffset({ x: 120, y: 10 }, { width: 1400, height: 800 }, stage)).toEqual({ x: 120, y: 10 });
  });

  it('snaps the corner to whole device pixels', () => {
    const at = placement({ scale: 1, x: 0.3, y: 0 }, { width: 101, height: 51 }, stage, 1);
    expect(at).toEqual({ left: 450, top: 325 });
    // At 150% scaling a whole device pixel is 2/3 of a CSS pixel.
    const scaled = placement({ scale: 1, x: 0.3, y: 0 }, { width: 101, height: 51 }, stage, 1.5);
    expect((scaled.left * 1.5) % 1).toBeCloseTo(0);
  });
});

describe('upscaling', () => {
  it('asks for four times the resolution, capped at the output edge', () => {
    expect(upscaleEdge({ width: 640, height: 360 })).toBe(2560);
    expect(upscaleEdge({ width: 1600, height: 1200 })).toBe(4096);
  });

  it('does not ask when there is nothing to gain', () => {
    expect(upscaleEdge({ width: 4000, height: 3000 })).toBeNull();
    expect(upscaleEdge({ width: 3600, height: 2000 })).toBeNull();
    expect(upscaleEdge({ width: 0, height: 0 })).toBeNull();
  });
});
