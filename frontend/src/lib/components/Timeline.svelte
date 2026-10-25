<script lang="ts">
  import { onDestroy, tick, untrack } from 'svelte';
  import { markRoomRead, paginateBackwards, shrinkTimeline } from '../actions';
  import { nameOf } from '../people';
  import { buildRows, reuseRows, type Row } from '../timeline/rows';
  import type { ReplyPreview } from '../bindings';
  import { matchesQuery } from '../timeline/search';
  import { rooms } from '../stores/rooms.svelte';
  import { session } from '../stores/session.svelte';
  import { settings } from '../stores/settings.svelte';
  import { MAX_LIVE_ITEMS, timelines } from '../stores/timelines.svelte';
  import { ui } from '../stores/ui.svelte';
  import Icon from './Icon.svelte';
  import MessageRow from './MessageRow.svelte';
  import { formatDay, formatTime } from '../format';

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
    // A quote jump loading history holds the view where it is, which can be
    // the bottom; that mustn't re-stick it and snap back down after the jump.
    if (distanceFromBottom < BOTTOM_SLACK) stuck ||= !jumping;
    else if (top < lastScrollTop - 1) stuck = false;
    lastScrollTop = top;
    lastScrolled = performance.now();
    captureAnchor();
    schedulePaginate();
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
  // Several rows from the top of the view are remembered, not one: when a
  // back-pagination overlaps history already loaded, the SDK removes those
  // events and inserts them again, and the one row everything hangs on can be
  // gone for a frame. The next one down still pins the place.
  const ANCHORS = 6;
  let anchor: { eventId: string; offset: number }[] = [];

  function messageRows(): HTMLElement[] {
    return content ? [...content.querySelectorAll<HTMLElement>('[data-event-id]')] : [];
  }

  function captureAnchor() {
    anchor = [];
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
    for (const row of elements.slice(low, low + ANCHORS)) {
      if (row.dataset.eventId) anchor.push({ eventId: row.dataset.eventId, offset: row.getBoundingClientRect().top - viewTop });
    }
  }

  function keepAnchor() {
    if (!scroller || !content || anchor.length === 0) return;
    const viewTop = scroller.getBoundingClientRect().top;
    for (const candidate of anchor) {
      const row = content.querySelector<HTMLElement>(`[data-event-id="${CSS.escape(candidate.eventId)}"]`);
      if (!row) continue;
      const shift = row.getBoundingClientRect().top - viewTop - candidate.offset;
      if (Math.abs(shift) < 1) return;
      scroller.scrollTop += shift;
      lastScrollTop = scroller.scrollTop;
      return;
    }
    // Every remembered row is gone (the room was reset): start over from here.
    captureAnchor();
  }

  // Whenever the content or the viewport changes size — a message arrives, an
  // image finishes loading, older history comes in, the composer grows a line —
  // stay glued to the bottom if that's where the reader is, and otherwise keep
  // what they're reading where it was.
  $effect(() => {
    const list = content;
    if (!list || !scroller) return;
    const observer = new ResizeObserver(() => {
      if (stuck) scrollToBottom();
      else keepAnchor();
    });
    observer.observe(list);
    observer.observe(scroller);
    // Every row as well: two changes that cancel out (a read receipt leaving
    // one message for another) leave the list's height as it was, yet still
    // move everything between them.
    for (const row of list.children) observer.observe(row);
    const rowsChanged = new MutationObserver((records) => {
      for (const record of records) {
        for (const node of record.addedNodes) if (node instanceof Element && node.parentNode === list) observer.observe(node);
        for (const node of record.removedNodes) if (node instanceof Element && node.parentNode !== list) observer.unobserve(node);
      }
    });
    rowsChanged.observe(list, { childList: true });
    return () => {
      rowsChanged.disconnect();
      observer.disconnect();
    };
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

  /** The list's height when the last page was asked for; see `maybePaginate`. */
  let heightAtRequest = Infinity;
  /** When the reader last used the wheel, touch, keys or the scrollbar here. */
  let lastInput = 0;
  /** Any scroll, the reader's or ours; only the shrink's idle check looks at it. */
  let lastScrolled = 0;
  let paginateTimer: ReturnType<typeof setTimeout> | undefined;

  function noteInput() {
    lastInput = performance.now();
  }
  $effect(() => {
    const target = scroller;
    if (!target) return;
    const events = ['wheel', 'touchmove', 'keydown', 'pointerdown'] as const;
    for (const name of events) target.addEventListener(name, noteInput, { passive: true });
    // Dragging the scrollbar is a pointer held down.
    const drag = (event: PointerEvent) => event.buttons && noteInput();
    target.addEventListener('pointermove', drag, { passive: true });
    return () => {
      for (const name of events) target.removeEventListener(name, noteInput);
      target.removeEventListener('pointermove', drag);
    };
  });

  /**
   * Checks a moment from now rather than immediately. A page can land as
   * several batches (events removed, then inserted again further down), and
   * for a frame or two the view sits at the top of a list that is about to be
   * long again; reacting to that fetched a second page on top of the first.
   */
  function schedulePaginate() {
    if (paginateTimer) return;
    paginateTimer = setTimeout(() => {
      paginateTimer = undefined;
      afterLayout(maybePaginate);
    }, 120);
  }

  function maybePaginate() {
    const tl = timeline;
    if (!tl || !scroller || !tl.loaded || tl.loadingOlder || tl.startReached || rows.length === 0 || jumping) return;
    if (scroller.scrollTop > PAGINATE_AT) return;
    // Following the live edge with a full screen of messages: nothing above
    // matters yet. (A short list that doesn't fill the screen does need more.)
    const fillsViewport = scroller.scrollHeight > scroller.clientHeight + 100;
    if (stuck && fillsViewport) return;
    // A page that grew the list by more than the trigger distance, with the
    // view still inside it, means the reader was not held in place. Asking for
    // the next page by itself would pull history in page after page (the SDK
    // hands over up to 128 events at a time); only the reader asks again.
    if (fillsViewport && scroller.scrollHeight - heightAtRequest >= PAGINATE_AT && performance.now() - lastInput > 400) return;
    tl.loadingOlder = true;
    const before = tl.items.length;
    heightAtRequest = scroller.scrollHeight;
    captureAnchor();
    paginateBackwards(roomId)
      .catch((error: unknown) => ui.error(`Couldn't load older messages: ${String(error)}`))
      .finally(() => {
        tl.loadingOlder = false;
        // A short list may still not fill the viewport; ask again, but only if
        // that page actually produced something (otherwise we'd spin).
        if (tl.items.length > before) schedulePaginate();
      });
  }
  $effect(() => {
    void rows.length;
    schedulePaginate();
  });
  onDestroy(() => clearTimeout(paginateTimer));

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
  // collapses to the newest page. The swap replaces the whole list a moment
  // after it is asked for, and anything that has the reader moving at that
  // moment gets their view yanked. So it only happens when nobody can be
  // reading: the window is in the background, or has been left alone a while.
  const SHRINK_AFTER_IDLE_MS = 90_000;
  const SHRINK_RECHECK_MS = 5_000;
  let shrinkTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const tl = timeline;
    clearTimeout(shrinkTimer);
    if (!tl || !stuck || tl.shrinkPending || tl.loadingOlder || ui.lightbox) return;
    if (tl.items.length <= MAX_LIVE_ITEMS) return;
    const attempt = () => {
      if (!stuck || tl.shrinkPending || tl.loadingOlder || ui.lightbox) return;
      const away = document.hidden || !document.hasFocus();
      if (!away && performance.now() - Math.max(lastInput, lastScrolled) < SHRINK_AFTER_IDLE_MS) {
        shrinkTimer = setTimeout(attempt, SHRINK_RECHECK_MS);
        return;
      }
      tl.shrinkPending = true;
      shrinkTimeline(roomId);
    };
    shrinkTimer = setTimeout(attempt, SHRINK_RECHECK_MS);
    return () => clearTimeout(shrinkTimer);
  });
  onDestroy(() => clearTimeout(shrinkTimer));

  // The list was replaced while someone was reading higher up (the worker's
  // swap landed after they had already scrolled away from the bottom): the
  // messages they were looking at are gone from it. Load back to them and put
  // the view where it was.
  let seenResets = 0;
  $effect(() => {
    const count = timeline?.resets ?? 0;
    if (count === seenResets) return;
    seenResets = count;
    untrack(() => {
      if (stuck || jumping || anchor.length === 0) return;
      const present = anchor.some((place) => content?.querySelector(`[data-event-id="${CSS.escape(place.eventId)}"]`));
      if (!present) void jumpTo(anchor[0]!.eventId, { restoreOffset: anchor[0]!.offset });
    });
  });

  // --- jump to a quoted message ---
  /** Pages of history (80 events each) a quote jump loads looking for its message. */
  const MAX_JUMP_PAGES = 10;
  /** A quote jump is under way (loading history for it, or scrolling to it). */
  let jumping = false;

  async function until(condition: () => boolean, timeoutMs: number): Promise<boolean> {
    const deadline = performance.now() + timeoutMs;
    while (!condition() && performance.now() < deadline) await new Promise((resolve) => setTimeout(resolve, 50));
    return condition();
  }

  /** Clock differences between servers and out-of-order arrival: how far past its time a search goes. */
  const JUMP_SLACK_MS = 10 * 60_000;

  /** Pages a search for a quote the server called unavailable may load: that answer has been wrong for messages a page away. */
  const GONE_JUMP_PAGES = 3;

  /** A click on a quote. */
  function jumpToQuote(reply: ReplyPreview) {
    void jumpTo(reply.event_id, { sentAt: reply.timestamp_ms, maxPages: reply.gone ? GONE_JUMP_PAGES : MAX_JUMP_PAGES });
  }

  /**
   * Scrolls to the quoted message and flashes it. One that is older than what
   * is loaded is fetched first, a page of history at a time: until it turns
   * up, or the history loaded reaches back past when it was sent (then it is
   * not among the events the timeline shows, and going further finds nothing).
   * `restoreOffset` puts the message at that distance from the top of the view
   * instead, quietly: it is how a reader gets their place back.
   */
  export async function jumpTo(
    eventId: string,
    { restoreOffset, sentAt, maxPages = MAX_JUMP_PAGES }: { restoreOffset?: number; sentAt?: number | null; maxPages?: number } = {},
  ) {
    const tl = timeline;
    if (!tl || jumping) return;
    const loaded = () => tl.items.some((item) => item.event_id === eventId);
    const pastItsTime = () => {
      if (!sentAt) return false;
      const oldest = tl.items.find((item) => item.event_id !== null)?.timestamp_ms;
      return oldest !== undefined && oldest < sentAt - JUMP_SLACK_MS;
    };
    let tooFar = false;

    // Stop following the bottom for the whole jump. Loading history for it
    // holds the view in place, often at the bottom, and a resize while still
    // following (the last page landing, an image loading in) would snap back
    // down there right after the jump.
    jumping = true;
    stuck = false;
    // What the jump loads counts as a page like any other: once it is done,
    // only the reader scrolling asks for more.
    if (scroller) heightAtRequest = scroller.scrollHeight;
    captureAnchor();
    try {
      if (!loaded() && !tl.startReached && !pastItsTime()) {
        if (restoreOffset === undefined) ui.toast('Loading older messages to find it…', 'info', 2500);
        try {
          let page = 0;
          for (; page < maxPages && !loaded() && !tl.startReached && !pastItsTime(); page++) {
            await until(() => !tl.loadingOlder, 10_000);
            const before = tl.items.length;
            tl.loadingOlder = true;
            try {
              await paginateBackwards(roomId);
            } finally {
              tl.loadingOlder = false;
            }
            // The page's diffs can land just after the command reports success.
            await until(() => tl.items.length !== before || loaded(), 1500);
          }
          tooFar = page === maxPages && !loaded() && !tl.startReached && !pastItsTime();
        } catch (error) {
          ui.error(`Couldn't load older messages: ${String(error)}`);
        }
      }

      await tick();
      const target = content?.querySelector<HTMLElement>(`[data-event-id="${CSS.escape(eventId)}"]`);
      if (!target) {
        const why = loaded()
          ? 'Clear the search to jump to that message.'
          : tooFar
            ? maxPages === GONE_JUMP_PAGES
              ? 'That message was deleted, or the server no longer has it.'
              : 'That message is too far back to jump to.'
            : "That message isn't among the ones this room shows.";
        if (restoreOffset === undefined) ui.toast(why, 'info', 3500);
        if (scroller && scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < BOTTOM_SLACK) stuck = true;
        return;
      }
      stuck = false;
      if (restoreOffset !== undefined && scroller) scroller.scrollTop += target.getBoundingClientRect().top - scroller.getBoundingClientRect().top - restoreOffset;
      else target.scrollIntoView({ block: 'center', behavior: 'instant' });
      if (scroller) {
        lastScrollTop = scroller.scrollTop;
        distanceFromBottom = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight;
      }
      captureAnchor();
      if (restoreOffset !== undefined) return;
      ui.highlightEvent = eventId;
      setTimeout(() => ui.highlightEvent === eventId && (ui.highlightEvent = null), 1800);
    } finally {
      // Held until the jump's own scroll event and resize callbacks are through.
      afterLayout(() => (jumping = false));
    }
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
            <div class="divider"><span>{formatDay(row.timestampMs)}</span></div>
          {:else if row.kind === 'unread'}
            <div class="divider unread"><span>New messages</span></div>
          {:else if row.kind === 'system'}
            <div class="system muted small">{row.text}</div>
          {:else}
            <MessageRow item={row.item} {roomId} continuation={row.continuation} onjump={jumpToQuote} />
          {/if}
        {/each}

        {#if rows.length === 0}
          <p class="muted state">{searching ? 'No loaded messages match. Scroll up to load more history, then search again.' : 'No messages yet. Say hello!'}</p>
        {/if}
      {/if}
    </div>
  </div>

  {#if !stuck && distanceFromBottom > 300}
    <button class="jump btn" onclick={() => { noteInput(); scrollToBottom(true); stuck = true; }}>
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
