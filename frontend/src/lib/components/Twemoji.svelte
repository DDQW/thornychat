<script lang="ts">
  import { backend } from '../api';
  import { twemojiCodepoints } from '../emoji';

  interface Props {
    /** A unicode emoji (any grapheme cluster). */
    char: string;
    size?: number;
  }
  let { char, size = 20 }: Props = $props();

  // If the SVG can't be fetched (offline, first use), show the system glyph.
  let failed = $state(false);
  $effect(() => {
    void char;
    failed = false;
  });
</script>

{#if failed}
  <span class="glyph" style:font-size="{size * 0.9}px" style:width="{size}px">{char}</span>
{:else}
  <img
    class="twemoji"
    src={backend.twemojiUrl(twemojiCodepoints(char))}
    alt={char}
    width={size}
    height={size}
    draggable="false"
    loading="lazy"
    onerror={() => (failed = true)}
  />
{/if}

<style>
  .twemoji {
    display: inline-block;
    vertical-align: -0.2em;
  }
  .glyph {
    display: inline-block;
    line-height: 1;
    text-align: center;
  }
</style>
