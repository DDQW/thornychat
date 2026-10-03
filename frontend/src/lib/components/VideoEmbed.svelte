<script lang="ts">
  import { backend } from '../api';
  import { embedUrl, isIframePlayer, PLATFORM_ACCENT, PLATFORM_LABEL, videoMimeFor, type EmbedVideo } from '../embeds';
  import { player } from '../stores/player.svelte';
  import { previews } from '../stores/previews.svelte';
  import Icon from './Icon.svelte';

  interface Props {
    video: EmbedVideo;
  }
  let { video }: Props = $props();

  const id = $derived(`embed:${video.sourceUrl}`);
  const playing = $derived(player.current === id);
  const preview = $derived(previews.og[video.sourceUrl]);
  const og = $derived(preview?.state === 'ready' ? preview.value : null);
  const title = $derived(og?.title ?? (video.platform === 'file' ? decodeURIComponent(video.sourceUrl.split('/').pop()?.split('?')[0] ?? 'Video') : `${PLATFORM_LABEL[video.platform]} video`));
</script>

<div class="embed" style:--accent-strip={PLATFORM_ACCENT[video.platform]}>
  <div class="head">
    <span class="platform">{PLATFORM_LABEL[video.platform]}</span>
    <a class="title truncate" href={video.sourceUrl} data-external>{title}</a>
    {#if playing}<button class="icon-btn" onclick={() => player.stop(id)} aria-label="Close player"><Icon name="x" size={14} /></button>{/if}
  </div>

  <div class="stage">
    {#if playing}
      {#if isIframePlayer(video)}
        <!-- referrerpolicy="origin" pins exactly what YouTube requires the request to carry. -->
        <iframe
          title={title}
          src={embedUrl(video)}
          allow="autoplay; encrypted-media; fullscreen; picture-in-picture"
          allowfullscreen
          referrerpolicy="origin"
          sandbox="allow-scripts allow-same-origin allow-presentation allow-popups allow-popups-to-escape-sandbox"
        ></iframe>
      {:else}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video src={embedUrl(video)} controls autoplay preload="metadata" playsinline>
          <track kind="captions" />
        </video>
      {/if}
    {:else}
      <button class="poster" onclick={() => player.play(id)} aria-label="Play {title}">
        {#if og?.image_mxc}<img src={backend.mediaUrl(og.image_mxc)} alt="" loading="lazy" />{/if}
        <span class="play"><Icon name="play" size={26} filled /></span>
      </button>
    {/if}
  </div>
</div>

<style>
  .embed {
    width: min(448px, 100%);
    margin-top: 6px;
    border: 1px solid var(--border);
    border-left: 3px solid var(--accent-strip);
    border-radius: var(--radius-sm);
    background: var(--surface);
    overflow: hidden;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
  }
  .platform {
    color: var(--muted);
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }
  .title {
    flex: 1 1 auto;
    min-width: 0;
    font-weight: 600;
  }
  .stage {
    aspect-ratio: 16 / 9;
    background: #000;
  }
  iframe,
  video {
    display: block;
    width: 100%;
    height: 100%;
    border: 0;
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
    background: var(--accent-strip);
  }
</style>
