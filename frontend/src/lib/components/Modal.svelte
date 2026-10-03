<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';

  interface Props {
    title: string;
    onclose: () => void;
    width?: string;
    children: Snippet;
    footer?: Snippet;
  }
  let { title, onclose, width = '560px', children, footer }: Props = $props();
</script>

<!-- Clicking the dimmed area closes; Escape is handled globally (App.svelte). -->
<div class="backdrop" role="presentation" onmousedown={(event) => event.target === event.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-label={title} style:width="min({width}, 100%)">
    <header>
      <span class="grow">{title}</span>
      <button class="icon-btn" onclick={onclose} aria-label="Close"><Icon name="x" /></button>
    </header>
    <div class="body">{@render children()}</div>
    {#if footer}<footer>{@render footer()}</footer>{/if}
  </div>
</div>
