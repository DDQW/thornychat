<script lang="ts">
  import { joinCall } from '../actions';
  import type { RoomSummary } from '../bindings';
  import { nameOf } from '../people';
  import { composer } from '../stores/composer.svelte';
  import { rooms } from '../stores/rooms.svelte';
  import { session } from '../stores/session.svelte';
  import { settings } from '../stores/settings.svelte';
  import { ui } from '../stores/ui.svelte';
  import Avatar from './Avatar.svelte';
  import CallBanner from './CallBanner.svelte';
  import Composer from './Composer.svelte';
  import Icon from './Icon.svelte';
  import MembersPanel from './MembersPanel.svelte';
  import Timeline from './Timeline.svelte';

  interface Props {
    room: RoomSummary;
  }
  let { room }: Props = $props();

  let timeline = $state<ReturnType<typeof Timeline>>();
  let composerRef = $state<ReturnType<typeof Composer>>();
  let dragging = $state(0);
  let searchOpen = $state(false);
  let query = $state('');
  let matches = $state(0);

  const showMembers = $derived(!(settings.value?.chat.hide_members ?? false));
  const hasCall = $derived((rooms.calls[room.room_id]?.participants.length ?? 0) > 0);

  // A reply, an edit or a search belongs to one room.
  $effect(() => {
    void room.room_id;
    composer.clear();
    searchOpen = false;
    query = '';
  });

  function closeSearch() {
    searchOpen = false;
    query = '';
  }

  const typingText = $derived.by(() => {
    const names = (rooms.typing[room.room_id] ?? []).filter((id) => id !== session.info?.user_id).map((id) => nameOf(id, room.room_id));
    if (names.length === 0) return '';
    if (names.length === 1) return `${names[0]} is typing…`;
    if (names.length === 2) return `${names[0]} and ${names[1]} are typing…`;
    return 'Several people are typing…';
  });

  function toggleMembers() {
    settings.update((draft) => (draft.chat.hide_members = !draft.chat.hide_members));
  }

  async function startCall() {
    try {
      await joinCall(room.room_id);
    } catch (error) {
      ui.error(`Couldn't start the call: ${String(error)}`);
    }
  }

  // --- dropping files anywhere on the room attaches them ---
  const hasFiles = (event: DragEvent) => event.dataTransfer?.types.includes('Files') ?? false;
  function ondragenter(event: DragEvent) {
    if (!hasFiles(event)) return;
    event.preventDefault();
    dragging++;
  }
  function ondragover(event: DragEvent) {
    if (hasFiles(event)) event.preventDefault();
  }
  function ondragleave(event: DragEvent) {
    if (hasFiles(event)) dragging = Math.max(0, dragging - 1);
  }
  function ondrop(event: DragEvent) {
    if (!hasFiles(event)) return;
    event.preventDefault();
    dragging = 0;
    composerRef?.addFiles(event.dataTransfer?.files ?? []);
  }
</script>

<section class="room" {ondragenter} {ondragover} {ondragleave} {ondrop} role="presentation">
  <div class="main">
    <header>
      {#if room.is_dm}
        <Avatar src={room.avatar_url} name={room.name} size={30} />
      {:else}
        <Avatar src={room.avatar_url} name={room.name} size={30} square />
      {/if}
      <div class="titles grow">
        <h1 class="truncate">
          {#if room.is_encrypted}<span class="lock" title="End-to-end encrypted"><Icon name="lock" size={14} /></span>{/if}{room.name}
        </h1>
        {#if room.topic}<div class="topic truncate muted small" title={room.topic}>{room.topic}</div>{/if}
      </div>
      {#if !hasCall && !room.is_space}
        <button class="icon-btn" onclick={startCall} title="Start a call (signaling only)" aria-label="Start a call"><Icon name="phone" /></button>
      {/if}
      <button class="icon-btn" class:active={searchOpen} onclick={() => (searchOpen ? closeSearch() : (searchOpen = true))} title="Search this conversation" aria-label="Search messages" aria-pressed={searchOpen}>
        <Icon name="search" />
      </button>
      <button class="icon-btn" class:active={showMembers} onclick={toggleMembers} title={showMembers ? 'Hide members' : 'Show members'} aria-label="Toggle member list" aria-pressed={showMembers}>
        <Icon name="users" />
      </button>
    </header>

    {#if searchOpen}
      <div class="search">
        <Icon name="search" size={15} />
        <!-- svelte-ignore a11y_autofocus -->
        <input
          class="input"
          placeholder="Search the messages loaded here…"
          bind:value={query}
          autofocus
          spellcheck="false"
          aria-label="Search messages"
          onkeydown={(event) => event.key === 'Escape' && (event.stopPropagation(), closeSearch())}
        />
        {#if query.trim()}<span class="muted small nowrap">{matches === 1 ? '1 match' : `${matches} matches`}</span>{/if}
        <button class="icon-btn" onclick={closeSearch} aria-label="Close search"><Icon name="x" size={14} /></button>
      </div>
    {/if}

    <CallBanner roomId={room.room_id} />

    {#key room.room_id}
      <Timeline roomId={room.room_id} bind:this={timeline} {query} onmatches={(count) => (matches = count)} />
    {/key}

    <div class="typing small muted" aria-live="polite">{typingText}</div>

    {#key room.room_id}
      <Composer roomId={room.room_id} bind:this={composerRef} />
    {/key}

    {#if dragging > 0}
      <div class="drop" aria-hidden="true"><Icon name="paperclip" size={28} /> Drop files to attach</div>
    {/if}
  </div>

  {#if showMembers}
    <MembersPanel roomId={room.room_id} />
  {/if}
</section>

<style>
  .room {
    display: flex;
    flex: 1 1 auto;
    min-width: 0;
    min-height: 0;
  }
  .main {
    position: relative;
    display: flex;
    flex: 1 1 auto;
    flex-direction: column;
    min-width: 0;
  }
  header {
    display: flex;
    align-items: center;
    gap: 10px;
    height: var(--header-height);
    padding: 0 12px 0 16px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }
  .titles {
    line-height: 1.25;
  }
  h1 {
    margin: 0;
    font-size: 15px;
  }
  .lock {
    margin-right: 5px;
    color: var(--muted);
  }
  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 16px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    color: var(--muted);
  }
  .nowrap {
    white-space: nowrap;
  }
  .typing {
    min-height: 20px;
    padding: 0 20px;
  }
  .drop {
    position: absolute;
    inset: 8px;
    z-index: 20;
    display: grid;
    gap: 8px;
    border: 2px dashed var(--accent);
    border-radius: var(--radius);
    background: rgb(0 0 0 / 0.6);
    color: var(--accent);
    font-size: 16px;
    font-weight: 600;
    place-content: center;
    place-items: center;
    pointer-events: none;
  }
</style>
