// Zoom maths for the image lightbox.
//
// The picture is sized by layout — its CSS width and height are the fitted
// size times `scale` — and only *moved* with a transform. Scaling it with
// `transform: scale()` instead looks the same in code and renders blurry:
// WebView2 keeps the layer rastered at the fitted size and magnifies that
// bitmap, so zooming into a photo never shows more than the fit view had
// (measured 2026-10: a 4 px checkerboard at 1:1 came out about half grey).
//
// `x`/`y` offset the picture's centre from the stage's centre, so a point at
// stage offset `p` (from the centre) shows fitted-image content at
// `(p - t) / scale`.

export interface Size {
  width: number;
  height: number;
}

export interface View {
  /** Multiplier on the fitted size: 1 is the rest view. */
  scale: number;
  x: number;
  y: number;
}

/** How much of the stage a fitted picture may take up. */
export const FIT_MARGIN = 0.94;

/** Zoom stops once one image pixel covers this many screen pixels. */
export const MAX_MAGNIFICATION = 16;

/** Past this magnification the browser's scaling looks soft; time for the Lanczos copy. */
export const UPSCALE_AT = 1.5;
/** The upscaled copy aims for this many times the image's resolution… */
export const UPSCALE_FACTOR = 4;
/** …capped at this long edge (the shell's `upscale::MAX_OUTPUT_EDGE`). */
export const UPSCALE_MAX_EDGE = 4096;
/** Below this gain an upscale is indistinguishable from the browser's own (the shell's `MIN_GAIN`). */
const UPSCALE_MIN_GAIN = 1.25;

const EMPTY: Size = { width: 0, height: 0 };

/**
 * The picture at rest: contain-fit inside the stage (less a margin), never
 * enlarged past its natural CSS size — a sticker isn't blown up to fill the
 * screen until asked.
 */
export function fitSize(natural: Size, stage: Size, margin = FIT_MARGIN): Size {
  if (!(natural.width > 0 && natural.height > 0 && stage.width > 0 && stage.height > 0)) return EMPTY;
  const ratio = Math.min(1, (stage.width * margin) / natural.width, (stage.height * margin) / natural.height);
  return { width: natural.width * ratio, height: natural.height * ratio };
}

/** Screen (device) pixels per image pixel at `scale`: what the % label shows (100% = actual pixels). */
export function magnification(fit: Size, natural: Size, scale: number, dpr: number): number {
  return natural.width > 0 ? (fit.width * scale * dpr) / natural.width : 0;
}

/** The scale at which one image pixel covers `target` screen pixels. */
export function scaleFor(target: number, fit: Size, natural: Size, dpr: number): number {
  return fit.width > 0 ? (target * natural.width) / (dpr * fit.width) : 1;
}

/** Keeps a scale between the rest view and `max`, repairing non-finite values. */
export function clampScale(scale: number, max: number): number {
  if (!Number.isFinite(scale)) return 1;
  return Math.min(Math.max(1, max), Math.max(1, scale));
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

/**
 * Keeps the picture from being dragged off past its own edge: no panning at
 * all along an axis where it fits the stage, and only within the overflow
 * where it doesn't.
 */
export function clampOffset(offset: { x: number; y: number }, displayed: Size, stage: Size): { x: number; y: number } {
  // A fitting axis is pinned at 0 (not -0, which a reset would compare unequal to).
  const clamp = (value: number, overflow: number) => (overflow > 0 ? Math.min(overflow, Math.max(-overflow, value)) : 0);
  return {
    x: clamp(offset.x, (displayed.width - stage.width) / 2),
    y: clamp(offset.y, (displayed.height - stage.height) / 2),
  };
}

/**
 * Where the picture's top-left corner goes in the stage, snapped to whole
 * device pixels: a picture straddling pixel boundaries is resampled by half a
 * pixel and loses exactly the crispness the zoom is for.
 */
export function placement(view: View, displayed: Size, stage: Size, dpr: number): { left: number; top: number } {
  const snap = (value: number) => Math.round(value * dpr) / dpr;
  return {
    left: snap(stage.width / 2 + view.x - displayed.width / 2),
    top: snap(stage.height / 2 + view.y - displayed.height / 2),
  };
}

/** The long edge to ask the shell to upscale to, or null when there is nothing to gain. */
export function upscaleEdge(natural: Size): number | null {
  const long = Math.max(natural.width, natural.height);
  if (!(long > 0)) return null;
  const edge = Math.min(UPSCALE_MAX_EDGE, Math.round(long * UPSCALE_FACTOR));
  return edge >= long * UPSCALE_MIN_GAIN ? edge : null;
}
