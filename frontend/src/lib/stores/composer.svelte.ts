import type { TimelineItem } from '../bindings';

/** What the composer is currently attached to: a message being replied to or edited. */
class ComposerContext {
  replyTo = $state.raw<TimelineItem | null>(null);
  editing = $state.raw<TimelineItem | null>(null);
  /** Text to place in the composer (set when an edit starts, a mention is clicked…). */
  inject = $state.raw<{ text: string; mention: { userId: string; displayName: string } | null; nonce: number } | null>(null);
  #nonce = 0;

  reply(item: TimelineItem): void {
    this.editing = null;
    this.replyTo = item;
  }

  edit(item: TimelineItem): void {
    this.replyTo = null;
    this.editing = item;
  }

  /** Asks the composer to insert text at the caret. */
  insert(text: string): void {
    this.inject = { text, mention: null, nonce: ++this.#nonce };
  }

  /** Inserts `@Name ` and remembers the mention, so it is attached when the message is sent. */
  mention(userId: string, displayName: string): void {
    this.inject = { text: `@${displayName} `, mention: { userId, displayName }, nonce: ++this.#nonce };
  }

  clear(): void {
    this.replyTo = null;
    this.editing = null;
  }
}

export const composer = new ComposerContext();
