import { describe, expect, it } from 'vitest';
import corpusSource from '../../../crates/desktop/src/spellcheck/corpus.rs?raw';
import {
  CHAT_WORDS,
  DraftSpeller,
  editKind,
  flagTarget,
  isCheckable,
  markSegments,
  scanDraft,
  startsLowercase,
  undoRevert,
  VerdictCache,
  wordBeforeCaret,
  wordBeingTyped,
  words,
  type Draft,
} from './spell';

const collect = (text: string) => words(text).map((w) => [w.start, w.end, w.core, w.raw] as const);

describe('words', () => {
  it('split on whitespace and trim to the core', () => {
    expect(collect('i recieved teh file')).toEqual([
      [0, 1, 'i', 'i'],
      [2, 10, 'recieved', 'recieved'],
      [11, 14, 'teh', 'teh'],
      [15, 19, 'file', 'file'],
    ]);
  });

  it('trim punctuation from the core but keep it in raw', () => {
    // The range excludes the comma so a replacement can't eat it; the raw
    // token keeps it so the skip heuristics see the whole thing.
    expect(collect('(wat,)')).toEqual([[1, 4, 'wat', '(wat,)']]);
    // Internal punctuation stays — the speller does its own word breaking.
    expect(collect("don't")).toEqual([[0, 5, "don't", "don't"]]);
  });

  it('use UTF-16 offsets, the units selectionStart speaks', () => {
    // '😀' is two code units; 'é' one. The range must land on the word exactly.
    const text = '😀 café';
    const [word] = words(text);
    expect([word!.start, word!.end]).toEqual([3, 7]);
    expect(text.slice(word!.start, word!.end)).toBe('café');
  });

  it('skip tokens without a core rather than stopping at them', () => {
    expect(collect('ok --- teh')).toEqual([
      [0, 2, 'ok', 'ok'],
      [7, 10, 'teh', 'teh'],
    ]);
  });

  it('are not shifted by irregular whitespace', () => {
    expect(collect('  teh\tfile\n')).toEqual([
      [2, 5, 'teh', 'teh'],
      [6, 10, 'file', 'file'],
    ]);
  });
});

describe('isCheckable', () => {
  it('accepts prose and rejects chat tokens', () => {
    expect(isCheckable('teh')).toBe(true);
    expect(isCheckable('hello')).toBe(true);
    expect(isCheckable('Hello')).toBe(true); // capitalised is fine for marking

    expect(isCheckable('a')).toBe(false); // needs 2+ letters
    expect(isCheckable('GG')).toBe(false); // acronym
    expect(isCheckable('camelCase')).toBe(false); // code
    expect(isCheckable('v2')).toBe(false); // has a digit
    expect(isCheckable('@bob')).toBe(false); // mention
    expect(isCheckable(':smile:')).toBe(false); // emoji shortcode
    expect(isCheckable('http://x.com')).toBe(false); // url
    expect(isCheckable('a/b')).toBe(false); // path
    expect(isCheckable('co_op')).toBe(false); // identifier
  });

  it('leaves chat slang alone', () => {
    // The speller flags every one of these and suggests badly for it:
    // "goanna" for "gonna", "urn" for "ur", "prolyl" for "prolly".
    for (const word of ['gonna', 'wanna', 'ur', 'im', 'prolly', 'tbh', 'yall']) expect(isCheckable(word), word).toBe(false);
    // Edge punctuation is trimmed, so it matches in real prose...
    expect(isCheckable('gonna,')).toBe(false);
    expect(isCheckable('(tho)')).toBe(false);
    // ...but internal punctuation is not: "he's" is prose, only "hes" is slang.
    expect(isCheckable("he's")).toBe(true);
    expect(isCheckable('hes')).toBe(false);
  });

  it('keeps contractions the speller fixes correctly checkable', () => {
    for (const word of ['dont', 'thats', 'whats', 'shes']) expect(isCheckable(word), word).toBe(true);
  });

  it('has a lexicon of lowercase entries without duplicates', () => {
    expect(new Set(CHAT_WORDS).size).toBe(CHAT_WORDS.length);
    expect(CHAT_WORDS.every((word) => word === word.toLowerCase())).toBe(true);
  });

  it('lets every word of the autocorrect corpus through to the speller', () => {
    // The Rust corpus test feeds these to the speller directly; this keeps the
    // page's filter from silently hiding any of them.
    const typos = [...corpusSource.matchAll(/\("(\w+)","\w+"\)/g)].map((match) => match[1]!);
    expect(typos.length).toBeGreaterThan(100);
    for (const typo of typos) expect(isCheckable(typo), typo).toBe(true);
  });
});

describe('caret targets', () => {
  it('the finished word is the one left of the caret', () => {
    // "teh| cat": the space was just typed, the caret is at 4.
    expect(wordBeforeCaret('teh cat', 4)?.core).toBe('teh');
    // Nothing is finished until the caret has passed a boundary.
    expect(wordBeforeCaret('teh', 3)).toBeNull();
    expect(wordBeforeCaret('', 0)).toBeNull();
  });

  it('a word finished mid-line is found too', () => {
    const word = wordBeforeCaret('one teh two', 8);
    expect([word?.start, word?.end, word?.core]).toEqual([4, 7, 'teh']);
  });

  it("only the caret's trailing edge counts as still typing", () => {
    expect(wordBeingTyped('recieve', 7)?.core).toBe('recieve');
    // Clicked into the middle of it: finished, and stays marked.
    expect(wordBeingTyped('recieve', 3)).toBeNull();
    expect(wordBeingTyped('recieve ', 8)).toBeNull();
  });

  it('the bar targets the word the caret is in or just left', () => {
    expect(flagTarget('one teh two', 6)?.core).toBe('teh');
    expect(flagTarget('one teh two', 7)?.core).toBe('teh');
    expect(flagTarget('one teh two', 8)?.core).toBe('two');
    expect(flagTarget('   ', 3)).toBeNull();
  });

  it('autocorrect skips a leading capital', () => {
    expect(startsLowercase('teh')).toBe(true);
    expect(startsLowercase('Teh')).toBe(false);
    expect(startsLowercase('')).toBe(false);
  });
});

/** A verdict function that "knows" these words are misspelled and everything else is fine. */
const knows =
  (...misspelled: string[]) =>
  (word: string) =>
    misspelled.includes(word);

describe('scanDraft', () => {
  it('marks flagged words wherever they appear', () => {
    const scan = scanDraft('i recieved teh file and teh cat', 0, knows('recieved', 'teh'));
    expect(scan.marks.map((w) => [w.start, w.end])).toEqual([
      [2, 10],
      [11, 14],
      [24, 27],
    ]);
  });

  it('does not mark a token that is not prose, even when its text is flagged', () => {
    // "teh" inside a URL is not prose.
    const scan = scanDraft('http://teh/x teh ', 17, knows('teh'));
    expect(scan.marks.map((w) => w.start)).toEqual([13]);
  });

  it('never checks chat tokens', () => {
    for (const text of ['@alice:example.org ', 'https://example.org/page ', ':shrug: ', 'ACRONYM ']) {
      const scan = scanDraft(text, text.length, () => true);
      expect(scan.marks, text).toEqual([]);
      expect(scan.unknown, text).toEqual([]);
    }
  });

  it('does not mark (or ask about) the word still being typed', () => {
    let scan = scanDraft('teh', 3, knows('teh'));
    expect(scan.marks).toEqual([]);
    expect(scan.target).toBeNull();
    expect(scanDraft('teh', 3, () => undefined).unknown).toEqual([]);

    // The space finishes it, and now it is marked.
    scan = scanDraft('teh ', 4, knows('teh'));
    expect(scan.misspelled.has('teh')).toBe(true);
    expect(scan.marks).toHaveLength(1);
  });

  it('reports words with no verdict yet, once each', () => {
    expect(scanDraft('one two one three', 0, () => undefined).unknown).toEqual(['one', 'two', 'three']);
  });

  it('targets the flagged word at the caret for the suggestion bar', () => {
    const scan = scanDraft('teh ', 4, knows('teh'));
    expect([scan.target?.start, scan.target?.end, scan.target?.core]).toEqual([0, 3, 'teh']);
    // On a correct word, there is nothing to suggest.
    expect(scanDraft('teh cat ', 8, knows('teh')).target).toBeNull();
  });

  it('honours the skip list (a mention display name is not a typo)', () => {
    const text = '@John Smyth hi ';
    const scan = scanDraft(text, text.length, knows('Smyth'), (word) => word.start < 11);
    expect(scan.marks).toEqual([]);
  });

  it('cuts the draft into plain runs and marks for the overlay', () => {
    const text = 'i recieved teh file';
    const scan = scanDraft(text, 0, knows('recieved', 'teh'));
    expect(markSegments(text, scan.marks)).toEqual([
      { text: 'i ', start: 0, typo: false },
      { text: 'recieved', start: 2, typo: true },
      { text: ' ', start: 10, typo: false },
      { text: 'teh', start: 11, typo: true },
      { text: ' file', start: 14, typo: false },
    ]);
    expect(markSegments('', [])).toEqual([]);
  });
});

describe('editKind', () => {
  it('only a boundary keystroke finishes a word', () => {
    expect(editKind({ inputType: 'insertText', data: ' ' })).toBe('word-end');
    expect(editKind({ inputType: 'insertLineBreak', data: null })).toBe('word-end');
    expect(editKind({ inputType: 'insertText', data: 'x' })).toBe('other');
    expect(editKind({ inputType: 'deleteContentBackward', data: null })).toBe('delete-backward');
    // A paste can drop in any amount of text — never a correction cue.
    expect(editKind({ inputType: 'insertFromPaste', data: 'teh ' })).toBe('other');
    expect(editKind({ inputType: 'insertText', data: ' ', isComposing: true })).toBe('other');
  });
});

describe('undoRevert', () => {
  const revert = { start: 0, end: 3, corrected: 'the', original: 'teh' };
  it('restores the typed word only at the right caret', () => {
    // Backspace ate the space, leaving the caret at the word's end.
    expect(undoRevert(revert, { text: 'the', caret: 3 })).toEqual({ text: 'teh', caret: 3 });
    // Caret somewhere else — this Backspace wasn't the undo.
    expect(undoRevert(revert, { text: 'the cat', caret: 7 })).toBeNull();
    // The correction is gone, so there is nothing to put back.
    expect(undoRevert(revert, { text: 'th', caret: 3 })).toBeNull();
  });
});

/**
 * Drives `DraftSpeller` the way the composer does: every character is an
 * input event, a boundary character asks for a correction, and the answer is
 * applied to whatever the draft has become by then.
 */
class FakeComposer {
  draft: Draft = { text: '', caret: 0 };
  speller = new DraftSpeller();
  constructor(private readonly fixes: Record<string, string>) {}

  async type(chars: string): Promise<void> {
    for (const char of chars) {
      const { text, caret } = this.draft;
      this.draft = { text: text.slice(0, caret) + char + text.slice(caret), caret: caret + char.length };
      const kind = editKind({ inputType: char === '\n' ? 'insertLineBreak' : 'insertText', data: char === '\n' ? null : char });
      this.speller.afterEdit(kind, this.draft);
      if (kind === 'word-end') await this.correct();
    }
  }

  async correct(): Promise<void> {
    const fixed = await this.speller.autocorrect(this.draft, async (word) => this.fixes[word] ?? null, () => this.draft);
    if (fixed) this.draft = fixed;
  }

  backspace(): void {
    const { text, caret } = this.draft;
    this.draft = { text: text.slice(0, caret - 1) + text.slice(caret), caret: caret - 1 };
    const undone = this.speller.afterEdit('delete-backward', this.draft);
    if (undone) this.draft = undone;
  }
}

describe('autocorrect', () => {
  const fixes = { teh: 'the', disappointet: 'disappointed', gonna: 'goanna', ur: 'urn' };

  it('corrects the word a space finishes, and the next Backspace undoes it', async () => {
    const composer = new FakeComposer(fixes);
    await composer.type('teh');
    // Mid-word: nothing has been rewritten yet.
    expect(composer.draft.text).toBe('teh');
    await composer.type(' ');
    // The boundary character stays put, and the caret stays after it.
    expect(composer.draft).toEqual({ text: 'the ', caret: 4 });
    composer.backspace();
    expect(composer.draft).toEqual({ text: 'teh', caret: 3 });
  });

  it('works mid-line, keeping the caret relative to the text after it', async () => {
    const composer = new FakeComposer(fixes);
    composer.draft = { text: 'one  two', caret: 4 };
    await composer.type('disappointet ');
    expect(composer.draft).toEqual({ text: 'one disappointed  two', caret: 17 });
  });

  it('does not make an undone correction again in the same draft', async () => {
    const composer = new FakeComposer(fixes);
    await composer.type('teh ');
    composer.backspace();
    expect(composer.draft.text).toBe('teh');
    // Finishing the word again leaves it alone...
    await composer.type(' ');
    expect(composer.draft.text).toBe('teh ');
    // ...for the rest of the draft, not just once.
    await composer.type('teh ');
    expect(composer.draft.text).toBe('teh teh ');
  });

  it('forgets rejections with the draft', async () => {
    const composer = new FakeComposer(fixes);
    await composer.type('teh ');
    composer.backspace();
    composer.speller.reset();
    composer.draft = { text: '', caret: 0 };
    await composer.type('teh ');
    expect(composer.draft.text).toBe('the ');
  });

  it('is good for exactly one edit: a later Backspace is an ordinary one', async () => {
    const composer = new FakeComposer(fixes);
    await composer.type('teh x');
    composer.backspace();
    composer.backspace();
    expect(composer.draft.text).toBe('the');
  });

  it('leaves capitalised words, slang and non-prose alone', async () => {
    const composer = new FakeComposer({ ...fixes, Teh: 'The' });
    await composer.type('Teh gonna ur @teh ');
    expect(composer.draft.text).toBe('Teh gonna ur @teh ');
  });

  it('drops a correction whose word changed while the speller was answering', async () => {
    const speller = new DraftSpeller();
    let draft: Draft = { text: 'teh ', caret: 4 };
    const pending = speller.autocorrect(
      draft,
      async (word) => {
        // The user deleted the space before the answer came back.
        draft = { text: 'teh', caret: 3 };
        return fixes[word as keyof typeof fixes] ?? null;
      },
      () => draft,
    );
    expect(await pending).toBeNull();
  });

  it('keeps applying a correction the user has typed past, without arming the undo for it', async () => {
    const speller = new DraftSpeller();
    let draft: Draft = { text: 'teh ', caret: 4 };
    const fixed = await speller.autocorrect(
      draft,
      async () => {
        draft = { text: 'teh ca', caret: 6 };
        return 'the';
      },
      () => draft,
    );
    expect(fixed).toEqual({ text: 'the ca', caret: 6 });
    // Backspace now deletes the "a", which is not the undo.
    expect(speller.afterEdit('delete-backward', { text: 'the c', caret: 5 })).toBeNull();
  });

  it('drops a correction for a draft that was sent meanwhile', async () => {
    const speller = new DraftSpeller();
    const draft: Draft = { text: 'teh ', caret: 4 };
    const pending = speller.autocorrect(draft, async () => 'the', () => draft);
    speller.reset();
    expect(await pending).toBeNull();
  });
});

describe('VerdictCache', () => {
  it('asks only about words it does not know, once', async () => {
    const asked: string[][] = [];
    const cache = new VerdictCache(async (list) => {
      asked.push(list);
      return list.map((word) => word === 'teh');
    });
    expect(await cache.ask(['teh', 'the', 'teh'])).toBe(true);
    expect(cache.get('teh')).toBe(true);
    expect(cache.get('the')).toBe(false);
    expect(await cache.ask(['teh', 'the'])).toBe(false);
    expect(asked).toEqual([['teh', 'the']]);
  });

  it('does not ask twice while a question is in flight', async () => {
    let calls = 0;
    const cache = new VerdictCache(async (list) => {
      calls++;
      return list.map(() => false);
    });
    await Promise.all([cache.ask(['one']), cache.ask(['one'])]);
    expect(calls).toBe(1);
  });

  it('degrades to "not misspelled" when the speller fails', async () => {
    const cache = new VerdictCache(async () => {
      throw new Error('no speller');
    });
    await cache.ask(['teh']);
    expect(cache.get('teh')).toBe(false);
  });

  it('discards answers to questions asked before a clear', async () => {
    let release: (value: boolean[]) => void = () => {};
    const cache = new VerdictCache(() => new Promise((resolve) => (release = resolve)));
    const pending = cache.ask(['teh']);
    // "Add to dictionary" while the old verdict is still on its way.
    cache.clear();
    release([true]);
    expect(await pending).toBe(false);
    expect(cache.get('teh')).toBeUndefined();
  });

  it('starts over past its cap instead of growing without bound', async () => {
    const cache = new VerdictCache(async (list) => list.map(() => false), 3);
    await cache.ask(['a1', 'a2', 'a3']);
    await cache.ask(['b1']);
    expect(cache.get('a1')).toBeUndefined();
    expect(cache.get('b1')).toBe(false);
  });
});
