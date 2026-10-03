<script lang="ts">
  import { openDirectMessage } from '../actions';
  import { groupMembers, layoutRoster, safeColor, searchMembers, visibleRows } from '../members';
  import { memberMenu } from '../memberMenu';
  import { rooms } from '../stores/rooms.svelte';
  import { session } from '../stores/session.svelte';
  import { ui } from '../stores/ui.svelte';
  import { plural } from '../format';
  import Avatar from './Avatar.svelte';

  interface Props {
    roomId: string;
  }
  let { roomId }: Props = $props();

  let filter = $state('');
  const members = $derived(rooms.members[roomId] ?? []);
  const groups = $derived(groupMembers(searchMembers(members, filter), rooms.powerTags[roomId] ?? []));
  const roster = $derived(layoutRoster(groups));

  // Only the rows on screen exist (see `layoutRoster`).
  let scroller = $state<HTMLDivElement>();
  let scrollTop = $state(0);
  let viewport = $state(0);
  const visible = $derived(visibleRows(roster.rows, scrollTop, viewport));
  $effect(() => {
    const element = scroller;
    if (!element) return;
    const observer = new ResizeObserver(() => (viewport = element.clientHeight));
    observer.observe(element);
    return () => observer.disconnect();
  });
  // A new filter starts from the top of its (shorter) list.
  $effect(() => {
    void filter;
    if (scroller) scroller.scrollTop = 0;
    scrollTop = 0;
  });

  function message(userId: string) {
    if (userId === session.info?.user_id) return;
    openDirectMessage(userId).catch((error: unknown) => ui.error(`Couldn't open the conversation: ${String(error)}`));
  }
</script>

<aside class="members" aria-label="Members">
  <header>
    <span class="grow">{plural(members.length, 'member')}</span>
  </header>
  {#if members.length > 12}
    <input class="input filter" placeholder="Find a member" bind:value={filter} spellcheck="false" aria-label="Find a member" />
  {/if}
  <div class="scroll" bind:this={scroller} onscroll={() => (scrollTop = scroller?.scrollTop ?? 0)}>
    <div class="list" style:height="{roster.height}px">
      {#each visible as row (row.key)}
        {#if row.kind === 'header'}
          <h3 style:top="{row.top}px" style:color={safeColor(row.group.color)}>{row.group.name} — {row.group.members.length}</h3>
        {:else}
          {@const member = row.member}
          <button
            class="member"
            style:top="{row.top}px"
            onclick={() => message(member.user_id)}
            oncontextmenu={(event) => ui.showMenu(event, memberMenu(member.user_id, member.display_name || member.user_id))}
            title={member.user_id}
          >
            <Avatar src={member.avatar_url} name={member.display_name || member.user_id} seed={member.user_id} size={28} />
            <span class="truncate grow" style:color={safeColor(row.group.color)}>{member.display_name || member.user_id}</span>
            {#if rooms.ignored.includes(member.user_id)}<span class="muted small">ignored</span>{/if}
          </button>
        {/if}
      {/each}
    </div>
    {#if members.length === 0}<p class="muted pad">Loading members…</p>{/if}
  </div>
</aside>

<style>
  .members {
    display: flex;
    flex-direction: column;
    width: var(--members-width);
    min-height: 0;
    border-left: 1px solid var(--border);
    background: var(--surface);
  }
  header {
    display: flex;
    align-items: center;
    height: var(--header-height);
    padding: 0 14px;
    border-bottom: 1px solid var(--border);
    font-weight: 600;
  }
  .filter {
    width: auto;
    margin: 8px 8px 0;
  }
  .scroll {
    flex: 1 1 auto;
    padding: 0 6px 12px;
    overflow-y: auto;
  }
  .list {
    position: relative;
  }
  /* Fixed heights (members.ts HEADER_HEIGHT / MEMBER_HEIGHT): the list is windowed by arithmetic. */
  h3,
  .member {
    position: absolute;
    right: 0;
    left: 0;
  }
  h3 {
    height: 32px;
    margin: 0;
    padding: 14px 6px 4px;
    overflow: hidden;
    color: var(--muted);
    white-space: nowrap;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }
  .member {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 36px;
    padding: 4px 8px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    text-align: left;
  }
  .member:hover {
    background: var(--hover);
  }
  .pad {
    padding: 12px;
  }
</style>
