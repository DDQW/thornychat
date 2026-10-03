<script lang="ts">
  import { createRoom } from '../actions';
  import { friendlyUserId } from '../format';
  import { buildSidebar } from '../sidebar';
  import { rooms } from '../stores/rooms.svelte';
  import { session } from '../stores/session.svelte';
  import { ui } from '../stores/ui.svelte';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';
  import RoomItem from './RoomItem.svelte';
  import SpaceTree from './SpaceTree.svelte';

  const model = $derived(buildSidebar(rooms.list, rooms.spaceChildren, rooms.filter));
  const name = $derived(session.profile?.display_name || (session.info ? friendlyUserId(session.info.user_id) : ''));
  const syncLabel = $derived(
    session.sync.type === 'Error' ? `Sync error: ${session.sync.data}` : { Connecting: 'Connecting…', Syncing: 'Connected', Offline: 'Offline' }[session.sync.type],
  );
  const syncTone = $derived(session.sync.type === 'Syncing' ? 'ok' : session.sync.type === 'Connecting' ? 'wait' : 'bad');

  function openNewMenu(event: MouseEvent) {
    ui.showMenu(event, [
      { label: 'Direct message…', onselect: () => (ui.dmSearchOpen = true) },
      { label: 'Create room', onselect: () => createRoom().catch((error: unknown) => ui.error(`Couldn't create the room: ${String(error)}`)) },
      { label: 'Join room by address…', onselect: () => (ui.roomAction = { kind: 'join' }) },
    ]);
  }
</script>

<aside class="sidebar">
  <div class="profile">
    <Avatar src={session.profile?.avatar_url} name={name || '?'} seed={session.info?.user_id} size={34} />
    <div class="who grow">
      <div class="truncate display">{name}</div>
      <div class="truncate muted small" title={syncLabel}>
        <span class="dot {syncTone}"></span>{syncLabel}
      </div>
    </div>
    <button class="icon-btn" onclick={openNewMenu} aria-label="New conversation" title="New conversation"><Icon name="plus" /></button>
    <button class="icon-btn" onclick={() => ui.openSettings()} aria-label="Settings" title="Settings"><Icon name="settings" /></button>
  </div>

  <div class="filter">
    <Icon name="search" size={14} />
    <input class="input" placeholder="Filter rooms" bind:value={rooms.filter} aria-label="Filter rooms" spellcheck="false" />
  </div>

  <nav class="lists" aria-label="Rooms">
    {#if !rooms.listLoaded}
      <p class="muted pad"><span class="spinner"></span></p>
    {:else}
      {#if model.spaces.length > 0}
        <section>
          <h2>Spaces</h2>
          {#each model.spaces as node (node.space.room_id)}
            {#if model.filtered}
              <RoomItem room={node.space} />
            {:else}
              <SpaceTree {node} />
            {/if}
          {/each}
        </section>
      {/if}

      {#if model.directMessages.length > 0}
        <section>
          <h2>Direct messages</h2>
          {#each model.directMessages as room (room.room_id)}<RoomItem {room} />{/each}
        </section>
      {/if}

      {#if model.rooms.length > 0}
        <section>
          <h2>Rooms</h2>
          {#each model.rooms as room (room.room_id)}<RoomItem {room} />{/each}
        </section>
      {/if}

      {#if rooms.list.length === 0}
        <p class="muted pad">You're not in any rooms yet. Use <strong>+</strong> to start a conversation or join a room.</p>
      {:else if model.filtered && model.spaces.length + model.directMessages.length + model.rooms.length === 0}
        <p class="muted pad">Nothing matches “{rooms.filter}”.</p>
      {/if}
    {/if}
  </nav>
</aside>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    width: var(--sidebar-width);
    min-height: 0;
    border-right: 1px solid var(--border);
    background: var(--surface);
  }
  .profile {
    display: flex;
    align-items: center;
    gap: 8px;
    height: var(--header-height);
    padding: 0 8px 0 12px;
    border-bottom: 1px solid var(--border);
  }
  .who {
    line-height: 1.25;
  }
  .display {
    font-weight: 600;
  }
  .dot {
    display: inline-block;
    width: 7px;
    height: 7px;
    margin-right: 5px;
    border-radius: 50%;
    background: var(--muted);
  }
  .dot.ok {
    background: var(--success);
  }
  .dot.wait {
    background: #f5a623;
  }
  .dot.bad {
    background: var(--danger);
  }
  .filter {
    position: relative;
    display: flex;
    align-items: center;
    margin: 8px;
    color: var(--muted);
  }
  .filter :global(svg) {
    position: absolute;
    left: 10px;
    pointer-events: none;
  }
  .filter .input {
    padding-left: 30px;
  }
  .lists {
    flex: 1 1 auto;
    padding: 0 6px 12px;
    overflow-y: auto;
  }
  h2 {
    margin: 12px 6px 4px;
    color: var(--muted);
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .pad {
    padding: 12px 8px;
  }
</style>
