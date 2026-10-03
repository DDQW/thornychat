<script lang="ts">
  import { tick } from 'svelte';
  import { ui } from '../stores/ui.svelte';

  let element = $state<HTMLDivElement>();
  let left = $state(0);
  let top = $state(0);

  // Open at the pointer, but never off-screen.
  $effect(() => {
    const menu = ui.menu;
    if (!menu) return;
    left = menu.x;
    top = menu.y;
    void tick().then(() => {
      if (!element) return;
      const { width, height } = element.getBoundingClientRect();
      left = Math.max(4, Math.min(menu.x, window.innerWidth - width - 4));
      top = Math.max(4, Math.min(menu.y, window.innerHeight - height - 4));
    });
  });
</script>

{#if ui.menu}
  <!-- A transparent layer under the menu catches the click that dismisses it. -->
  <div class="catcher" role="presentation" onmousedown={() => ui.closeMenu()} oncontextmenu={(e) => { e.preventDefault(); ui.closeMenu(); }}></div>
  <div class="menu" role="menu" bind:this={element} style:left="{left}px" style:top="{top}px">
    {#each ui.menu.items as item, index (index)}
      {#if item.separator}<hr />{/if}
      <button
        role="menuitem"
        class:danger={item.danger}
        disabled={item.disabled}
        onclick={() => {
          ui.closeMenu();
          item.onselect();
        }}
      >
        {item.label}
      </button>
    {/each}
  </div>
{/if}

<style>
  .catcher {
    position: fixed;
    inset: 0;
    z-index: 79;
  }
</style>
