<script lang="ts">
  import { tick } from 'svelte';
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

  let filterInput = $state<HTMLInputElement>();
  let lists = $state<HTMLElement>();

  /** The sidebar's rows that take focus, top to bottom (collapsed spaces hide theirs). */
  const rows = () => (lists ? [...lists.querySelectorAll<HTMLButtonElement>('button.item, button.title')] : []);

  /** Enter opens the first match, Down moves into the list, Escape clears the filter. */
  function onfilterkeydown(event: KeyboardEvent) {
    if (event.key === 'Enter') {
      const first = lists?.querySelector<HTMLButtonElement>('button.item');
      if (!first) return;
      event.preventDefault();
      first.click();
      rooms.filter = '';
      // Picked to talk in: straight on to the message box.
      void tick().then(() => document.querySelector<HTMLTextAreaElement>('.composer textarea')?.focus());
    } else if (event.key === 'ArrowDown') {
      event.preventDefault();
      rows()[0]?.focus();
    } else if (event.key === 'Escape' && rooms.filter) {
      event.stopPropagation();
      rooms.filter = '';
    }
  }

  /** Up and Down walk the list; Up from the first row goes back to the filter. */
  function onlistkeydown(event: KeyboardEvent) {
    if (event.altKey || event.ctrlKey || event.metaKey) return;
    const list = rows();
    const index = list.indexOf(document.activeElement as HTMLButtonElement);
    if (index < 0) return;
    const next = event.key === 'ArrowDown' ? index + 1 : event.key === 'ArrowUp' ? index - 1 : event.key === 'Home' ? 0 : event.key === 'End' ? list.length - 1 : null;
    if (next === null) return;
    event.preventDefault();
    if (next < 0) filterInput?.focus();
    else list[Math.min(next, list.length - 1)]?.focus();
  }

  export function focusFilter() {
    filterInput?.focus();
    filterInput?.select();
  }

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
      <div class="truncate muted small" title={syncLabel} role="status">
        <span class="dot {syncTone}" aria-hidden="true"></span>{syncLabel}
      </div>
    </div>
    <button class="icon-btn" onclick={openNewMenu} aria-label="New conversation" aria-haspopup="menu" title="New conversation"><Icon name="plus" /></button>
    <button class="icon-btn" onclick={() => ui.openSettings()} aria-label="Settings" title="Settings (Ctrl+,)"><Icon name="settings" /></button>
  </div>

  <div class="filter">
    <Icon name="search" size={14} />
    <input
      class="input"
      type="search"
      placeholder="Filter rooms"
      bind:this={filterInput}
      bind:value={rooms.filter}
      aria-label="Filter rooms"
      aria-keyshortcuts="Control+K"
      title="Filter rooms (Ctrl+K)"
      spellcheck="false"
      autocomplete="off"
      onkeydown={onfilterkeydown}
    />
  </div>

  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <nav class="lists" aria-label="Rooms" bind:this={lists} onkeydown={onlistkeydown}>
    {#if !rooms.listLoaded}
      <p class="muted pad" role="status"><span class="spinner"></span><span class="sr-only">Loading rooms…</span></p>
    {:else}
      {#if model.spaces.length > 0}
        <section>
          <h2 class="eyebrow">Spaces</h2>
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
          <h2 class="eyebrow">Direct messages</h2>
          {#each model.directMessages as room (room.room_id)}<RoomItem {room} />{/each}
        </section>
      {/if}

      {#if model.rooms.length > 0}
        <section>
          <h2 class="eyebrow">Rooms</h2>
          {#each model.rooms as room (room.room_id)}<RoomItem {room} />{/each}
        </section>
      {/if}

      {#if rooms.list.length === 0}
        <p class="muted pad">You're not in any rooms yet. Use <strong>+</strong> to start a conversation or join a room.</p>
      {:else if model.filtered && model.spaces.length + model.directMessages.length + model.rooms.length === 0}
        <p class="muted pad" role="status">Nothing matches “{rooms.filter}”.</p>
      {/if}
    {/if}
  </nav>
</aside>

<style>
  .sidebar {
    display: flex;
    flex: none;
    flex-direction: column;
    width: var(--sidebar-width);
    min-height: 0;
    background: var(--surface);
  }
  .profile {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: var(--header-height);
    padding: 0 var(--space-2) 0 var(--space-3);
    border-bottom: 1px solid var(--border);
  }
  .who {
    line-height: 1.25;
  }
  .display {
    font-weight: 600;
  }
  .who .small {
    display: flex;
    align-items: center;
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
    margin: var(--space-3) var(--space-2) var(--space-1);
    color: var(--muted);
  }
  .filter :global(svg) {
    position: absolute;
    left: 10px;
    pointer-events: none;
  }
  .filter .input {
    padding-left: 30px;
    border-color: transparent;
    background: var(--surface-strong);
  }
  /* The search field's own clear button: Escape does the same. */
  .filter .input::-webkit-search-cancel-button {
    filter: grayscale(1);
    cursor: pointer;
  }
  .lists {
    flex: 1 1 auto;
    padding: 0 var(--space-2) var(--space-3);
    overflow-y: auto;
  }
  h2 {
    padding: var(--space-4) var(--space-2) 6px;
  }
  .pad {
    padding: 12px 8px;
  }
</style>
