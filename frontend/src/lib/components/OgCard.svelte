<script lang="ts">
  import { backend } from '../api';
  import type { UrlPreview } from '../bindings';
  import { truncate } from '../format';

  interface Props {
    preview: UrlPreview;
  }
  let { preview }: Props = $props();

  // Square images are avatars/icons and sit beside the text; wide ones are
  // banners and go below it.
  const square = $derived(!!preview.image_width && !!preview.image_height && Math.abs(preview.image_width - preview.image_height) < 24 && preview.image_width < 300);
  // A banner's shape is reserved before it loads (the declared one, else the
  // 1.91:1 OpenGraph images are made for): images in the timeline only start
  // loading once they scroll into view, and one growing from nothing there
  // pushed every message below it down.
  const bannerRatio = $derived(preview.image_width && preview.image_height ? `${preview.image_width} / ${preview.image_height}` : '1.91 / 1');
</script>

{#if preview.title || preview.description}
  <article class="og" class:square>
    <div class="text">
      {#if preview.site_name}<div class="site muted small">{preview.site_name}</div>{/if}
      {#if preview.title}<a class="title" href={preview.url} data-external>{truncate(preview.title, 140)}</a>{/if}
      {#if preview.description}<p class="muted small">{truncate(preview.description, 280)}</p>{/if}
    </div>
    {#if preview.image_mxc}
      <img src={backend.mediaUrl(preview.image_mxc)} alt="" loading="lazy" style:aspect-ratio={square ? undefined : bannerRatio} />
    {/if}
  </article>
{/if}

<style>
  .og {
    display: flex;
    flex-direction: column;
    width: min(420px, 100%);
    margin-top: 6px;
    border: 1px solid var(--border);
    border-left: 3px solid var(--accent);
    border-radius: var(--radius-sm);
    background: var(--surface);
    overflow: hidden;
  }
  .og.square {
    flex-direction: row;
    align-items: flex-start;
  }
  .text {
    padding: 8px 12px;
    min-width: 0;
    flex: 1 1 auto;
  }
  .title {
    font-weight: 600;
  }
  p {
    margin: 4px 0 0;
  }
  img {
    display: block;
    width: 100%;
    max-height: 240px;
    object-fit: cover;
  }
  .square img {
    width: 72px;
    height: 72px;
    margin: 8px;
    border-radius: var(--radius-sm);
    object-fit: cover;
  }
</style>
