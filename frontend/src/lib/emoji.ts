// Unicode emoji: Twemoji filenames, picker data, and search.

/**
 * Converts an emoji grapheme cluster into Twemoji's filename convention:
 * lowercase hex codepoints joined by `-`. The variation selector (U+FE0F) is
 * stripped ONLY when the sequence has no zero-width joiner — twemoji.js's exact
 * rule; ZWJ sequences (all the gendered/profession emoji) keep their FE0F in
 * the filename (`1f3c3-200d-2642-fe0f` exists on the CDN, the stripped form
 * 404s). The backend serves `/twemoji/<this>.svg` and validates the shape.
 */
export function twemojiCodepoints(emoji: string): string {
  const codepoints = [...emoji].map((char) => char.codePointAt(0)!);
  const hasZwj = codepoints.includes(0x200d);
  return codepoints
    .filter((cp) => hasZwj || cp !== 0xfe0f)
    .map((cp) => cp.toString(16))
    .join('-');
}

export interface PickerEmoji {
  /** The glyph to send, with this client's skin-tone preference applied. */
  char: string;
  label: string;
  tags: string[];
  hexcode: string;
}

export interface EmojiCategory {
  key: string;
  label: string;
  emojis: PickerEmoji[];
}

export interface EmojiData {
  categories: EmojiCategory[];
  all: PickerEmoji[];
  /** `:shortcode:` (without colons) → emoji, for composer autocomplete. */
  shortcodes: Map<string, PickerEmoji>;
}

interface RawEmoji {
  group?: number;
  hexcode: string;
  label: string;
  order?: number;
  tags?: string[];
  unicode: string;
  skins?: RawEmoji[];
}

const GROUP_LABELS: Record<number, [string, string]> = {
  0: ['smileys-emotion', 'Smileys & Emotion'],
  1: ['people-body', 'People & Body'],
  3: ['animals-nature', 'Animals & Nature'],
  4: ['food-drink', 'Food & Drink'],
  5: ['travel-places', 'Travel & Places'],
  6: ['activities', 'Activities'],
  7: ['objects', 'Objects'],
  8: ['symbols', 'Symbols'],
  9: ['flags', 'Flags'],
};

/**
 * This client's skin-tone preference, applied everywhere an emoji with tone
 * variants is shown or sent: light (the first variant), falling back to the
 * neutral yellow for emoji without any.
 */
export function preferredTone(raw: RawEmoji): string {
  return raw.skins?.[0]?.unicode ?? raw.unicode;
}

/** Groups the raw dataset into picker categories. Pure, for testing. */
export function buildEmojiData(raw: RawEmoji[], shortcodeTable: Record<string, string | string[]>): EmojiData {
  const categories = new Map<number, EmojiCategory>();
  const all: PickerEmoji[] = [];
  const shortcodes = new Map<string, PickerEmoji>();

  const ordered = raw.filter((entry) => entry.group !== undefined && GROUP_LABELS[entry.group]).sort((a, b) => (a.order ?? 0) - (b.order ?? 0));
  for (const entry of ordered) {
    const [key, label] = GROUP_LABELS[entry.group!]!;
    const emoji: PickerEmoji = {
      char: preferredTone(entry),
      label: entry.label,
      tags: entry.tags ?? [],
      hexcode: entry.hexcode,
    };
    let category = categories.get(entry.group!);
    if (!category) {
      category = { key, label, emojis: [] };
      categories.set(entry.group!, category);
    }
    category.emojis.push(emoji);
    all.push(emoji);

    const codes = shortcodeTable[entry.hexcode];
    for (const code of Array.isArray(codes) ? codes : codes ? [codes] : []) {
      if (!shortcodes.has(code)) shortcodes.set(code, emoji);
    }
  }
  return { categories: [...categories.values()], all, shortcodes };
}

let loading: Promise<EmojiData> | null = null;

/** Loads the emoji dataset on first use (it's ~0.5 MB, so it isn't in the main bundle). */
export function loadEmojiData(): Promise<EmojiData> {
  loading ??= (async () => {
    const [compact, shortcodes] = await Promise.all([
      import('emojibase-data/en/compact.json'),
      import('emojibase-data/en/shortcodes/cldr.json'),
    ]);
    return buildEmojiData(
      compact.default as unknown as RawEmoji[],
      shortcodes.default as unknown as Record<string, string | string[]>,
    );
  })();
  return loading;
}

/** Case-insensitive search over labels and tags; label matches rank first. */
export function searchEmoji(all: readonly PickerEmoji[], query: string, limit = 120): PickerEmoji[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return [];
  const labelHits: PickerEmoji[] = [];
  const tagHits: PickerEmoji[] = [];
  for (const emoji of all) {
    if (emoji.label.toLowerCase().includes(needle)) labelHits.push(emoji);
    else if (emoji.tags.some((tag) => tag.toLowerCase().includes(needle))) tagHits.push(emoji);
    if (labelHits.length >= limit) break;
  }
  return [...labelHits, ...tagHits].slice(0, limit);
}
