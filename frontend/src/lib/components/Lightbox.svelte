<script lang="ts">
  import { backend } from '../api';
  import { clampScale, zoomAround } from '../zoom';
  import { ui } from '../stores/ui.svelte';
  import Icon from './Icon.svelte';

  const target = $derived(ui.lightbox);

  let scale = $state(1);
  let x = $state(0);
  let y = $state(0);
  let stage = $state<HTMLDivElement>();
  let dragging = $state(false);
  let drag = { startX: 0, startY: 0, originX: 0, originY: 0, moved: false };

  // A new picture starts fitted and centred.
  $effect(() => {
    void target;
    scale = 1;
    x = 0;
    y = 0;
  });

  /** Wheel listeners are passive by default; zooming has to cancel the page scroll. */
  function nonPassiveWheel(node: HTMLElement, handler: (event: WheelEvent) => void) {
    node.addEventListener('wheel', handler, { passive: false });
    return { destroy: () => node.removeEventListener('wheel', handler) };
  }

  /** Wheel zooms around the cursor, so the point under it stays put. */
  function onwheel(event: WheelEvent) {
    event.preventDefault();
    if (!stage) return;
    const rect = stage.getBoundingClientRect();
    // Offsets are measured from the stage centre, where the image is anchored.
    const cursor = { x: event.clientX - rect.left - rect.width / 2, y: event.clientY - rect.top - rect.height / 2 };
    const next = clampScale(scale * Math.exp(-event.deltaY * 0.0015));
    ({ x, y } = zoomAround({ scale, x, y }, next, cursor));
    scale = next;
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
    x = drag.originX + dx;
    y = drag.originY + dy;
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

  async function save() {
    if (!target) return;
    try {
      if (await backend.saveMedia(target.mxcUrl, target.name)) ui.toast(`Saved ${target.name}`, 'success', 3000);
    } catch (error) {
      ui.error(String(error));
    }
  }
</script>

{#if target}
  <div class="lightbox" role="dialog" aria-modal="true" aria-label="Image viewer: {target.name}">
    <div class="toolbar">
      <span class="grow truncate">{target.name}</span>
      <span class="muted small">{Math.round(scale * 100)}%</span>
      <button class="icon-btn" onclick={reset} title="Fit to window" aria-label="Fit to window"><Icon name="expand" /></button>
      <button class="icon-btn" onclick={save} title="Save to disk" aria-label="Save to disk"><Icon name="download" /></button>
      <button class="icon-btn" onclick={() => (ui.lightbox = null)} title="Close (Esc)" aria-label="Close"><Icon name="x" /></button>
    </div>
    <div class="stage" bind:this={stage} role="presentation" use:nonPassiveWheel={onwheel} onclick={onbackdrop}>
      <img
        src={backend.mediaUrl(target.mxcUrl)}
        alt={target.name}
        draggable="false"
        class:dragging
        style:transform="translate({x}px, {y}px) scale({scale})"
        {onpointerdown}
        {onpointermove}
        {onpointerup}
        onpointercancel={onpointerup}
        ondblclick={() => (scale === 1 ? ((scale = 2), (x = 0), (y = 0)) : reset())}
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
  .stage {
    position: relative;
    display: grid;
    flex: 1 1 auto;
    min-height: 0;
    place-items: center;
    overflow: hidden;
  }
  img {
    max-width: 94%;
    max-height: 94%;
    cursor: grab;
    touch-action: none;
    user-select: none;
    will-change: transform;
  }
  img.dragging {
    cursor: grabbing;
  }
</style>
