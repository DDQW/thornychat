<script module lang="ts">
  /** Upscale URLs the shell turned down (animated, vector, already large): not asked again this session. */
  const refused = new Set<string>();
</script>

<script lang="ts">
  import { backend } from '../api';
  import {
    clampOffset,
    clampScale,
    fitSize,
    magnification,
    MAX_MAGNIFICATION,
    placement,
    scaleFor,
    UPSCALE_AT,
    upscaleEdge,
    zoomAround,
    type Size,
  } from '../zoom';
  import { ui } from '../stores/ui.svelte';
  import Icon from './Icon.svelte';

  const target = $derived(ui.lightbox);

  let scale = $state(1);
  let x = $state(0);
  let y = $state(0);
  let stage = $state<HTMLDivElement>();
  let stageSize = $state<Size>({ width: 0, height: 0 });
  /** The original's pixel size, known once it has loaded. */
  let natural = $state<Size>({ width: 0, height: 0 });
  let dpr = $state(window.devicePixelRatio || 1);
  /** The Lanczos copy, once it has decoded; shown in the original's place. */
  let upscaled = $state<string | null>(null);
  let upscaleAsked: string | null = null;
  let dragging = $state(false);
  let drag = { startX: 0, startY: 0, originX: 0, originY: 0, moved: false };

  const original = $derived(target ? backend.mediaUrl(target.mxcUrl) : '');
  const fit = $derived(fitSize(natural, stageSize));
  const maxScale = $derived(scaleFor(MAX_MAGNIFICATION, fit, natural, dpr));
  const displayed = $derived({ width: fit.width * scale, height: fit.height * scale });
  const corner = $derived(placement({ scale, x, y }, displayed, stageSize, dpr));
  const native = $derived(magnification(fit, natural, scale, dpr));
  const pannable = $derived(displayed.width > stageSize.width + 0.5 || displayed.height > stageSize.height + 0.5);

  // A new picture starts fitted and centred, at its own resolution.
  $effect(() => {
    void target;
    scale = 1;
    x = 0;
    y = 0;
    natural = { width: 0, height: 0 };
    upscaled = null;
    upscaleAsked = null;
  });

  // The stage follows the window; the fit (and so every size) follows the stage.
  $effect(() => {
    const element = stage;
    if (!element) return;
    const observer = new ResizeObserver(() => {
      stageSize = { width: element.clientWidth, height: element.clientHeight };
      dpr = window.devicePixelRatio || 1;
    });
    observer.observe(element);
    return () => observer.disconnect();
  });

  // A smaller stage can leave the picture panned past its edge: pull it back.
  $effect(() => {
    const kept = clampOffset({ x, y }, displayed, stageSize);
    if (kept.x !== x || kept.y !== y) ({ x, y } = kept);
  });

  // Magnified past its own resolution, the browser's scaling goes soft: ask
  // the shell for a Lanczos3 copy and swap it in once it has decoded. The
  // picture's size is set by layout, so the swap changes nothing but the
  // pixels behind it.
  $effect(() => {
    if (!target || upscaled || native <= UPSCALE_AT) return;
    const edge = upscaleEdge(natural);
    const url = edge ? backend.upscaledUrl(target.mxcUrl, edge) : null;
    if (!url || url === upscaleAsked || refused.has(url)) return;
    upscaleAsked = url;
    const mxcUrl = target.mxcUrl;
    const copy = new Image();
    copy.src = url;
    copy.decode().then(
      () => {
        if (ui.lightbox?.mxcUrl === mxcUrl) upscaled = url;
      },
      () => refused.add(url),
    );
  });

  function onload(event: Event) {
    const image = event.currentTarget as HTMLImageElement;
    // The upscaled copy loads into the same element; only the original says how big the picture is.
    if (upscaled) return;
    natural = { width: image.naturalWidth, height: image.naturalHeight };
  }

  /** Moves to `next` scale, keeping the content under `cursor` (stage offset from the centre) still. */
  function zoomTo(next: number, cursor = { x: 0, y: 0 }) {
    const clamped = clampScale(next, maxScale);
    const moved = zoomAround({ scale, x, y }, clamped, cursor);
    scale = clamped;
    ({ x, y } = clampOffset(moved, { width: fit.width * clamped, height: fit.height * clamped }, stageSize));
  }

  /** A stage offset from the centre, for a pointer position. */
  function fromCentre(event: MouseEvent) {
    const rect = stage!.getBoundingClientRect();
    return { x: event.clientX - rect.left - rect.width / 2, y: event.clientY - rect.top - rect.height / 2 };
  }

  /** Wheel listeners are passive by default; zooming has to cancel the page scroll. */
  function nonPassiveWheel(node: HTMLElement, handler: (event: WheelEvent) => void) {
    node.addEventListener('wheel', handler, { passive: false });
    return { destroy: () => node.removeEventListener('wheel', handler) };
  }

  function onwheel(event: WheelEvent) {
    event.preventDefault();
    if (!stage || !fit.width) return;
    zoomTo(scale * Math.exp(-event.deltaY * 0.0015), fromCentre(event));
  }

  /** Double-click toggles between the fit and actual pixels, centred on the click. */
  function ondblclick(event: MouseEvent) {
    if (scale > 1.01) return reset();
    const actual = scaleFor(1, fit, natural, dpr);
    // Already at (or past) actual pixels when fitted: double the size instead.
    zoomTo(actual > 1.1 ? actual : 2, fromCentre(event));
  }

  function onpointerdown(event: PointerEvent) {
    if (event.button !== 0) return;
    dragging = true;
    drag = { startX: event.clientX, startY: event.clientY, originX: x, originY: y, moved: false };
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }
  function onpointermove(event: PointerEvent) {
    if (!dragging) return;
    const dx = event.clientX - drag.startX;
    const dy = event.clientY - drag.startY;
    if (Math.abs(dx) + Math.abs(dy) > 3) drag.moved = true;
    ({ x, y } = clampOffset({ x: drag.originX + dx, y: drag.originY + dy }, displayed, stageSize));
  }
  function onpointerup() {
    dragging = false;
  }

  /** A click on the empty backdrop closes; a click on the image (or the end of a drag) does not. */
  function onbackdrop(event: MouseEvent) {
    if (event.target === event.currentTarget && !drag.moved) ui.lightbox = null;
    drag.moved = false;
  }

  function reset() {
    scale = 1;
    x = 0;
    y = 0;
  }

  /** + / − zoom, 0 fits, 1 shows actual pixels. */
  function onkeydown(event: KeyboardEvent) {
    if (!target || event.ctrlKey || event.altKey || event.metaKey) return;
    if ((event.target as Element | null)?.closest?.('input, textarea, [contenteditable="true"]')) return;
    if (event.key === '+' || event.key === '=') zoomTo(scale * 1.25);
    else if (event.key === '-') zoomTo(scale / 1.25);
    else if (event.key === '0') reset();
    else if (event.key === '1') zoomTo(scaleFor(1, fit, natural, dpr));
    else return;
    event.preventDefault();
  }

  async function save() {
    if (!target) return;
    try {
      if (await backend.saveMedia(target.mxcUrl, target.name)) ui.toast(`Saved ${target.name}`, 'success', 3000);
    } catch (error) {
      ui.error(String(error));
    }
  }
</script>

<svelte:window {onkeydown} />

{#if target}
  <div class="lightbox" role="dialog" aria-modal="true" aria-label="Image viewer: {target.name}">
    <div class="toolbar">
      <span class="grow truncate">{target.name}</span>
      {#if natural.width > 0}
        <span class="info small" title="Size on screen relative to the image's own pixels{upscaled ? ' — sharpened with Lanczos3' : ''}">
          {Math.round(native * 100)}%{#if upscaled}<span class="hq">HQ</span>{/if}
        </span>
        <span class="info small dims">{natural.width} × {natural.height}</span>
      {/if}
      <button class="icon-btn" onclick={reset} title="Fit to window (0)" aria-label="Fit to window"><Icon name="expand" /></button>
      <button class="icon-btn" onclick={save} title="Save to disk" aria-label="Save to disk"><Icon name="download" /></button>
      <button class="icon-btn" onclick={() => (ui.lightbox = null)} title="Close (Esc)" aria-label="Close"><Icon name="x" /></button>
    </div>
    <div class="stage" bind:this={stage} role="presentation" use:nonPassiveWheel={onwheel} onclick={onbackdrop}>
      <img
        src={upscaled ?? original}
        alt={target.name}
        draggable="false"
        class:dragging
        class:pannable
        class:ready={fit.width > 0}
        style:width="{displayed.width}px"
        style:height="{displayed.height}px"
        style:transform="translate({corner.left}px, {corner.top}px)"
        {onload}
        {onpointerdown}
        {onpointermove}
        {onpointerup}
        onpointercancel={onpointerup}
        {ondblclick}
      />
    </div>
  </div>
{/if}

<style>
  .lightbox {
    position: fixed;
    inset: 0;
    z-index: 60;
    display: flex;
    flex-direction: column;
    background: rgb(0 0 0 / 0.9);
    color: #fff;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    background: rgb(0 0 0 / 0.5);
  }
  .toolbar .icon-btn {
    color: #fff;
  }
  /* The backdrop is black in either theme, so these don't use the theme's muted/accent colours. */
  .info {
    color: rgb(255 255 255 / 0.7);
  }
  .hq {
    margin-left: 6px;
    padding: 0 4px;
    border-radius: 3px;
    background: rgb(255 255 255 / 0.15);
    color: #fff;
    font-weight: 600;
  }
  .dims {
    font-variant-numeric: tabular-nums;
  }
  .stage {
    position: relative;
    flex: 1 1 auto;
    min-height: 0;
    overflow: hidden;
  }
  /* Sized by layout and only moved by the transform (see zoom.ts for why). */
  img {
    position: absolute;
    top: 0;
    left: 0;
    max-width: none;
    visibility: hidden;
    cursor: zoom-in;
    touch-action: none;
    user-select: none;
    will-change: transform;
  }
  img.ready {
    visibility: visible;
  }
  img.pannable {
    cursor: grab;
  }
  img.dragging {
    cursor: grabbing;
  }
</style>
