<script lang="ts">
  import { backend } from '../api';
  import { initialsOf, nameHue } from '../format';

  interface Props {
    /** `mxc://` URI of the picture, if there is one. */
    src?: string | null;
    name: string;
    /** What the placeholder colour is derived from (defaults to the name). */
    seed?: string;
    size?: number;
    square?: boolean;
  }
  let { src = null, name, seed, size = 36, square = false }: Props = $props();

  let failed = $state(false);
  const url = $derived(src && !failed ? backend.mediaUrl(src) : '');
  // A new picture gets a fresh chance to load.
  $effect(() => {
    void src;
    failed = false;
  });
</script>

<span
  class="avatar"
  class:square
  style:width="{size}px"
  style:height="{size}px"
  style:font-size="{Math.max(10, Math.round(size * 0.4))}px"
  style:--hue={nameHue(seed ?? name)}
>
  {#if url}
    <img src={url} alt="" loading="lazy" draggable="false" onerror={() => (failed = true)} />
  {:else}
    {initialsOf(name)}
  {/if}
</span>

<style>
  .avatar {
    display: inline-grid;
    flex: 0 0 auto;
    place-items: center;
    border-radius: 50%;
    background: hsl(var(--hue) 35% 32%);
    color: hsl(var(--hue) 70% 90%);
    font-weight: 600;
    overflow: hidden;
    user-select: none;
  }
  .avatar.square {
    border-radius: var(--radius-sm);
  }
  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
</style>
