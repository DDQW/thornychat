/**
 * At most one inline video plays at a time; starting another stops the first.
 * An id names one player in one message (the same link posted twice is two
 * players), and a player stops when it leaves the page: otherwise its message
 * coming back — older history loading in, the room reopened — drew it playing
 * again, and its embed autostarted.
 */
class PlayerStore {
  current = $state<string | null>(null);

  play(id: string): void {
    this.current = id;
  }

  stop(id: string): void {
    if (this.current === id) this.current = null;
  }
}

export const player = new PlayerStore();
