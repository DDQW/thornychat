<script lang="ts">
  import { rooms } from '../stores/rooms.svelte';
  import { ui } from '../stores/ui.svelte';
  import DmSearch from './DmSearch.svelte';
  import Lightbox from './Lightbox.svelte';
  import RoomActionDialog from './RoomActionDialog.svelte';
  import RoomView from './RoomView.svelte';
  import SecurityBanners from './SecurityBanners.svelte';
  import SettingsModal from './SettingsModal.svelte';
  import Sidebar from './Sidebar.svelte';
  import SpaceExplorer from './SpaceExplorer.svelte';

  let sidebar = $state<ReturnType<typeof Sidebar>>();

  /**
   * The app's keyboard shortcuts (listed in Settings → Manual):
   * Ctrl+K finds a room, Alt+↑/↓ moves to the room above/below, with Shift
   * only through unread ones, Ctrl+, opens Settings. Off while a modal is up.
   */
  function onkeydown(event: KeyboardEvent) {
    if (ui.modalOpen || event.defaultPrevented || event.isComposing) return;
    const ctrl = event.ctrlKey || event.metaKey;
    if (ctrl && !event.altKey && !event.shiftKey && event.key.toLowerCase() === 'k') {
      event.preventDefault();
      sidebar?.focusFilter();
    } else if (ctrl && !event.altKey && event.key === ',') {
      event.preventDefault();
      ui.openSettings();
    } else if (ctrl && !event.altKey && event.key === '/') {
      event.preventDefault();
      ui.openSettings('manual');
    } else if (event.altKey && !ctrl && (event.key === 'ArrowUp' || event.key === 'ArrowDown')) {
      event.preventDefault();
      stepRoom(event.key === 'ArrowUp' ? -1 : 1, event.shiftKey);
    }
  }

  /** Opens the next room in sidebar order (as drawn, so collapsed spaces and the filter count). */
  function stepRoom(direction: 1 | -1, unreadOnly: boolean) {
    const items = [...document.querySelectorAll<HTMLElement>('.sidebar [data-room-id]')];
    const ids = items.map((item) => item.dataset.roomId!);
    const at = rooms.selectedId ? ids.indexOf(rooms.selectedId) : -1;
    for (let step = 1; step <= ids.length; step++) {
      const index = at < 0 ? (direction > 0 ? step - 1 : ids.length - step) : (at + direction * step + ids.length * step) % ids.length;
      if (unreadOnly && !items[index]!.hasAttribute('data-unread')) continue;
      rooms.select(ids[index]!);
      items[index]!.scrollIntoView({ block: 'nearest' });
      return;
    }
  }
</script>

<svelte:window {onkeydown} />

<!-- Under a modal the page can't be clicked, tabbed to or read by a screen
     reader. (The context menu and toasts live outside the shell, so they keep
     working on top of a modal.) -->
<div class="shell" inert={ui.modalOpen}>
  <Sidebar bind:this={sidebar} />
  <main>
    <SecurityBanners />
    {#if rooms.selected && !rooms.selected.is_space}
      <RoomView room={rooms.selected} />
    {:else}
      <div class="empty">
        <img class="mark" src="/logo.svg" alt="" width="88" height="88" />
        <h1>ThornyChat</h1>
        <p class="muted">
          {#if rooms.listLoaded && rooms.list.length === 0}
            You're not in any rooms yet. Use the <strong>+</strong> button to start a conversation or join a room.
          {:else}
            Pick a room from the sidebar to start chatting.
          {/if}
        </p>
        {#if rooms.listLoaded && rooms.list.length > 0}
          <p class="muted small">Press <kbd>Ctrl</kbd>+<kbd>K</kbd> to find a room, <kbd>Ctrl</kbd>+<kbd>/</kbd> for all shortcuts.</p>
        {/if}
      </div>
    {/if}
  </main>
</div>

{#if ui.settingsOpen}<SettingsModal />{/if}
{#if ui.spaceExplorer}<SpaceExplorer />{/if}
{#if ui.dmSearchOpen}<DmSearch />{/if}
<RoomActionDialog />
<Lightbox />

<style>
  .shell {
    display: flex;
    height: 100%;
  }
  main {
    display: flex;
    flex: 1 1 auto;
    flex-direction: column;
    min-width: 0;
    background: var(--bg);
  }
  .empty {
    display: grid;
    flex: 1 1 auto;
    align-content: center;
    justify-items: center;
    padding: 24px;
    text-align: center;
  }
  .mark {
    margin-bottom: var(--space-4);
    opacity: 0.9;
  }
  h1 {
    margin: 0 0 var(--space-2);
    font-size: var(--text-2xl);
  }
  .empty p {
    max-width: 420px;
    margin: 0 0 var(--space-2);
  }
</style>
