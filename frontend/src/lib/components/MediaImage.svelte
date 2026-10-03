<script lang="ts">
  import { backend } from '../api';
  import { ui } from '../stores/ui.svelte';

  interface Props {
    mxcUrl: string;
    name: string;
    /** Sender-declared dimensions, used to reserve the footprint before the bytes arrive. */
    width?: number | null;
    height?: number | null;
    /** Largest edge to draw at, in CSS pixels. */
    max?: number;
  }
  let { mxcUrl, name, width = null, height = null, max = 420 }: Props = $props();

  let failed = $state(false);

  // Reserving the exact aspect ratio up front is what stops every image load
  // from reflowing the timeline and yanking the scroll position around.
  const known = $derived(!!width && !!height);
  const drawWidth = $derived(known ? Math.min(width!, max) : undefined);
  const drawMax = $derived(known ? Math.min(max, (max * width!) / Math.max(height!, 1)) : max);
</script>

{#if failed}
  <div class="broken muted"><span>Couldn't load {name}</span></div>
{:else}
  <button
    class="frame"
    style:width={drawWidth ? `${drawWidth}px` : undefined}
    style:max-width="min(100%, {drawMax}px)"
    style:aspect-ratio={known ? `${width} / ${height}` : undefined}
    onclick={() => (ui.lightbox = { mxcUrl, name })}
    aria-label="View {name}"
  >
    <img src={backend.mediaUrl(mxcUrl)} alt={name} loading="lazy" decoding="async" draggable="false" onerror={() => (failed = true)} />
  </button>
{/if}

<style>
  .frame {
    display: block;
    max-height: 420px;
    padding: 0;
    border: 0;
    border-radius: var(--radius-sm);
    background: var(--surface);
    overflow: hidden;
    cursor: zoom-in;
  }
  img {
    display: block;
    width: 100%;
    height: 100%;
    max-height: 420px;
    object-fit: contain;
  }
  .broken {
    padding: 14px;
    border: 1px dashed var(--border);
    border-radius: var(--radius-sm);
  }
</style>
