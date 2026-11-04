<script module lang="ts">
  import type { CustomEmoji } from '../bindings';
  import type { CollectedSticker } from '../stores/emoji.svelte';

  export type Pick =
    | { kind: 'unicode'; char: string }
    | { kind: 'custom'; emoji: CustomEmoji }
    | { kind: 'sticker'; sticker: CollectedSticker };
</script>

<script lang="ts">
  import { onMount } from 'svelte';
  import { backend } from '../api';
  import { refreshEmojiPacks } from '../actions';
  import { popover } from '../focus';
  import { loadEmojiData, searchEmoji, type EmojiData } from '../emoji';
  import { emoji as emojiStore } from '../stores/emoji.svelte';
  import { rooms } from '../stores/rooms.svelte';
  import Icon from './Icon.svelte';
  import Twemoji from './Twemoji.svelte';

  interface Props {
    /** The button the picker opened from; the popover sits next to it. */
    anchor: DOMRect;
    onpick: (pick: Pick) => void;
    onclose: () => void;
    /** Offer the sticker tab (the composer does; reactions don't). */
    stickers?: boolean;
  }
  let { anchor, onpick, onclose, stickers = false }: Props = $props();

  const WIDTH = 352;
  const HEIGHT = 400;

  let tab = $state<'emoji' | 'stickers'>('emoji');
  let query = $state('');
  let data = $state<EmojiData | null>(null);
  let failed = $state(false);

  // Open above or below the anchor, whichever has room, and stay on screen.
  const placement = $derived.by(() => {
    const above = anchor.top > HEIGHT + 12 || anchor.top > window.innerHeight - anchor.bottom;
    const top = above ? Math.max(8, anchor.top - HEIGHT - 6) : Math.min(window.innerHeight - HEIGHT - 8, anchor.bottom + 6);
    const left = Math.max(8, Math.min(anchor.right - WIDTH, window.innerWidth - WIDTH - 8));
    return { top, left };
  });

  onMount(() => {
    loadEmojiData().then(
      (loaded) => (data = loaded),
      () => (failed = true),
    );
    // Custom emoji are the one piece of room state the worker is never pushed:
    // re-fetch the packs now, so one edited server-side shows up.
    if (rooms.selectedId) refreshEmojiPacks(rooms.selectedId);
  });

  const customEmoji = $derived(emojiStore.packs.map((pack) => ({ name: pack.name, emojis: pack.emojis.filter((e) => e.is_emoticon) })).filter((pack) => pack.emojis.length > 0));
  const stickerPacks = $derived(emojiStore.packs.map((pack) => ({ name: pack.name, emojis: pack.emojis.filter((e) => e.is_sticker) })).filter((pack) => pack.emojis.length > 0));

  const results = $derived(data && query.trim() ? searchEmoji(data.all, query) : []);
  const customResults = $derived(
    query.trim() ? emojiStore.packs.flatMap((pack) => pack.emojis).filter((e) => e.is_emoticon && e.shortcode.toLowerCase().includes(query.trim().toLowerCase())) : [],
  );

  /** Frequently used: unicode glyphs and custom emoji (keyed by their mxc URL). */
  const frequent = $derived(
    emojiStore.frequent
      .map((key): Pick | null => {
        if (key.startsWith('mxc://')) {
          const custom = emojiStore.byMxc.get(key);
          return custom ? { kind: 'custom', emoji: custom } : null;
        }
        return { kind: 'unicode', char: key };
      })
      .filter((pick): pick is Pick => pick !== null),
  );

  function pickUnicode(char: string) {
    emojiStore.recordUse(char);
    onpick({ kind: 'unicode', char });
  }

  function pickCustom(custom: CustomEmoji) {
    emojiStore.recordUse(custom.mxc_url);
    onpick({ kind: 'custom', emoji: custom });
  }

  function pickSticker(sticker: CollectedSticker) {
    onpick({ kind: 'sticker', sticker });
  }

  // --- keyboard ---
  // The cells aren't tab stops (there are nearly two thousand): Down from the
  // search field enters the grid, the arrows move through it a row or a cell
  // at a time, Enter picks, Up from the top row goes back to the search field.
  let scroll = $state<HTMLDivElement>();
  let search = $state<HTMLInputElement>();

  function columnsOf(grid: Element): number {
    const cells = [...grid.children] as HTMLElement[];
    const top = cells[0]?.offsetTop;
    let count = 0;
    while (count < cells.length && cells[count]!.offsetTop === top) count++;
    return Math.max(1, count);
  }

  function ongridkeydown(event: KeyboardEvent) {
    const cell = (event.target as Element).closest?.<HTMLButtonElement>('.cell');
    if (!cell || !scroll) return;
    const grid = cell.parentElement!;
    const cells = [...grid.children] as HTMLButtonElement[];
    const index = cells.indexOf(cell);
    const columns = columnsOf(grid);
    const grids = [...scroll.querySelectorAll('.grid')];
    const at = grids.indexOf(grid);
    let target: HTMLButtonElement | undefined;
    if (event.key === 'ArrowRight' || event.key === 'ArrowLeft') {
      const all = [...scroll.querySelectorAll<HTMLButtonElement>('.cell')];
      target = all[all.indexOf(cell) + (event.key === 'ArrowRight' ? 1 : -1)];
    } else if (event.key === 'ArrowDown') {
      target = cells[index + columns];
      if (!target && index + columns >= cells.length) {
        // Past the last row: the same column in the next section (or this section's last cell).
        const lastRow = Math.floor((cells.length - 1) / columns) * columns;
        const next = index >= lastRow ? grids[at + 1] : undefined;
        target = next ? ((next.children[Math.min(index % columns, next.children.length - 1)] as HTMLButtonElement) ?? undefined) : cells.at(-1);
      }
    } else if (event.key === 'ArrowUp') {
      if (index >= columns) target = cells[index - columns];
      else if (at > 0) {
        const previous = grids[at - 1]!;
        const length = previous.children.length;
        const previousColumns = columnsOf(previous);
        target = previous.children[Math.min(Math.floor((length - 1) / previousColumns) * previousColumns + index, length - 1)] as HTMLButtonElement;
      } else {
        event.preventDefault();
        search?.focus();
        return;
      }
    } else if (event.key === 'Home' || event.key === 'End') {
      const all = scroll.querySelectorAll<HTMLButtonElement>('.cell');
      target = event.key === 'Home' ? all[0] : all[all.length - 1];
    } else return;
    event.preventDefault();
    target?.focus();
    target?.scrollIntoView({ block: 'nearest' });
  }

  function onsearchkeydown(event: KeyboardEvent) {
    if (event.key === 'ArrowDown') {
      const first = scroll?.querySelector<HTMLButtonElement>('.cell');
      if (!first) return;
      event.preventDefault();
      first.focus();
    } else if (event.key === 'Enter' && query.trim()) {
      // The best match, without leaving the field.
      event.preventDefault();
      if (customResults[0]) pickCustom(customResults[0]);
      else if (results[0]) pickUnicode(results[0].char);
    }
  }

  /** Tabbing out of the picker closes it, like a click outside does. */
  function onfocusout(event: FocusEvent) {
    const next = event.relatedTarget as Node | null;
    if (next && !(event.currentTarget as Element).contains(next)) onclose();
  }
</script>

<svelte:window onmousedown={(event) => !(event.target as Element).closest?.('.picker') && onclose()} />

<div
  class="picker"
  style:top="{placement.top}px"
  style:left="{placement.left}px"
  style:width="{WIDTH}px"
  style:height="{HEIGHT}px"
  role="dialog"
  aria-label={stickers ? 'Emoji and stickers' : 'Add reaction'}
  use:popover={onclose}
  {onfocusout}
>
  <div class="top">
    {#if stickers}
      <div class="tabs" role="tablist">
        <!-- Left/Right switch tabs (only the selected one is a tab stop). -->
        <button role="tab" class="tab" class:on={tab === 'emoji'} aria-selected={tab === 'emoji'} aria-controls="picker-panel" tabindex={tab === 'emoji' ? 0 : -1} onclick={() => (tab = 'emoji')} onkeydown={(e) => e.key === 'ArrowRight' && ((tab = 'stickers'), (e.currentTarget.nextElementSibling as HTMLElement | null)?.focus())}><Icon name="smile" size={15} /> Emoji</button>
        <button role="tab" class="tab" class:on={tab === 'stickers'} aria-selected={tab === 'stickers'} aria-controls="picker-panel" tabindex={tab === 'stickers' ? 0 : -1} onclick={() => (tab = 'stickers')} onkeydown={(e) => e.key === 'ArrowLeft' && ((tab = 'emoji'), (e.currentTarget.previousElementSibling as HTMLElement | null)?.focus())}><Icon name="sticker" size={15} /> Stickers</button>
      </div>
    {/if}
    {#if tab === 'emoji'}
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="input"
        type="search"
        placeholder="Search emoji"
        bind:this={search}
        bind:value={query}
        autofocus
        spellcheck="false"
        autocomplete="off"
        aria-label="Search emoji"
        aria-describedby="picker-hint"
        onkeydown={onsearchkeydown}
      />
      <span class="sr-only" id="picker-hint">Enter picks the best match; Down arrow moves into the list.</span>
    {/if}
  </div>

  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="scroll" id="picker-panel" role={stickers ? 'tabpanel' : 'group'} aria-label={stickers ? undefined : 'Emoji'} bind:this={scroll} onkeydown={ongridkeydown}>
    {#if tab === 'stickers'}
      {#each stickerPacks as pack (pack.name)}
        <h3 class="eyebrow">{pack.name}</h3>
        <div class="grid stickers">
          {#each pack.emojis as sticker (sticker.mxc_url)}
            <button tabindex="-1" class="cell sticker" title={sticker.shortcode} onclick={() => pickSticker({ url: sticker.mxc_url, body: sticker.shortcode, width: sticker.width, height: sticker.height })}>
              <img src={backend.mediaUrl(sticker.mxc_url)} alt={sticker.shortcode} loading="lazy" />
            </button>
          {/each}
        </div>
      {/each}
      {#if emojiStore.stickers.length > 0}
        <h3 class="eyebrow">Your collection</h3>
        <div class="grid stickers">
          {#each emojiStore.stickers as sticker (sticker.url)}
            <button tabindex="-1" class="cell sticker" title={sticker.body} onclick={() => pickSticker(sticker)}>
              <img src={backend.mediaUrl(sticker.url)} alt={sticker.body} loading="lazy" />
            </button>
          {/each}
        </div>
      {/if}
      {#if stickerPacks.length === 0 && emojiStore.stickers.length === 0}
        <p class="muted empty">No stickers yet. Stickers you see in rooms are collected here, and sticker packs from your rooms and spaces show up automatically.</p>
      {/if}
    {:else if query.trim()}
      {#if customResults.length > 0}
        <h3 class="eyebrow">Custom</h3>
        <div class="grid">
          {#each customResults as custom (custom.mxc_url)}
            <button tabindex="-1" class="cell" title=":{custom.shortcode}:" onclick={() => pickCustom(custom)}><img class="custom" src={backend.mediaUrl(custom.mxc_url)} alt={custom.shortcode} loading="lazy" /></button>
          {/each}
        </div>
      {/if}
      {#if results.length > 0}
        <div class="grid">
          {#each results as result (result.hexcode)}
            <button tabindex="-1" class="cell" title={result.label} onclick={() => pickUnicode(result.char)}><Twemoji char={result.char} size={26} /></button>
          {/each}
        </div>
      {:else if customResults.length === 0}
        <p class="muted empty">{data ? 'No emoji found.' : 'Loading…'}</p>
      {/if}
    {:else}
      {#if frequent.length > 0}
        <h3 class="eyebrow">Frequently used</h3>
        <div class="grid">
          {#each frequent as pick (pick.kind === 'unicode' ? pick.char : pick.kind === 'custom' ? pick.emoji.mxc_url : '')}
            {#if pick.kind === 'unicode'}
              <button tabindex="-1" class="cell" aria-label={pick.char} onclick={() => pickUnicode(pick.char)}><Twemoji char={pick.char} size={26} /></button>
            {:else if pick.kind === 'custom'}
              <button tabindex="-1" class="cell" title=":{pick.emoji.shortcode}:" onclick={() => pickCustom(pick.emoji)}><img class="custom" src={backend.mediaUrl(pick.emoji.mxc_url)} alt={pick.emoji.shortcode} loading="lazy" /></button>
            {/if}
          {/each}
        </div>
      {/if}
      {#each customEmoji as pack (pack.name)}
        <h3 class="eyebrow">{pack.name}</h3>
        <div class="grid">
          {#each pack.emojis as custom (custom.mxc_url)}
            <button tabindex="-1" class="cell" title=":{custom.shortcode}:" onclick={() => pickCustom(custom)}><img class="custom" src={backend.mediaUrl(custom.mxc_url)} alt={custom.shortcode} loading="lazy" /></button>
          {/each}
        </div>
      {/each}
      {#if data}
        {#each data.categories as category (category.key)}
          <h3 class="eyebrow">{category.label}</h3>
          <div class="grid section">
            {#each category.emojis as unicode (unicode.hexcode)}
              <button tabindex="-1" class="cell" title={unicode.label} onclick={() => pickUnicode(unicode.char)}><Twemoji char={unicode.char} size={26} /></button>
            {/each}
          </div>
        {/each}
      {:else if failed}
        <p class="error-text empty">Couldn't load the emoji list.</p>
      {:else}
        <p class="muted empty">Loading…</p>
      {/if}
    {/if}
  </div>
</div>

<style>
  .picker {
    position: fixed;
    z-index: 70;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: var(--shadow-lg);
    overflow: hidden;
    animation: fade 0.1s var(--ease);
  }
  .top {
    display: grid;
    gap: 8px;
    padding: 8px;
    border-bottom: 1px solid var(--border);
  }
  .tabs {
    display: flex;
    gap: 4px;
  }
  .tab {
    display: inline-flex;
    flex: 1;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 5px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--muted);
  }
  .tab.on {
    background: var(--surface-strong);
    color: var(--text);
  }
  .scroll {
    flex: 1 1 auto;
    padding: 0 8px 8px;
    overflow-y: auto;
  }
  h3 {
    position: sticky;
    top: 0;
    z-index: 1;
    padding: 8px 2px 4px;
    background: var(--surface);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(34px, 1fr));
    gap: 2px;
  }
  .grid.section {
    content-visibility: auto;
    contain-intrinsic-size: auto 300px;
  }
  .grid.stickers {
    grid-template-columns: repeat(auto-fill, minmax(76px, 1fr));
    gap: 6px;
  }
  .cell {
    display: grid;
    place-items: center;
    aspect-ratio: 1;
    padding: 0;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
  }
  .cell:hover,
  .cell:focus-visible {
    background: var(--hover);
  }
  .cell:focus-visible {
    outline-offset: -2px;
  }
  .cell img.custom {
    width: 28px;
    height: 28px;
    object-fit: contain;
  }
  .cell.sticker img {
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .empty {
    padding: 16px 4px;
    text-align: center;
  }
</style>
