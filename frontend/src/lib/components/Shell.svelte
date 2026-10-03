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
</script>

<div class="shell">
  <Sidebar />
  <main>
    <SecurityBanners />
    {#if rooms.selected && !rooms.selected.is_space}
      <RoomView room={rooms.selected} />
    {:else}
      <div class="empty">
        <h1>ThornyChat</h1>
        <p class="muted">
          {#if rooms.listLoaded && rooms.list.length === 0}
            You're not in any rooms yet. Use the <strong>+</strong> button to start a conversation or join a room.
          {:else}
            Pick a room from the sidebar to start chatting.
          {/if}
        </p>
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
  h1 {
    margin: 0 0 8px;
    font-size: 28px;
  }
</style>
