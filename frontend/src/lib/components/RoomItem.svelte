<script lang="ts">
  import { clearRoomNotificationMode, setRoomNotificationMode } from '../actions';
  import type { NotificationMode, RoomSummary } from '../bindings';
  import { rooms } from '../stores/rooms.svelte';
  import { ui, type MenuItem } from '../stores/ui.svelte';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';

  interface Props {
    room: RoomSummary;
    /** Nesting depth under spaces, for the left padding. */
    depth?: number;
  }
  let { room, depth = 0 }: Props = $props();

  const active = $derived(rooms.selectedId === room.room_id);
  const mode = $derived(rooms.mode(room.room_id, room.is_dm));
  const hasOverride = $derived(room.room_id in rooms.modes);
  const muted = $derived(mode === 'Mute');
  const inCall = $derived((rooms.calls[room.room_id]?.participants.length ?? 0) > 0);

  const MODE_LABEL: Record<NotificationMode, string> = {
    AllMessages: 'All messages',
    MentionsAndKeywordsOnly: 'Mentions & keywords only',
    Mute: 'Muted',
  };

  function setMode(next: NotificationMode) {
    setRoomNotificationMode(room.room_id, next).catch((error: unknown) => ui.error(`Couldn't change notifications: ${String(error)}`));
  }

  function openMenu(event: MouseEvent) {
    const check = (selected: boolean) => (selected ? '✓ ' : ' ');
    const items: MenuItem[] = [
      ...(['AllMessages', 'MentionsAndKeywordsOnly', 'Mute'] as const).map((value) => ({
        label: `${check(hasOverride && mode === value)}Notify: ${MODE_LABEL[value]}`,
        onselect: () => setMode(value),
      })),
      {
        label: `${check(!hasOverride)}Notify: account default`,
        onselect: () => clearRoomNotificationMode(room.room_id).catch((error: unknown) => ui.error(String(error))),
      },
      { label: 'Rename room…', separator: true, onselect: () => (ui.roomAction = { kind: 'rename', roomId: room.room_id }) },
      { label: 'Leave room…', onselect: () => (ui.roomAction = { kind: 'leave', roomId: room.room_id }) },
      { label: 'Leave and forget…', danger: true, onselect: () => (ui.roomAction = { kind: 'forget', roomId: room.room_id }) },
    ];
    ui.showMenu(event, items);
  }
</script>

<button
  class="item"
  class:active
  class:muted
  style:padding-left="{10 + depth * 14}px"
  onclick={() => (room.is_space ? (ui.spaceExplorer = room.room_id) : rooms.select(room.room_id))}
  oncontextmenu={openMenu}
  aria-current={active ? 'page' : undefined}
>
  {#if room.is_dm}
    <Avatar src={room.avatar_url} name={room.name} size={26} />
  {:else}
    <Avatar src={room.avatar_url} name={room.name} size={26} square />
  {/if}
  <span class="name grow truncate" class:unread={room.unread_count > 0 && !muted}>{room.name}</span>
  {#if room.is_encrypted}<span class="meta" title="End-to-end encrypted"><Icon name="lock" size={13} /></span>{/if}
  {#if muted}<span class="meta" title="Muted"><Icon name="bell-off" size={13} /></span>{/if}
  {#if inCall}<span class="meta call" title="Call in progress"><Icon name="phone" size={13} /></span>{/if}
  {#if room.unread_count > 0}
    <span class="badge" class:muted-badge={muted}>{room.unread_count > 99 ? '99+' : room.unread_count}</span>
  {/if}
</button>

<style>
  .item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 5px 10px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    text-align: left;
  }
  .item:hover {
    background: var(--hover);
  }
  .item.active {
    background: var(--surface-strong);
  }
  .item.muted .name {
    color: var(--muted);
  }
  .name {
    /* Overrides the per-user colour utility: room names are plain text. */
    color: inherit;
    font-weight: 400;
  }
  .name.unread {
    font-weight: 700;
  }
  .meta {
    display: inline-flex;
    color: var(--muted);
  }
  .meta.call {
    color: var(--success);
  }
</style>
