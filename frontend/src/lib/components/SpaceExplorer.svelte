<script lang="ts">
  import { fetchSpaceHierarchy, joinRoom, knockRoom } from '../actions';
  import type { SpaceChildSummary } from '../bindings';
  import { plural } from '../format';
  import { rooms } from '../stores/rooms.svelte';
  import { ui } from '../stores/ui.svelte';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';
  import Modal from './Modal.svelte';

  interface Crumb {
    id: string;
    name: string;
  }

  const rootId = $derived(ui.spaceExplorer);
  let trail = $state<Crumb[]>([]);
  let children = $state<SpaceChildSummary[]>([]);
  let nextBatch = $state<string | null>(null);
  let loading = $state(false);
  let error = $state('');
  /** Per-room outcome of a join / knock the user started here. */
  let status = $state<Record<string, 'working' | 'joined' | 'requested' | string>>({});

  const current = $derived(trail.at(-1));

  // Opening the explorer (or changing space) starts a fresh trail.
  $effect(() => {
    if (!rootId) return;
    const root = rooms.byId.get(rootId);
    trail = [{ id: rootId, name: root?.name ?? 'Space' }];
  });

  // Load the first page whenever the space being looked at changes.
  let loadedFor = '';
  $effect(() => {
    const id = current?.id;
    if (!id || id === loadedFor) return;
    loadedFor = id;
    children = [];
    nextBatch = null;
    void loadPage(id, null);
  });

  async function loadPage(spaceId: string, from: string | null) {
    loading = true;
    error = '';
    try {
      const page = await fetchSpaceHierarchy(spaceId, from);
      // Ignore an answer for a space we've since navigated away from.
      if (current?.id !== spaceId) return;
      children = from ? [...children, ...page.children] : page.children;
      nextBatch = page.next_batch;
    } catch (e) {
      if (current?.id === spaceId) error = String(e);
    } finally {
      if (current?.id === spaceId) loading = false;
    }
  }

  function drillInto(child: SpaceChildSummary) {
    trail = [...trail, { id: child.room_id, name: child.name ?? child.canonical_alias ?? child.room_id }];
  }

  function back() {
    if (trail.length > 1) trail = trail.slice(0, -1);
  }

  function open(child: SpaceChildSummary) {
    rooms.select(child.room_id);
    ui.spaceExplorer = null;
  }

  async function act(child: SpaceChildSummary, knock: boolean) {
    status = { ...status, [child.room_id]: 'working' };
    try {
      if (knock) {
        await knockRoom(child.room_id, child.via);
        status = { ...status, [child.room_id]: 'requested' };
      } else {
        await joinRoom(child.room_id, child.via);
        status = { ...status, [child.room_id]: 'joined' };
      }
    } catch (e) {
      status = { ...status, [child.room_id]: String(e) };
    }
  }

  const label = (child: SpaceChildSummary) => child.name ?? child.canonical_alias ?? child.room_id;
  const isJoined = (child: SpaceChildSummary) => child.joined || status[child.room_id] === 'joined' || rooms.byId.has(child.room_id);
  const failure = (child: SpaceChildSummary) => {
    const value = status[child.room_id];
    return value && value !== 'working' && value !== 'joined' && value !== 'requested' ? value : null;
  };
</script>

{#if rootId}
  <Modal title="Explore {trail[0]?.name ?? 'space'}" onclose={() => (ui.spaceExplorer = null)} width="620px">
    <nav class="crumbs">
      <button class="icon-btn" onclick={back} disabled={trail.length < 2} aria-label="Back"><Icon name="chevron-left" /></button>
      {#each trail as crumb, index (crumb.id)}
        {#if index > 0}<Icon name="chevron-right" size={14} />{/if}
        <span class:muted={index < trail.length - 1} class="truncate">{crumb.name}</span>
      {/each}
    </nav>

    {#if error}<p class="error-text" role="alert">{error}</p>{/if}

    <ul>
      {#each children as child (child.room_id)}
        <li>
          <Avatar src={child.avatar_url} name={label(child)} size={38} square />
          <div class="grow">
            <div class="truncate"><strong>{label(child)}</strong>{#if child.is_space}<span class="tag">space</span>{/if}</div>
            {#if child.topic}<div class="muted small clamp">{child.topic}</div>{/if}
            <div class="muted small">{plural(child.num_joined_members, 'member')} · {child.join_rule === 'Public' ? 'public' : child.join_rule === 'Restricted' ? 'members of the space' : child.join_rule === 'Knock' ? 'ask to join' : 'invite only'}</div>
            {#if failure(child)}<div class="error-text small">{failure(child)}</div>{/if}
          </div>
          <div class="actions">
            {#if child.is_space}
              <button class="btn small" onclick={() => drillInto(child)}>Explore</button>
            {:else if isJoined(child)}
              <button class="btn small" onclick={() => open(child)}>Open</button>
            {:else if status[child.room_id] === 'working'}
              <span class="spinner"></span>
            {:else if status[child.room_id] === 'requested'}
              <span class="muted small">Requested</span>
            {:else if child.join_rule === 'Knock'}
              <button class="btn small" onclick={() => act(child, true)}>Request to join</button>
            {:else if child.join_rule === 'InviteOnly'}
              <span class="muted small">Invite only</span>
            {:else}
              <button class="btn primary small" onclick={() => act(child, false)}>Join</button>
            {/if}
          </div>
        </li>
      {/each}
    </ul>

    {#if loading}<p class="center"><span class="spinner"></span></p>{/if}
    {#if nextBatch && !loading}
      <p class="center"><button class="btn" onclick={() => current && loadPage(current.id, nextBatch)}>Load more</button></p>
    {/if}
    {#if !loading && !error && children.length === 0}<p class="muted">Nothing here yet.</p>{/if}
  </Modal>
{/if}

<style>
  .crumbs {
    display: flex;
    align-items: center;
    gap: 4px;
    margin-bottom: 8px;
    color: var(--muted);
  }
  .crumbs span:last-child {
    color: var(--text);
    font-weight: 600;
  }
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  li {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 2px;
    border-bottom: 1px solid var(--border);
  }
  li:last-child {
    border-bottom: 0;
  }
  .tag {
    margin-left: 8px;
    padding: 1px 6px;
    border-radius: 4px;
    background: var(--accent-wash);
    color: var(--accent);
    font-size: 11px;
  }
  .clamp {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    overflow: hidden;
  }
  .actions {
    flex: 0 0 auto;
  }
  .center {
    display: grid;
    place-items: center;
  }
</style>
