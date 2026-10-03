import type { TimelineDiff, TimelineItem } from '../bindings';
import { applyDiffs } from '../timeline/diff';

/**
 * Past this many items, a timeline that is quietly following the live edge is
 * reopened so it collapses back to the newest page (older history stays one
 * pagination away, exactly as if the room had just been opened). Days of live
 * appends otherwise grow the list, and the DOM with it, without bound.
 */
export const MAX_LIVE_ITEMS = 200;

/** One open room's message list and paging state. */
export class RoomTimeline {
  items = $state.raw<TimelineItem[]>([]);
  /** Back-pagination reached the room's first event. */
  startReached = $state(false);
  loadingOlder = $state(false);
  /** The first batch has arrived (until then the view shows a spinner). */
  loaded = $state(false);
  /** The user caught up since the divider was placed; hide it locally. */
  dividerSuppressed = $state(false);
  /** A `ShrinkTimeline` is in flight and its `Reset` hasn't landed yet. */
  shrinkPending = $state(false);

  apply(diffs: readonly TimelineDiff[]): void {
    this.items = applyDiffs(this.items, diffs);
    this.loaded = true;
    // A reset (including the one a shrink asks for) ends a pending shrink.
    if (diffs.some((diff) => diff.type === 'Reset')) this.shrinkPending = false;
  }
}

class TimelinesStore {
  #open = new Map<string, RoomTimeline>();
  /** Bumped when the set of open timelines changes, so readers re-evaluate. */
  version = $state(0);

  /** The timeline for an open room, or `undefined` if the room isn't open. */
  get(roomId: string | null): RoomTimeline | undefined {
    void this.version;
    return roomId ? this.#open.get(roomId) : undefined;
  }

  open(roomId: string): RoomTimeline {
    const timeline = new RoomTimeline();
    this.#open.set(roomId, timeline);
    this.version++;
    return timeline;
  }

  close(roomId: string): void {
    if (this.#open.delete(roomId)) this.version++;
  }

  /** Routes a batch to its room; a batch for a room that was closed meanwhile is dropped. */
  apply(roomId: string, diffs: readonly TimelineDiff[]): void {
    this.#open.get(roomId)?.apply(diffs);
  }

  markStartReached(roomId: string): void {
    const timeline = this.#open.get(roomId);
    if (timeline) timeline.startReached = true;
  }

  reset(): void {
    this.#open.clear();
    this.version++;
  }
}

export const timelines = new TimelinesStore();
