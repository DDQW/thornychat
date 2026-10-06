import { backend } from '../api';
import type { CustomEmoji, EmojiPack, TimelineItem } from '../bindings';

/** Cap on the grow-with-use sticker collection: most recent kept, oldest dropped. */
export const MAX_COLLECTED_STICKERS = 120;

/** A sticker seen in a room or picked from a pack, kept so it can be sent again. */
export interface CollectedSticker {
  /** `mxc://` URL of the sticker image. */
  url: string;
  /** Alt text / shortcode, reused as the event body when resending. */
  body: string;
  width: number | null;
  height: number | null;
}

class EmojiStore {
  /** Every custom emoji/sticker pack this account can currently use. */
  packs = $state.raw<EmojiPack[]>([]);
  /** Emoji use counts behind "Frequently used": unicode glyph or `mxc://` URL → count. */
  usage = $state.raw<Record<string, number>>({});
  stickers = $state.raw<CollectedSticker[]>([]);

  /** Shortcode (without colons) → custom emoji, for `:name:` autocomplete. First pack wins. */
  byShortcode = $derived.by(() => {
    const map = new Map<string, CustomEmoji>();
    for (const pack of this.packs) {
      for (const emoji of pack.emojis) {
        if (emoji.is_emoticon && !map.has(emoji.shortcode)) map.set(emoji.shortcode, emoji);
      }
    }
    return map;
  });

  /**
   * Lowercased shortcode → pack image, for turning a `:name:` in a message or
   * a reaction key into the picture. Case-insensitive like the old client
   * (packs say `NotSure`, people type `:notsure:`); emoticons win over
   * sticker-only images of the same name.
   */
  #byLowerShortcode = $derived.by(() => {
    const map = new Map<string, CustomEmoji>();
    for (const emoticons of [true, false]) {
      for (const pack of this.packs) {
        for (const emoji of pack.emojis) {
          const key = emoji.shortcode.toLowerCase();
          if (emoji.is_emoticon === emoticons && !map.has(key)) map.set(key, emoji);
        }
      }
    }
    return map;
  });

  /** The pack image a shortcode (with or without its colons) names, if any. */
  resolve(shortcode: string): CustomEmoji | undefined {
    return this.#byLowerShortcode.get(shortcode.replace(/^:|:$/g, '').toLowerCase());
  }

  /**
   * The custom emoji a message body uses, by `:shortcode:` — what the send
   * commands need to turn them into inline images other clients can show.
   * Same matching as the timeline's (`richtext.ts`), so what renders as an
   * image locally is exactly what is sent as one.
   */
  usedIn(body: string): CustomEmoji[] {
    const used = new Map<string, CustomEmoji>();
    const pattern = /:([\w+-]{1,64})(?=:)/g;
    for (let match = pattern.exec(body); match; match = pattern.exec(body)) {
      const emoji = this.resolve(match[1]!);
      if (!emoji) continue;
      used.set(emoji.shortcode, emoji);
      pattern.lastIndex = match.index + match[0].length + 1;
    }
    return [...used.values()];
  }

  /** `mxc://` URL → custom emoji, for rendering reactions keyed by the image. */
  byMxc = $derived.by(() => {
    const map = new Map<string, CustomEmoji>();
    for (const pack of this.packs) for (const emoji of pack.emojis) map.set(emoji.mxc_url, emoji);
    return map;
  });

  /** The most-used emoji keys, best first. */
  frequent = $derived(
    Object.entries(this.usage)
      .sort((a, b) => b[1] - a[1])
      .slice(0, 24)
      .map(([key]) => key),
  );

  #persistTimers = new Map<string, ReturnType<typeof setTimeout>>();

  async load(): Promise<void> {
    const [usage, stickers] = await Promise.all([
      backend.loadProfileFile('emoji_usage').catch(() => null),
      backend.loadProfileFile('stickers').catch(() => null),
    ]);
    this.usage = parseJson<Record<string, number>>(usage, isCountMap) ?? {};
    this.stickers = parseJson<CollectedSticker[]>(stickers, isStickerList) ?? [];
  }

  recordUse(key: string): void {
    this.usage = { ...this.usage, [key]: (this.usage[key] ?? 0) + 1 };
    this.#persistSoon('emoji_usage', () => JSON.stringify(this.usage));
  }

  /** Remembers any stickers in `items` (newest first, deduped by URL, capped). */
  harvest(items: readonly TimelineItem[]): void {
    let next = this.stickers;
    for (const item of items) {
      if (item.content.type !== 'Sticker') continue;
      const { url, body, width, height } = item.content.data;
      if (next.some((sticker) => sticker.url === url)) continue;
      next = [{ url, body, width, height }, ...next].slice(0, MAX_COLLECTED_STICKERS);
    }
    if (next !== this.stickers) {
      this.stickers = next;
      this.#persistSoon('stickers', () => JSON.stringify(this.stickers));
    }
  }

  /** Writes a few seconds after the last change: picking emoji in a burst is one write. */
  #persistSoon(file: 'emoji_usage' | 'stickers', serialize: () => string): void {
    const pending = this.#persistTimers.get(file);
    if (pending) clearTimeout(pending);
    this.#persistTimers.set(
      file,
      setTimeout(() => {
        this.#persistTimers.delete(file);
        void backend.saveProfileFile(file, serialize()).catch(() => {});
      }, 2000),
    );
  }

  reset(): void {
    this.packs = [];
    this.usage = {};
    this.stickers = [];
    for (const timer of this.#persistTimers.values()) clearTimeout(timer);
    this.#persistTimers.clear();
  }
}

function parseJson<T>(text: string | null, guard: (value: unknown) => value is T): T | null {
  if (!text) return null;
  try {
    const value: unknown = JSON.parse(text);
    return guard(value) ? value : null;
  } catch {
    return null;
  }
}

function isCountMap(value: unknown): value is Record<string, number> {
  return typeof value === 'object' && value !== null && !Array.isArray(value) && Object.values(value).every((n) => typeof n === 'number');
}

function isStickerList(value: unknown): value is CollectedSticker[] {
  return Array.isArray(value) && value.every((s) => typeof s === 'object' && s !== null && typeof (s as CollectedSticker).url === 'string' && typeof (s as CollectedSticker).body === 'string');
}

export const emoji = new EmojiStore();
