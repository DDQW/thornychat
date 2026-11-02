<script lang="ts">
  import { tick } from 'svelte';
  import { arrowTarget, giveBack, rememberMenuOpener, returnPoint } from '../focus';
  import { ui } from '../stores/ui.svelte';
  import Icon from './Icon.svelte';

  let element = $state<HTMLDivElement>();
  let left = $state(0);
  let top = $state(0);

  // Open at the pointer, but never off-screen; the first item takes focus so
  // the arrow keys work straight away (a menu opened by keyboard has no pointer).
  $effect(() => {
    const menu = ui.menu;
    if (!menu) return;
    const opener = document.activeElement instanceof HTMLElement && document.activeElement !== document.body ? document.activeElement : null;
    rememberMenuOpener(opener);
    const back = returnPoint(opener);
    left = menu.x;
    top = menu.y;
    void tick().then(() => {
      if (!element) return;
      const { width, height } = element.getBoundingClientRect();
      left = Math.max(4, Math.min(menu.x, window.innerWidth - width - 4));
      top = Math.max(4, Math.min(menu.y, window.innerHeight - height - 4));
      items()[0]?.focus({ preventScroll: true });
    });
    // Back to the opener, unless the chosen item moved focus somewhere on purpose.
    return () => {
      giveBack(back, document.querySelector('.menu[role=menu]'));
      // Kept just long enough for a dialog the chosen item opens to find it.
      setTimeout(() => rememberMenuOpener(null));
    };
  });

  const items = () => (element ? [...element.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')] : []);

  function onkeydown(event: KeyboardEvent) {
    if (event.key === 'Tab') {
      event.preventDefault();
      ui.closeMenu();
      return;
    }
    const list = items();
    const next = arrowTarget(event.key, list.indexOf(document.activeElement as HTMLButtonElement), list.length);
    if (next !== null) {
      event.preventDefault();
      list[next]?.focus();
      return;
    }
    // Typing a letter jumps to the next item starting with it.
    if (event.key.length === 1 && !event.ctrlKey && !event.altKey && !event.metaKey) {
      const from = list.indexOf(document.activeElement as HTMLButtonElement);
      const letter = event.key.toLowerCase();
      const match = [...list.slice(from + 1), ...list.slice(0, from + 1)].find((button) => button.textContent?.trim().toLowerCase().startsWith(letter));
      match?.focus();
    }
  }
</script>

<!-- The window moving or losing focus leaves the menu pointing at nothing. -->
<svelte:window onresize={() => ui.menu && ui.closeMenu()} onblur={() => ui.menu && ui.closeMenu()} />

{#if ui.menu}
  <!-- A transparent layer under the menu catches the click that dismisses it. -->
  <div class="catcher" role="presentation" onmousedown={() => ui.closeMenu()} oncontextmenu={(e) => { e.preventDefault(); ui.closeMenu(); }}></div>
  <div class="menu" role="menu" tabindex="-1" bind:this={element} style:left="{left}px" style:top="{top}px" {onkeydown}>
    {#each ui.menu.items as item, index (index)}
      {#if item.separator}<hr />{/if}
      <button
        role={item.checked === undefined ? 'menuitem' : 'menuitemradio'}
        aria-checked={item.checked}
        tabindex="-1"
        class:danger={item.danger}
        class:checkable={ui.menu.items.some((other) => other.checked !== undefined)}
        disabled={item.disabled}
        onclick={() => {
          ui.closeMenu();
          item.onselect();
        }}
      >
        {#if item.checked}<span class="check"><Icon name="check" size={14} /></span>{/if}
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
  .menu button:focus-visible {
    outline-offset: -2px;
  }
  .menu button:focus {
    background: var(--hover);
  }
  /* Items with a check mark line up with the ones without. */
  .checkable {
    position: relative;
    padding-left: 30px;
  }
  .check {
    position: absolute;
    top: 50%;
    left: 9px;
    display: inline-flex;
    color: var(--accent);
    transform: translateY(-50%);
  }
</style>
