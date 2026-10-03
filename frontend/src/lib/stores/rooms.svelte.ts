import type {
  CallState,
  NotificationMode,
  PowerLevelTag,
  RoomMember,
  RoomSummary,
} from '../bindings';
import { backend } from '../api';
import { fire } from '../requests';
import { timelines } from './timelines.svelte';

class RoomsStore {
  list = $state.raw<RoomSummary[]>([]);
  selectedId = $state<string | null>(null);
  /** Sidebar filter text. */
  filter = $state('');
  /** Space id → every child room/subspace id, joined or not. */
  spaceChildren = $state.raw<Record<string, string[]>>({});
  members = $state.raw<Record<string, RoomMember[]>>({});
  powerTags = $state.raw<Record<string, PowerLevelTag[]>>({});
  typing = $state.raw<Record<string, string[]>>({});
  calls = $state.raw<Record<string, CallState>>({});
  /** Per-room notification overrides; rooms absent follow the account default. */
  modes = $state.raw<Record<string, NotificationMode>>({});
  defaultModes = $state.raw<{ direct_messages: NotificationMode; group_chats: NotificationMode }>({
    direct_messages: 'AllMessages',
    group_chats: 'AllMessages',
  });
  ignored = $state.raw<string[]>([]);
  /** Rooms whose list has arrived at least once (so "no rooms" isn't shown while loading). */
  listLoaded = $state(false);

  selected = $derived(this.list.find((room) => room.room_id === this.selectedId) ?? null);
  byId = $derived(new Map(this.list.map((room) => [room.room_id, room])));

  setList(list: RoomSummary[]): void {
    this.list = list;
    this.listLoaded = true;
    // The open room disappeared (left or forgotten): close it.
    if (this.selectedId && !list.some((room) => room.room_id === this.selectedId)) {
      this.select(null);
    }
  }

  /** Opens `roomId` (and closes the previous one) in the worker, or just closes when null. */
  select(roomId: string | null): void {
    if (roomId === this.selectedId) return;
    const previous = this.selectedId;
    if (previous) {
      fire({ type: 'CloseRoom', data: { room_id: previous } });
      timelines.close(previous);
    }
    this.selectedId = roomId;
    if (roomId) {
      timelines.open(roomId);
      fire({ type: 'OpenRoom', data: { room_id: roomId } });
      void backend.saveProfileFile('last_room', roomId).catch(() => {});
    } else {
      void backend.saveProfileFile('last_room', '').catch(() => {});
    }
  }

  mode(roomId: string, isDm: boolean): NotificationMode {
    return this.modes[roomId] ?? (isDm ? this.defaultModes.direct_messages : this.defaultModes.group_chats);
  }

  reset(): void {
    this.list = [];
    this.selectedId = null;
    this.filter = '';
    this.spaceChildren = {};
    this.members = {};
    this.powerTags = {};
    this.typing = {};
    this.calls = {};
    this.modes = {};
    this.ignored = [];
    this.listLoaded = false;
  }
}

export const rooms = new RoomsStore();
