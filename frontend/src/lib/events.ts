// The single place `ClientEvent`s land. Each event is routed to the store that
// owns that slice of state; nothing else subscribes to the worker.

import { backend } from './api';
import type { ClientEvent, NotificationMode, TimelineDiff, TimelineItem } from './bindings';
import { withKey } from './record';
import { settle } from './requests';
import { emoji } from './stores/emoji.svelte';
import { previews } from './stores/previews.svelte';
import { rooms } from './stores/rooms.svelte';
import { security } from './stores/security.svelte';
import { session } from './stores/session.svelte';
import { timelines } from './stores/timelines.svelte';
import { ui } from './stores/ui.svelte';

export function handleEvent(event: ClientEvent): void {
  settle(event);

  switch (event.type) {
    // --- session / sync ---
    case 'SyncStateChanged':
      session.sync = event.data;
      break;
    case 'LoggedOut':
      session.handleLoggedOut();
      break;
    case 'SessionExpired':
      session.expired = true;
      break;

    // --- rooms ---
    case 'RoomListUpdated': {
      const first = !rooms.listLoaded;
      rooms.setList(event.data);
      if (first) void reopenLastRoom();
      break;
    }
    case 'RoomMembersUpdated':
      // Only the open room's roster is kept; one that lands after its room was
      // closed would never be released.
      if (event.data.room_id === rooms.selectedId) rooms.members = withKey(rooms.members, event.data.room_id, event.data.members);
      break;
    case 'DirectMessageReady':
    case 'RoomCreated':
      ui.dmSearchOpen = false;
      rooms.select(event.data.room_id);
      break;
    case 'PowerLevelTagsUpdated':
      if (event.data.room_id === rooms.selectedId) rooms.powerTags = withKey(rooms.powerTags, event.data.room_id, event.data.tags);
      break;
    case 'TypingUpdated':
      rooms.typing = withKey(rooms.typing, event.data.room_id, event.data.user_ids.length ? event.data.user_ids : undefined);
      break;
    case 'SpaceChildrenFetched':
      rooms.spaceChildren = withKey(rooms.spaceChildren, event.data.space_id, event.data.children);
      break;
    case 'CallStateUpdated': {
      const call = event.data;
      rooms.calls = withKey(rooms.calls, call.room_id, call.participants.length || call.joined ? call : undefined);
      break;
    }

    // --- timeline ---
    case 'TimelineDiffs':
      timelines.apply(event.data.room_id, event.data.diffs);
      emoji.harvest(itemsIn(event.data.diffs));
      break;
    case 'TimelineStartReached':
      timelines.markStartReached(event.data.room_id);
      break;
    case 'ReceiptsUpdated':
      // Receipts ride on the timeline items themselves (`read_by`).
      break;

    // --- emoji, media, previews ---
    case 'CustomEmojiPacksUpdated':
      emoji.packs = event.data;
      break;
    case 'UrlPreviewFetched':
      previews.ogFetched(event.data);
      break;
    case 'UrlPreviewFailed':
      previews.ogFailed(event.data.url);
      break;
    case 'MediaFetched':
    case 'MediaFileReady':
    case 'MediaFetchFailed':
      // Media goes through the `tcmedia` protocol, not events.
      break;

    // --- encryption ---
    case 'CrossSigningBootstrapNeedsFallback':
      security.crossSigning = { state: 'needs-fallback', url: event.data.url };
      break;
    case 'CrossSigningBootstrapDone':
      security.crossSigning = { state: 'idle' };
      break;
    case 'CrossSigningBootstrapFailed':
      security.crossSigning = { state: 'failed', reason: event.data.reason };
      break;
    case 'VerificationStateChanged':
      security.sas = event.data;
      break;
    case 'KeyBackupNeedsRecovery':
      security.needsRecovery = true;
      break;
    case 'RecoverySetupNeeded':
      security.setupNeeded = true;
      break;
    case 'RecoveryEnableProgress':
      security.recoveryStage = event.data;
      break;
    case 'RecoveryEnabled':
      security.recoveryStage = null;
      security.recoveryKeyToConfirm = event.data.recovery_key;
      break;
    case 'RecoveryEnableFailed':
      security.recoveryStage = null;
      security.recoveryError = event.data.reason;
      break;
    case 'KeyBackupRestored':
      security.restored = true;
      security.needsRecovery = false;
      break;
    case 'KeyBackupFailed':
      security.recoveryError = event.data.reason;
      break;

    // --- notifications, ignore list ---
    case 'Notification':
      // Shown as a toast by the shell.
      break;
    case 'RoomNotificationModeChanged':
      rooms.modes = withKey<NotificationMode>(rooms.modes, event.data.room_id, event.data.mode);
      break;
    case 'RoomNotificationModeCleared':
      rooms.modes = withKey<NotificationMode>(rooms.modes, event.data.room_id, undefined);
      break;
    case 'RoomNotificationModesUpdated':
      rooms.modes = Object.fromEntries(event.data);
      break;
    case 'DefaultNotificationModesUpdated':
      rooms.defaultModes = event.data;
      break;
    case 'IgnoredUsersUpdated':
      rooms.ignored = event.data;
      break;

    // --- correlated results are settled above ---
    case 'SearchResults':
    case 'UserSearchResults':
    case 'SpaceHierarchyFetched':
    case 'CommandSucceeded':
      break;
    case 'CommandFailed':
      // Whoever sent the command shows its error; an unobserved failure
      // (fire-and-forget commands carry no request id) never gets here.
      break;
  }
}

/** The items a batch of diffs introduces (what could contain a sticker to remember). */
function itemsIn(diffs: readonly TimelineDiff[]): TimelineItem[] {
  const items: TimelineItem[] = [];
  for (const diff of diffs) {
    switch (diff.type) {
      case 'Append':
      case 'Reset':
        items.push(...diff.data);
        break;
      case 'PushFront':
      case 'PushBack':
        items.push(diff.data);
        break;
      case 'Insert':
      case 'Set':
        items.push(diff.data.item);
        break;
      default:
        break;
    }
  }
  return items;
}

/** After the first room list: reopen whatever was on screen when the app last ran. */
async function reopenLastRoom(): Promise<void> {
  try {
    const remembered = (await backend.loadProfileFile('last_room'))?.trim();
    if (remembered && !rooms.selectedId && rooms.byId.has(remembered)) rooms.select(remembered);
  } catch {
    // Not remembering a room is not an error.
  }
}
