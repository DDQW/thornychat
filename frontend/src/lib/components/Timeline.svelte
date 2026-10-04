<script lang="ts">
  import { onDestroy, tick } from 'svelte';
  import { markRoomRead, paginateBackwards, shrinkTimeline } from '../actions';
  import { nameOf } from '../people';
  import { buildRows, reuseRows, type Row } from '../timeline/rows';
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
  // Unchanged rows keep their object, so only the rows a diff touched re-render.
  let previousRows: Row[] = [];
  const rows = $derived.by(() => {
    const next = timeline
      ? buildRows(visibleItems, {
          showMembership: !searching && (settings.value?.chat.show_membership_events ?? true),
          hideUnreadDivider: timeline.dividerSuppressed,
        })
      : [];
    previousRows = reuseRows(next, previousRows);
    return previousRows;
  });
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

  // Only the reader moving up lets go of the bottom. Deciding it from the
  // distance alone lost the bottom whenever content grew between a scroll to
  // the bottom and its scroll event (images and link cards loading in a row),
  // and then nothing scrolled down again.
  let lastScrollTop = 0;

  function onscroll() {
    if (!scroller) return;
    const top = scroller.scrollTop;
    distanceFromBottom = scroller.scrollHeight - top - scroller.clientHeight;
    if (distanceFromBottom < BOTTOM_SLACK) stuck = true;
    else if (top < lastScrollTop - 1) stuck = false;
    lastScrollTop = top;
    captureAnchor();
    maybePaginate();
  }

  // --- keeping the reader's place ---
  // The browser's own scroll anchoring is switched off for this scroller: it
  // does nothing at scrollTop 0, exactly where someone who just hit the top
  // is. Instead the message at the top of the view is remembered, and when
  // anything above it changes height (older history arriving, an image or
  // link card loading in), scrollTop is moved by the same amount. Anchoring
  // on a message rather than on whatever row comes first matters: on every
  // back-pagination the SDK replaces the date divider at the top with a new
  // one, so the first row is never the same row twice.
  let anchor: { eventId: string; offset: number } | null = null;

  function messageRows(): HTMLElement[] {
    return content ? [...content.querySelectorAll<HTMLElement>('[data-event-id]')] : [];
  }

  function captureAnchor() {
    anchor = null;
    if (!scroller || stuck) return;
    const viewTop = scroller.getBoundingClientRect().top;
    const elements = messageRows();
    // Rows are in document order, so the first one reaching into the view is
    // found by bisection.
    let low = 0;
    let high = elements.length;
    while (low < high) {
      const middle = (low + high) >> 1;
      if (elements[middle]!.getBoundingClientRect().bottom > viewTop) high = middle;
      else low = middle + 1;
    }
    const row = elements[low];
    if (row?.dataset.eventId) anchor = { eventId: row.dataset.eventId, offset: row.getBoundingClientRect().top - viewTop };
  }

  function keepAnchor() {
    if (!scroller || !content || !anchor) return;
    const row = content.querySelector<HTMLElement>(`[data-event-id="${CSS.escape(anchor.eventId)}"]`);
    if (!row) {
      captureAnchor();
      return;
    }
    const shift = row.getBoundingClientRect().top - scroller.getBoundingClientRect().top - anchor.offset;
    if (Math.abs(shift) < 1) return;
    scroller.scrollTop += shift;
    lastScrollTop = scroller.scrollTop;
  }

  // Whenever the content or the viewport changes size — a message arrives, an
  // image finishes loading, older history comes in, the composer grows a line —
  // stay glued to the bottom if that's where the reader is, and otherwise keep
  // what they're reading where it was.
  $effect(() => {
    if (!content || !scroller) return;
    const observer = new ResizeObserver(() => {
      if (stuck) scrollToBottom();
      else keepAnchor();
    });
    observer.observe(content);
    observer.observe(scroller);
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

  // --- older history ---
  /**
   * Runs `check` once the next frame has been laid out — after the resize
   * observer above has moved scrollTop for whatever just arrived. Checking
   * sooner reads the position from before the compensation (still at the top
   * after a page of history came in) and asks for one page too many.
   */
  function afterLayout(check: () => void) {
    requestAnimationFrame(() => setTimeout(check));
  }

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
        if (tl.items.length > before) afterLayout(maybePaginate);
      });
  }
  $effect(() => {
    void rows.length;
    afterLayout(maybePaginate);
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
    /* A zero basis, not auto: with auto, every layout of the room column (the
       composer growing a line, someone starting to type) re-measured all the
       messages to size this box — 7.7 ms with 200 of them, ~1 ms without. */
    flex: 1 1 0;
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
