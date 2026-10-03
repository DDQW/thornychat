import { backend } from '../api';
import { VerdictCache } from '../spell';
import type { SpellStatus } from '../types';

/** The speller's verdicts and status, shared by every composer (the dictionary is global). */
class SpellingStore {
  /** `null` until the shell has answered. */
  status = $state.raw<SpellStatus | null>(null);
  /** Bumped whenever verdicts are learned or forgotten, so drafts re-scan. */
  version = $state(0);
  #cache = new VerdictCache((words) => backend.spellCheck(words));
  #statusAsked = false;

  verdict(word: string): boolean | undefined {
    return this.#cache.get(word);
  }

  /** Asks the speller about `words`; drafts re-scan once the answers are in. */
  request(words: readonly string[]): void {
    void this.#cache.ask(words).then((learned) => {
      if (learned) this.version++;
    });
  }

  /** Adds `word` to the Windows dictionary and forgets every verdict (any of them may flip). */
  async add(word: string): Promise<void> {
    await backend.spellAdd(word);
    this.#cache.clear();
    this.version++;
  }

  /** Fetches the status once; this also starts the speller, so the first keystroke doesn't wait for it. */
  loadStatus(): void {
    if (this.#statusAsked) return;
    this.#statusAsked = true;
    backend.spellStatus().then(
      (status) => (this.status = status),
      () => (this.status = { available: false, language: null }),
    );
  }
}

export const spelling = new SpellingStore();
