<script lang="ts">
  import { backend } from '../api';
  import { truncate } from '../format';
  import type { SteamAppData } from '../types';

  interface Props {
    app: SteamAppData;
    url: string;
  }
  let { app, url }: Props = $props();

  const platforms = $derived([app.platforms?.windows && 'Windows', app.platforms?.mac && 'macOS', app.platforms?.linux && 'Linux'].filter(Boolean).join(' · '));
</script>

<article class="steam">
  {#if app.header_image}
    <img class="art" src={backend.webImageUrl(app.header_image)} alt="" loading="lazy" />
  {/if}
  <div class="info">
    <a class="name" href={url} data-external>{app.name}</a>
    {#if app.short_description}<p class="muted small">{truncate(app.short_description, 200)}</p>{/if}
    <div class="meta small">
      {#if app.release_date}<span class="muted">{app.release_date.coming_soon ? 'Coming ' : ''}{app.release_date.date}</span>{/if}
      {#if platforms}<span class="muted">{platforms}</span>{/if}
      <span class="price">
        {#if app.is_free}
          Free to play
        {:else if app.price_overview}
          {#if app.price_overview.discount_percent > 0}
            <span class="discount">−{app.price_overview.discount_percent}%</span>
            <s class="muted">{app.price_overview.initial_formatted}</s>
          {/if}
          {app.price_overview.final_formatted}
        {/if}
      </span>
    </div>
  </div>
</article>

<style>
  .steam {
    width: min(460px, 100%);
    margin-top: 6px;
    border: 1px solid var(--border);
    border-left: 3px solid #66c0f4;
    border-radius: var(--radius-sm);
    background: var(--surface);
    overflow: hidden;
  }
  .art {
    display: block;
    width: 100%;
    aspect-ratio: 460 / 215;
    object-fit: cover;
  }
  .info {
    padding: 8px 12px 10px;
  }
  .name {
    font-weight: 700;
  }
  p {
    margin: 4px 0 6px;
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    align-items: center;
  }
  .price {
    margin-left: auto;
    font-weight: 600;
  }
  .discount {
    margin-right: 4px;
    padding: 1px 5px;
    border-radius: 3px;
    background: #4c6b22;
    color: #beee11;
  }
</style>
