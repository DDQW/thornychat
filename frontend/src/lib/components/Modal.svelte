<script lang="ts">
  import type { Snippet } from 'svelte';
  import { trapFocus } from '../focus';
  import Icon from './Icon.svelte';

  interface Props {
    title: string;
    onclose: () => void;
    width?: string;
    children: Snippet;
    footer?: Snippet;
  }
  let { title, onclose, width = '560px', children, footer }: Props = $props();
  const titleId = `dialog-${Math.random().toString(36).slice(2, 9)}`;
</script>

<!-- Clicking the dimmed area closes; Escape is handled globally (App.svelte).
     Focus moves in, stays in, and goes back to the opener (focus.ts). -->
<div class="backdrop" role="presentation" onmousedown={(event) => event.target === event.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby={titleId} style:width="min({width}, 100%)" use:trapFocus>
    <header>
      <h2 class="grow title" id={titleId}>{title}</h2>
      <button class="icon-btn" onclick={onclose} aria-label="Close"><Icon name="x" /></button>
    </header>
    <div class="body">{@render children()}</div>
    {#if footer}<footer>{@render footer()}</footer>{/if}
  </div>
</div>

<style>
  .title {
    margin: 0;
    font-size: inherit;
  }
</style>
