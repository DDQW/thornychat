<script lang="ts">
  import { backend } from '../api';
  import { player } from '../stores/player.svelte';
  import { ui } from '../stores/ui.svelte';
  import Icon from './Icon.svelte';

  interface Props {
    mxcUrl: string;
    filename: string;
    mimetype: string | null;
    thumbnailUrl: string | null;
  }
  let { mxcUrl, filename, mimetype, thumbnailUrl }: Props = $props();

  const id = $derived(`attachment:${mxcUrl}`);
  const playing = $derived(player.current === id);
  let failed = $state(false);

  async function save() {
    try {
      if (await backend.saveMedia(mxcUrl, filename)) ui.toast(`Saved ${filename}`, 'success', 3000);
    } catch (error) {
      ui.error(String(error));
    }
  }
</script>

<div class="video">
  <div class="stage">
    {#if playing}
      <!-- The shell serves the file straight from the media cache with Range support, so seeking works. -->
      <!-- svelte-ignore a11y_media_has_caption -->
      <video src={backend.mediaUrl(mxcUrl, mimetype ?? undefined)} controls autoplay preload="metadata" playsinline onerror={() => (failed = true)}>
        <track kind="captions" />
      </video>
      {#if failed}<p class="fail">This video can't be played here. Save it to open it elsewhere.</p>{/if}
    {:else}
      <button class="poster" onclick={() => player.play(id)} aria-label="Play {filename}">
        {#if thumbnailUrl}<img src={backend.mediaUrl(thumbnailUrl)} alt="" loading="lazy" />{/if}
        <span class="play"><Icon name="play" size={26} filled /></span>
      </button>
    {/if}
  </div>
  <div class="bar">
    <span class="truncate grow">{filename}</span>
    <button class="icon-btn" onclick={save} aria-label="Save {filename}" title="Save"><Icon name="download" size={16} /></button>
  </div>
</div>

<style>
  .video {
    width: min(448px, 100%);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    overflow: hidden;
  }
  .stage {
    position: relative;
    aspect-ratio: 16 / 9;
    background: #000;
  }
  video {
    display: block;
    width: 100%;
    height: 100%;
  }
  .poster {
    position: relative;
    display: grid;
    width: 100%;
    height: 100%;
    padding: 0;
    border: 0;
    background: #111;
    place-items: center;
    overflow: hidden;
  }
  .poster img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    opacity: 0.85;
  }
  .play {
    position: relative;
    display: grid;
    width: 56px;
    height: 56px;
    border-radius: 50%;
    background: rgb(0 0 0 / 0.65);
    color: #fff;
    place-items: center;
  }
  .poster:hover .play {
    background: var(--accent);
    color: var(--accent-text);
  }
  .fail {
    position: absolute;
    inset: auto 0 0 0;
    margin: 0;
    padding: 8px 12px;
    background: rgb(0 0 0 / 0.75);
    color: #fff;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px 4px 12px;
  }
</style>
