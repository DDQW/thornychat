<script lang="ts">
  import { onDestroy, tick } from 'svelte';
  import { markRoomRead, paginateBackwards, shrinkTimeline } from '../actions';
  import { nameOf } from '../people';
  import { buildRows } from '../timeline/rows';
  import { matchesQuery } from '../timeline/search';
  import { rooms } from '../stores/rooms.svelte';
  import { session } from '../stores/session.svelte';
  import { settings } from '../stores/settings.svelte';
  import { MAX_LIVE_ITEMS, timelines } from '../stores/timelines.svelte';
  import { ui } from '../stores/ui.svelte';
  import Icon from './Icon.svelte';
  import MessageRow from './MessageRow.svelte';
  import { formatTime } from '../format';

  interface Props {
    roomId: string;
    /** Show only messages matching this text (the room search); empty shows everything. */
    query?: string;
    /** Reports how many loaded messages match, for the search bar. */
    onmatches?: (count: number) => void;
  }
  let { roomId, query = '', onmatches }: Props = $props();

  const timeline = $derived(timelines.get(roomId));
  const room = $derived(rooms.byId.get(roomId));
  const searching = $derived(query.trim().length > 0);
  // With a search active only matching messages are listed: no dividers, no
  // system lines. Names are resolved here so a search for "alice" finds her.
  const visibleItems = $derived.by(() => {
    const items = timeline?.items ?? [];
    return searching ? items.filter((item) => matchesQuery(item, query, nameOf(item.sender, roomId, item.sender_display_name))) : items;
  });
  const rows = $derived(
    timeline
      ? buildRows(visibleItems, {
          showMembership: !searching && (settings.value?.chat.show_membership_events ?? true),
          hideUnreadDivider: timeline.dividerSuppressed,
        })
      : [],
  );
  $effect(() => {
    onmatches?.(searching ? visibleItems.length : 0);
  });

  let scroller = $state<HTMLDivElement>();
  let content = $state<HTMLDivElement>();
  /** Following the newest message: new content scrolls into view. */
  let stuck = $state(true);
  let distanceFromBottom = $state(0);

  /** How close to the bottom still counts as "at the bottom", in pixels. */
  const BOTTOM_SLACK = 48;
  /** Start loading older history this far from the top. */
  const PAGINATE_AT = 600;

  function scrollToBottom(smooth = false) {
    scroller?.scrollTo({ top: scroller.scrollHeight, behavior: smooth ? 'smooth' : 'instant' });
  }

  function onscroll() {
    if (!scroller) return;
    distanceFromBottom = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight;
    stuck = distanceFromBottom < BOTTOM_SLACK;
    maybePaginate();
  }

  // Whenever the content gets taller or shorter — a message arrives, an image
  // finishes loading, a link card appears — stay glued to the bottom if that's
  // where the reader is.
  $effect(() => {
    if (!content) return;
    const observer = new ResizeObserver(() => {
      if (stuck) scrollToBottom();
    });
    observer.observe(content);
    return () => observer.disconnect();
  });

  // Sending a message brings you back to the bottom, even from deep in the
  // history: you just said something, and you expect to see it appear.
  let lastEcho = '';
  $effect(() => {
    const last = timeline?.items.at(-1);
    if (!last || last.event_id !== null || last.sender !== session.info?.user_id) return;
    if (last.content.type === 'DateDivider' || last.content.type === 'NewMessagesDivider') return;
    const key = `${last.timestamp_ms}:${last.content.type}`;
    if (key === lastEcho) return;
    lastEcho = key;
    stuck = true;
    void tick().then(() => scrollToBottom());
  });

  // --- keeping the reader's place when history is prepended ---
  // The browser's own scroll anchoring is switched off for this scroller: it
  // does nothing at scrollTop 0 (exactly where someone who just hit the top
  // is), and it can move scrollTop on its own, which this code would mistake
  // for the reader scrolling. Instead, when rows appear *above* the ones that
  // were first, the added height is added to scrollTop.
  let previousFirstKey: string | null = null;
  $effect.pre(() => {
    const first = rows[0]?.key ?? null;
    const element = scroller;
    const prepended = previousFirstKey !== null && first !== previousFirstKey && rows.some((row, index) => index > 0 && row.key === previousFirstKey);
    previousFirstKey = first;
    if (!element || !prepended || stuck) return;
    // This runs before the DOM catches up, so these are the old measurements.
    const before = { height: element.scrollHeight, top: element.scrollTop };
    void tick().then(() => {
      if (scroller) scroller.scrollTop = before.top + (scroller.scrollHeight - before.height);
    });
  });

  // --- older history ---
  function maybePaginate() {
    const tl = timeline;
    if (!tl || !scroller || !tl.loaded || tl.loadingOlder || tl.startReached || rows.length === 0) return;
    if (scroller.scrollTop > PAGINATE_AT) return;
    // Following the live edge with a full screen of messages: nothing above
    // matters yet. (A short list that doesn't fill the screen does need more.)
    const fillsViewport = scroller.scrollHeight > scroller.clientHeight + 100;
    if (stuck && fillsViewport) return;
    tl.loadingOlder = true;
    const before = tl.items.length;
    paginateBackwards(roomId)
      .catch((error: unknown) => ui.error(`Couldn't load older messages: ${String(error)}`))
      .finally(() => {
        tl.loadingOlder = false;
        // A short list may still not fill the viewport; ask again, but only if
        // that page actually produced something (otherwise we'd spin).
        if (tl.items.length > before) void tick().then(maybePaginate);
      });
  }
  $effect(() => {
    void rows.length;
    void tick().then(maybePaginate);
  });

  // --- read marker ---
  // Marked read only while scrolled to the newest message, whether or not the
  // window has focus.
  let lastMarked = '';
  $effect(() => {
    const tl = timeline;
    if (!tl?.loaded || !stuck) return;
    const newest = tl.items.findLast((item) => item.event_id !== null)?.event_id;
    if (!newest || newest === lastMarked) return;
    lastMarked = newest;
    markRoomRead(roomId);
    // Keep the "new messages" divider on screen a moment so you can see where
    // the unread run began, then hide it (the server's echo never arrives).
    dividerTimer = setTimeout(() => (tl.dividerSuppressed = true), 5000);
  });
  let dividerTimer: ReturnType<typeof setTimeout> | undefined;
  onDestroy(() => clearTimeout(dividerTimer));

  // --- live growth cap ---
  // Past the cap, a timeline quietly following the live edge is reopened so it
  // collapses to the newest page. At the bottom the swap is invisible (the tail
  // is identical before and after); under a scrolled-up reader it would yank
  // the view, so it waits.
  $effect(() => {
    const tl = timeline;
    if (!tl || !stuck || tl.shrinkPending || tl.loadingOlder || ui.lightbox) return;
    if (tl.items.length > MAX_LIVE_ITEMS) {
      tl.shrinkPending = true;
      shrinkTimeline(roomId);
    }
  });

  // --- jump to a quoted message ---
  export function jumpTo(eventId: string) {
    const target = content?.querySelector(`[data-event-id="${CSS.escape(eventId)}"]`);
    if (!target) {
      ui.toast("That message isn't loaded — scroll up to load older messages.", 'info', 3500);
      return;
    }
    target.scrollIntoView({ block: 'center', behavior: 'smooth' });
    ui.highlightEvent = eventId;
    setTimeout(() => ui.highlightEvent === eventId && (ui.highlightEvent = null), 1800);
  }

  const startedAt = $derived(timeline?.items.find((item) => item.timestamp_ms > 0)?.timestamp_ms);
</script>

<div class="timeline">
  <div class="scroller" bind:this={scroller} {onscroll} role="log" aria-live="polite" aria-label="Messages in {room?.name ?? 'this room'}">
    <div class="content" bind:this={content}>
      {#if !timeline?.loaded}
        <div class="state"><span class="spinner"></span></div>
      {:else}
        <div class="top">
          {#if timeline.loadingOlder}
            <span class="spinner"></span>
          {:else if timeline.startReached}
            <div class="start">
              <h2>{room?.name ?? 'This room'}</h2>
              <p class="muted">This is the start of the conversation{startedAt ? ` (${formatTime(startedAt)})` : ''}.</p>
            </div>
          {/if}
        </div>

        {#each rows as row (row.key)}
          {#if row.kind === 'date'}
            <div class="divider"><span>{row.label}</span></div>
          {:else if row.kind === 'unread'}
            <div class="divider unread"><span>New messages</span></div>
          {:else if row.kind === 'system'}
            <div class="system muted small">{row.text}</div>
          {:else}
            <MessageRow item={row.item} {roomId} continuation={row.continuation} onjump={jumpTo} />
          {/if}
        {/each}

        {#if rows.length === 0}
          <p class="muted state">{searching ? 'No loaded messages match. Scroll up to load more history, then search again.' : 'No messages yet. Say hello!'}</p>
        {/if}
      {/if}
    </div>
  </div>

  {#if !stuck && distanceFromBottom > 300}
    <button class="jump btn" onclick={() => { scrollToBottom(true); stuck = true; }}>
      <Icon name="arrow-down" size={15} /> Jump to latest
    </button>
  {/if}
</div>

<style>
  .timeline {
    position: relative;
    flex: 1 1 auto;
    min-height: 0;
  }
  .scroller {
    height: 100%;
    overflow-y: auto;
    overflow-anchor: none;
  }
  /* Few messages sit at the bottom, like every chat app. */
  .content {
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    min-height: 100%;
    padding-bottom: 8px;
  }
  .state {
    display: grid;
    padding: 32px;
    place-items: center;
  }
  .top {
    display: grid;
    min-height: 8px;
    padding: 12px;
    place-items: center;
  }
  .start h2 {
    margin: 0 0 4px;
  }
  .start p {
    margin: 0;
  }
  .divider {
    display: flex;
    align-items: center;
    gap: 12px;
    margin: 14px 16px 6px;
    color: var(--muted);
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .divider::before,
  .divider::after {
    flex: 1 1 auto;
    height: 1px;
    background: var(--border);
    content: '';
  }
  .divider.unread {
    color: var(--danger);
  }
  .divider.unread::before,
  .divider.unread::after {
    background: var(--danger);
    opacity: 0.5;
  }
  .system {
    padding: 2px 16px 2px 64px;
  }
  .jump {
    position: absolute;
    right: 20px;
    bottom: 14px;
    border-radius: 18px;
    box-shadow: 0 6px 20px rgb(0 0 0 / 0.4);
  }
</style>
