// Spell checking in the composer: which words to mark, which one the
// suggestion bar is about, and the autocorrect bookkeeping (apply on a word
// boundary, undo with the Backspace right after). The verdicts come from the
// Windows speller in the shell (crates/desktop/src/spellcheck.rs); everything
// here is plain text in, plain data out, so it is tested without one.
//
// Offsets are UTF-16 code units throughout — what `selectionStart` and
// `String.slice` speak.

/** One whitespace-delimited token of the draft, plus the alphanumeric core the speller sees. */
export interface Word {
  /** Range of `core` in the draft. */
  start: number;
  end: number;
  /** Range of `raw` in the draft. */
  rawStart: number;
  rawEnd: number;
  /** The token trimmed to its outermost alphanumeric characters. */
  core: string;
  /** The whole whitespace-delimited token. */
  raw: string;
}

const ALNUM = /[\p{Alphabetic}\p{N}]/u;
const ALPHA = /\p{Alphabetic}/u;
const UPPER = /\p{Uppercase}/u;
const LOWER = /\p{Lowercase}/u;
const DIGIT = /\p{N}/u;

/**
 * Every word in `text`, in order. Splits on whitespace only — the speller does
 * its own word breaking, and splitting on punctuation here would tear "don't"
 * and "e.g." apart before it ever saw them. The distinction between `raw` and
 * `core` matters both ways: skip decisions need the whole token to recognise a
 * URL or a mention, while a replacement must land on the core alone so
 * "helo," keeps its comma. Tokens with no alphanumeric character are skipped.
 */
export function words(text: string): Word[] {
  const found: Word[] = [];
  for (const match of text.matchAll(/\S+/gu)) {
    const raw = match[0];
    const rawStart = match.index;
    let lead = -1;
    let trail = -1;
    let offset = 0;
    for (const char of raw) {
      if (ALNUM.test(char)) {
        if (lead < 0) lead = offset;
        trail = offset + char.length;
      }
      offset += char.length;
    }
    if (lead < 0) continue;
    found.push({ start: rawStart + lead, end: rawStart + trail, rawStart, rawEnd: rawStart + raw.length, core: raw.slice(lead, trail), raw });
  }
  return found;
}

/**
 * Chat register the OS dictionary doesn't carry. These are not typos, and the
 * speller's guesses at them are not near misses — they are the closest
 * *dictionary* word, which for informal English is reliably wrong: "goanna"
 * for "gonna", "urn" for "ur", "mi" for "im", "prolyl" for "prolly", "irk" for
 * "ikr". Marking them is noise and autocorrecting them is worse, so they are
 * not checked at all.
 *
 * Deliberately narrow: every entry is a word the speller was observed to flag
 * *and* to suggest badly for. Apostrophe-less contractions it fixes correctly
 * ("dont" → "don't", "thats", "whats", "shes") stay checkable on purpose.
 */
export const CHAT_WORDS: readonly string[] = [
  'aint', 'cuz', 'dunno', 'finna', 'gonna', 'gotta', 'hes', 'iirc', 'ikr',
  'im', 'imho', 'imma', 'imo', 'irl', 'ive', 'kinda', 'lemme', 'ngl', 'nvm',
  'plz', 'prolly', 'smh', 'sorta', 'tbh', 'tho', 'tryna', 'ur', 'wanna',
  'wdym', 'yall',
];
const CHAT_SET = new Set(CHAT_WORDS);

/**
 * Whether a raw token is ordinary prose worth spell-checking — filters out what
 * chat is full of that a dictionary would wrongly flag: mentions, emoji
 * shortcodes, URLs and paths, code identifiers, acronyms, anything with a digit.
 */
export function isCheckable(raw: string): boolean {
  const letters = [...raw].filter((char) => ALPHA.test(char));
  // Needs at least two letters to be a word worth checking.
  if (letters.length < 2) return false;
  // Chat slang first: correctly spelled here even though no dictionary says
  // so. Edge punctuation is trimmed so "gonna," matches; internal punctuation
  // is not, so "he's" stays checkable while "hes" doesn't.
  const bare = raw.replace(/^[^\p{Alphabetic}\p{N}]+|[^\p{Alphabetic}\p{N}]+$/gu, '').toLowerCase();
  if (CHAT_SET.has(bare)) return false;
  // Mentions and emoji shortcodes.
  if (raw.startsWith('@') || raw.startsWith(':') || raw.includes('@')) return false;
  // URLs, paths, snake_case identifiers.
  if (raw.includes('://') || raw.includes('/') || raw.includes('\\') || raw.includes('_') || raw.startsWith('www.')) return false;
  // Versions, ids, l33t — anything with a digit.
  if (DIGIT.test(raw)) return false;
  // ALL-CAPS acronyms (GG, LOL) and MixedCase identifiers (camelCase): neither.
  // A plain Capitalized word is fine; autocorrect guards those separately.
  const allUpper = letters.every((char) => UPPER.test(char));
  const internalUpper = letters.slice(1).some((char) => UPPER.test(char));
  return !(allUpper || internalUpper);
}

/**
 * The word the caret is at the trailing edge of — the one being typed right
 * now. Deliberately not "the word the caret is inside": clicking into the
 * middle of a finished typo has to leave it marked, or the suggestion bar
 * would empty out at the exact moment you reached for it.
 */
export function wordBeingTyped(text: string, caret: number): Word | null {
  return words(text).find((word) => word.rawEnd === caret) ?? null;
}

/** The word the caret has just finished: the last one that ends *before* it. */
export function wordBeforeCaret(text: string, caret: number): Word | null {
  let last: Word | null = null;
  for (const word of words(text)) {
    if (word.rawEnd >= caret) break;
    last = word;
  }
  return last;
}

/** The word the suggestion bar is about: the one the caret is in, or — on whitespace — the one it just left. */
export function flagTarget(text: string, caret: number): Word | null {
  let last: Word | null = null;
  for (const word of words(text)) {
    if (word.rawStart > caret) break;
    last = word;
  }
  return last;
}

/** Autocorrect only rewrites words that start lowercase; a capital usually marks a name. */
export function startsLowercase(word: string): boolean {
  const first = [...word][0];
  return first !== undefined && LOWER.test(first);
}

export interface SpellScan {
  /** Misspelled words, by text: a repeated typo is marked everywhere it appears. */
  misspelled: ReadonlySet<string>;
  /** Where to draw the marks, in draft order. */
  marks: Word[];
  /** Checkable words the speller hasn't been asked about yet. */
  unknown: string[];
  /** The marked word at the caret, for the suggestion bar. */
  target: Word | null;
}

export const EMPTY_SCAN: SpellScan = { misspelled: new Set(), marks: [], unknown: [], target: null };

/**
 * Checks `text` against the verdicts known so far. `verdict` answers `true`
 * (misspelled), `false`, or `undefined` (not asked yet — returned in
 * `unknown`). `skip` excludes words the caller knows are not prose, such as
 * the parts of a mention's display name.
 *
 * The word under the caret is still being typed; marking it would paint it
 * halfway through and unpaint it at the end. Words are matched by text, so
 * that spares an identical word elsewhere in the draft too — until the next
 * space brings it back. It isn't asked about either, so the speller isn't
 * consulted on every prefix of every word.
 */
export function scanDraft(
  text: string,
  caret: number,
  verdict: (word: string) => boolean | undefined,
  skip: (word: Word) => boolean = () => false,
): SpellScan {
  const all = words(text);
  const inProgress = all.find((word) => word.rawEnd === caret)?.core ?? null;
  const prose = all.filter((word) => isCheckable(word.raw) && !skip(word));
  const misspelled = new Set<string>();
  const unknown = new Set<string>();
  for (const word of prose) {
    if (word.core === inProgress) continue;
    const known = verdict(word.core);
    if (known === undefined) unknown.add(word.core);
    else if (known) misspelled.add(word.core);
  }
  if (misspelled.size === 0) return { misspelled, marks: [], unknown: [...unknown], target: null };
  const marks = prose.filter((word) => misspelled.has(word.core));
  const at = flagTarget(text, caret);
  const target = at && marks.find((word) => word.start === at.start) ? at : null;
  return { misspelled, marks, unknown: [...unknown], target };
}

export interface Segment {
  text: string;
  /** Draft offset of the segment, so a marked one can be mapped back to its word. */
  start: number;
  typo: boolean;
}

/** `text` cut into plain runs and marked words, for the overlay behind the textarea. */
export function markSegments(text: string, marks: readonly Word[]): Segment[] {
  const segments: Segment[] = [];
  let at = 0;
  for (const mark of marks) {
    if (mark.start < at) continue;
    if (mark.start > at) segments.push({ text: text.slice(at, mark.start), start: at, typo: false });
    segments.push({ text: text.slice(mark.start, mark.end), start: mark.start, typo: true });
    at = mark.end;
  }
  if (at < text.length) segments.push({ text: text.slice(at), start: at, typo: false });
  return segments;
}

// --- autocorrect ---

export interface Draft {
  text: string;
  caret: number;
}

/** What kind of edit an `input` event was, as far as autocorrect cares. */
export type EditKind = 'delete-backward' | 'word-end' | 'other';

/**
 * A space or a line break finishes a word — the moment autocorrect gets to act.
 * A paste never does: it can drop in any amount of text, and silently rewriting
 * part of what someone pasted is not a fix anyone asked for. Nor does anything
 * mid-composition in an IME.
 */
export function editKind(event: { inputType: string; data: string | null; isComposing?: boolean }): EditKind {
  if (event.isComposing) return 'other';
  if (event.inputType === 'deleteContentBackward') return 'delete-backward';
  if (event.inputType === 'insertLineBreak' || event.inputType === 'insertParagraph') return 'word-end';
  if (event.inputType === 'insertText' && event.data !== null && /^\s$/u.test(event.data)) return 'word-end';
  return 'other';
}

/** How to take an autocorrect back. */
export interface Revert {
  /** Where the correction sits in the draft autocorrect left behind. */
  start: number;
  end: number;
  /** What autocorrect put there — checked first, so a draft edited out from under it is never corrupted. */
  corrected: string;
  /** What the user typed, to put back. */
  original: string;
}

/**
 * The draft to restore when the Backspace that just ran was the "undo the
 * autocorrect" one: the caret must have landed exactly where the boundary
 * character used to be, with the correction still intact.
 */
export function undoRevert(revert: Revert, draft: Draft): Draft | null {
  if (draft.caret !== revert.end || draft.text.slice(revert.start, revert.end) !== revert.corrected) return null;
  return {
    text: draft.text.slice(0, revert.start) + revert.original + draft.text.slice(revert.end),
    caret: revert.start + revert.original.length,
  };
}

/**
 * One draft's autocorrect state. Programmatic edits (emoji, mentions, picked
 * suggestions) call `interrupt`; sending calls `reset`.
 */
export class DraftSpeller {
  /**
   * Words un-corrected with the Backspace after an autocorrect. Autocorrect
   * leaves these alone for the rest of the draft; without this, undoing would
   * achieve nothing — finishing the word again would re-apply the same fix,
   * and the spelling the user wants could never survive a space. The
   * suggestion bar is deliberately not gated on it: the word stays marked and
   * the fix stays one click away, it just stops happening by itself.
   */
  #rejected = new Set<string>();
  /** Good for exactly one edit after an autocorrect. */
  #revert: Revert | null = null;
  /** Bumped by `reset`, so a correction still in flight for the old draft is dropped. */
  #generation = 0;

  /**
   * Call for every `input` event, with the draft as it stands after it.
   * Returns the draft to adopt when this edit was the Backspace that undoes an
   * autocorrect; that word is then rejected for the rest of the draft.
   */
  afterEdit(kind: EditKind, draft: Draft): Draft | null {
    const revert = this.#revert;
    this.#revert = null;
    if (kind !== 'delete-backward' || !revert) return null;
    const undone = undoRevert(revert, draft);
    if (undone) this.#rejected.add(revert.original);
    return undone;
  }

  /**
   * Corrects the word just finished at `draft.caret`, if the speller offers a
   * confident fix. The answer comes back asynchronously, so `current` reads the
   * draft at that point: the fix is applied only if the word is still there,
   * untouched and still finished, and the caret keeps its place relative to
   * the text after it. Resolves to the draft to adopt, or null.
   */
  async autocorrect(draft: Draft, correction: (word: string) => Promise<string | null>, current: () => Draft): Promise<Draft | null> {
    const word = wordBeforeCaret(draft.text, draft.caret);
    // Mentions, URLs and code stay as typed; so do capitalised words (names,
    // sentence starts) — the suggestion bar still offers those.
    if (!word || !isCheckable(word.raw) || !startsLowercase(word.core) || this.#rejected.has(word.core)) return null;
    const generation = this.#generation;
    const fix = await correction(word.core);
    if (!fix || fix === word.core || generation !== this.#generation) return null;

    const now = current();
    if (now.text.slice(word.rawStart, word.rawEnd) !== word.raw || !/^\s/u.test(now.text.slice(word.rawEnd))) return null;
    const delta = fix.length - word.core.length;
    const caret = now.caret >= word.end ? now.caret + delta : now.caret;
    this.#revert = { start: word.start, end: word.start + fix.length, corrected: fix, original: word.core };
    return { text: now.text.slice(0, word.start) + fix + now.text.slice(word.end), caret };
  }

  /** A programmatic edit happened; the next Backspace is an ordinary one. */
  interrupt(): void {
    this.#revert = null;
  }

  /** A fresh draft: nothing rejected, nothing to undo, nothing in flight. */
  reset(): void {
    this.#rejected.clear();
    this.#revert = null;
    this.#generation++;
  }
}

// --- verdicts ---

/**
 * Remembered speller verdicts, shared by every composer (the dictionary is
 * global). Each keystroke re-scans the whole draft, so without this a long
 * draft would cross to the shell for every word on every key. Keyed by word,
 * not position: an edit earlier in the draft shifts positions, not words.
 */
export class VerdictCache {
  #known = new Map<string, boolean>();
  #asking = new Set<string>();
  /** Bumped by `clear`, so answers to questions asked before it are not stored. */
  #epoch = 0;

  constructor(
    private readonly check: (words: string[]) => Promise<boolean[]>,
    /** A latency trick, not a store: past this many words, start over. */
    private readonly cap = 512,
  ) {}

  get(word: string): boolean | undefined {
    return this.#known.get(word);
  }

  /** Asks about the words neither known nor already asked about; resolves whether anything was learned. */
  async ask(words: readonly string[]): Promise<boolean> {
    const fresh = [...new Set(words)].filter((word) => !this.#known.has(word) && !this.#asking.has(word));
    if (fresh.length === 0) return false;
    for (const word of fresh) this.#asking.add(word);
    const epoch = this.#epoch;
    let verdicts: boolean[];
    try {
      verdicts = await this.check(fresh);
    } catch {
      // No speller: nothing is misspelled, and asking again won't change that.
      verdicts = [];
    } finally {
      for (const word of fresh) this.#asking.delete(word);
    }
    if (epoch !== this.#epoch) return false;
    if (this.#known.size + fresh.length > this.cap) this.#known.clear();
    fresh.forEach((word, index) => this.#known.set(word, verdicts[index] === true));
    return true;
  }

  /** The personal dictionary changed, which can flip the answer for any word. */
  clear(): void {
    this.#known.clear();
    this.#epoch++;
  }
}
