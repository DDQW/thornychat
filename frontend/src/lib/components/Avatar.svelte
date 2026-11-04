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
  class="avatar {square ? 'square' : 'squircle'}"
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
  /* The placeholder: the person's hue at one lightness for every hue
     (OKLCH), with white initials that read on all of them. */
  .avatar {
    display: inline-grid;
    flex: 0 0 auto;
    place-items: center;
    background: oklch(56% 0.12 var(--hue));
    color: #fff;
    font-weight: 600;
    letter-spacing: 0.01em;
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
  /* A picture with a white or black edge still has an outline on either theme. */
  .avatar:has(img) {
    box-shadow: inset 0 0 0 1px rgb(127 127 127 / 0.2);
  }
</style>
