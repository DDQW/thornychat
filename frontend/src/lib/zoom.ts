// Zoom maths for the image lightbox. The image is centred in its stage and
// drawn with `translate(x, y) scale(s)` about that centre, so a point at stage
// offset `p` (from the centre) shows image content at `(p - t) / s`.

export interface View {
  scale: number;
  x: number;
  y: number;
}

export const MIN_SCALE = 0.25;
export const MAX_SCALE = 20;

export function clampScale(scale: number): number {
  if (!Number.isFinite(scale)) return 1;
  return Math.min(MAX_SCALE, Math.max(MIN_SCALE, scale));
}

/**
 * The translation that keeps the content under `cursor` (a stage offset from
 * the centre) fixed while the scale changes from `view.scale` to `next`.
 */
export function zoomAround(view: View, next: number, cursor: { x: number; y: number }): { x: number; y: number } {
  const ratio = next / view.scale;
  return {
    x: cursor.x - (cursor.x - view.x) * ratio,
    y: cursor.y - (cursor.y - view.y) * ratio,
  };
}
