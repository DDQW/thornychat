<script lang="ts">
  import { readPref, writePref } from '../prefs';
  import { unreadIn, type SpaceNode } from '../sidebar';
  import { ui } from '../stores/ui.svelte';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';
  import RoomItem from './RoomItem.svelte';
  import SpaceTree from './SpaceTree.svelte';

  interface Props {
    node: SpaceNode;
    depth?: number;
  }
  let { node, depth = 0 }: Props = $props();

  let collapsedIds = $state<string[]>(readPref('collapsedSpaces', []));
  const collapsed = $derived(collapsedIds.includes(node.space.room_id));
  const unread = $derived(unreadIn(node) + node.space.unread_count);

  function toggle() {
    collapsedIds = collapsed ? collapsedIds.filter((id) => id !== node.space.room_id) : [...collapsedIds, node.space.room_id];
    writePref('collapsedSpaces', collapsedIds);
  }
</script>

<div class="space">
  <div class="header" style:padding-left="{4 + depth * 14}px">
    <button class="icon-btn small-btn" onclick={toggle} aria-label={collapsed ? 'Expand space' : 'Collapse space'} aria-expanded={!collapsed}>
      <Icon name={collapsed ? 'chevron-right' : 'chevron-down'} size={14} />
    </button>
    <button class="title grow" onclick={toggle} oncontextmenu={(event) => ui.showMenu(event, [{ label: 'Explore space…', onselect: () => (ui.spaceExplorer = node.space.room_id) }])}>
      <Avatar src={node.space.avatar_url} name={node.space.name} size={20} square />
      <span class="truncate">{node.space.name}</span>
      {#if collapsed && unread > 0}<span class="badge">{unread > 99 ? '99+' : unread}</span>{/if}
    </button>
    <button class="icon-btn small-btn" onclick={() => (ui.spaceExplorer = node.space.room_id)} title="Explore this space" aria-label="Explore space">
      <Icon name="globe" size={14} />
    </button>
  </div>

  {#if !collapsed}
    {#each node.rooms as room (room.room_id)}
      <RoomItem {room} depth={depth + 1} />
    {/each}
    {#each node.spaces as child (child.space.room_id)}
      <SpaceTree node={child} depth={depth + 1} />
    {/each}
  {/if}
</div>

<style>
  .header {
    display: flex;
    align-items: center;
    gap: 2px;
    padding-right: 2px;
  }
  .small-btn {
    width: 24px;
    height: 24px;
  }
  .title {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    padding: 4px 4px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    font-weight: 600;
    text-align: left;
  }
  .title:hover {
    background: var(--hover);
  }
</style>
