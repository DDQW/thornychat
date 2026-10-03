/** At most one inline video plays at a time; starting another stops the first. */
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
