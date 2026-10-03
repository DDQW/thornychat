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
</script>

<svelte:window onmousedown={(event) => !(event.target as Element).closest?.('.picker') && onclose()} />

<div class="picker" style:top="{placement.top}px" style:left="{placement.left}px" style:width="{WIDTH}px" style:height="{HEIGHT}px" role="dialog" aria-label="Emoji picker">
  <div class="top">
    {#if stickers}
      <div class="tabs" role="tablist">
        <button role="tab" class="tab" class:on={tab === 'emoji'} aria-selected={tab === 'emoji'} onclick={() => (tab = 'emoji')}><Icon name="smile" size={15} /> Emoji</button>
        <button role="tab" class="tab" class:on={tab === 'stickers'} aria-selected={tab === 'stickers'} onclick={() => (tab = 'stickers')}><Icon name="sticker" size={15} /> Stickers</button>
      </div>
    {/if}
    {#if tab === 'emoji'}
      <!-- svelte-ignore a11y_autofocus -->
      <input class="input" placeholder="Search emoji" bind:value={query} autofocus spellcheck="false" aria-label="Search emoji" />
    {/if}
  </div>

  <div class="scroll">
    {#if tab === 'stickers'}
      {#each stickerPacks as pack (pack.name)}
        <h3>{pack.name}</h3>
        <div class="grid stickers">
          {#each pack.emojis as sticker (sticker.mxc_url)}
            <button class="cell sticker" title={sticker.shortcode} onclick={() => pickSticker({ url: sticker.mxc_url, body: sticker.shortcode, width: sticker.width, height: sticker.height })}>
              <img src={backend.mediaUrl(sticker.mxc_url)} alt={sticker.shortcode} loading="lazy" />
            </button>
          {/each}
        </div>
      {/each}
      {#if emojiStore.stickers.length > 0}
        <h3>Your collection</h3>
        <div class="grid stickers">
          {#each emojiStore.stickers as sticker (sticker.url)}
            <button class="cell sticker" title={sticker.body} onclick={() => pickSticker(sticker)}>
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
        <h3>Custom</h3>
        <div class="grid">
          {#each customResults as custom (custom.mxc_url)}
            <button class="cell" title=":{custom.shortcode}:" onclick={() => pickCustom(custom)}><img class="custom" src={backend.mediaUrl(custom.mxc_url)} alt={custom.shortcode} loading="lazy" /></button>
          {/each}
        </div>
      {/if}
      {#if results.length > 0}
        <div class="grid">
          {#each results as result (result.hexcode)}
            <button class="cell" title={result.label} onclick={() => pickUnicode(result.char)}><Twemoji char={result.char} size={26} /></button>
          {/each}
        </div>
      {:else if customResults.length === 0}
        <p class="muted empty">{data ? 'No emoji found.' : 'Loading…'}</p>
      {/if}
    {:else}
      {#if frequent.length > 0}
        <h3>Frequently used</h3>
        <div class="grid">
          {#each frequent as pick (pick.kind === 'unicode' ? pick.char : pick.kind === 'custom' ? pick.emoji.mxc_url : '')}
            {#if pick.kind === 'unicode'}
              <button class="cell" onclick={() => pickUnicode(pick.char)}><Twemoji char={pick.char} size={26} /></button>
            {:else if pick.kind === 'custom'}
              <button class="cell" title=":{pick.emoji.shortcode}:" onclick={() => pickCustom(pick.emoji)}><img class="custom" src={backend.mediaUrl(pick.emoji.mxc_url)} alt={pick.emoji.shortcode} loading="lazy" /></button>
            {/if}
          {/each}
        </div>
      {/if}
      {#each customEmoji as pack (pack.name)}
        <h3>{pack.name}</h3>
        <div class="grid">
          {#each pack.emojis as custom (custom.mxc_url)}
            <button class="cell" title=":{custom.shortcode}:" onclick={() => pickCustom(custom)}><img class="custom" src={backend.mediaUrl(custom.mxc_url)} alt={custom.shortcode} loading="lazy" /></button>
          {/each}
        </div>
      {/each}
      {#if data}
        {#each data.categories as category (category.key)}
          <h3>{category.label}</h3>
          <div class="grid section">
            {#each category.emojis as unicode (unicode.hexcode)}
              <button class="cell" title={unicode.label} onclick={() => pickUnicode(unicode.char)}><Twemoji char={unicode.char} size={26} /></button>
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
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: 0 16px 48px rgb(0 0 0 / 0.5);
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
    margin: 0;
    padding: 8px 2px 4px;
    background: var(--surface);
    color: var(--muted);
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.05em;
    text-transform: uppercase;
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
  .cell:hover {
    background: var(--hover);
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
