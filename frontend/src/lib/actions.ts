// What the user can do, as functions. Components call these; these build the
// `ClientCommand`s. Anything that can fail returns a promise that rejects with
// the worker's message, so the caller decides how to show it.

import type { CustomEmoji, NotificationMode, NotificationScope } from './bindings';
import { fire, request, requestResult } from './requests';
import type { Action } from './slash';
import { emoji } from './stores/emoji.svelte';
import { rooms } from './stores/rooms.svelte';
import { settings } from './stores/settings.svelte';
import { ui } from './stores/ui.svelte';

// --- messages ---

export interface SendOptions {
  mentionedUserIds?: string[];
  replyTo?: string | null;
  /** `m.emote` (`/me`). */
  emote?: boolean;
  /** `false` posts the body verbatim (`/plain`). */
  markdown?: boolean;
}

export function sendMessage(roomId: string, body: string, options: SendOptions = {}): Promise<void> {
  return request((request_id) => ({
    type: 'SendMessage',
    data: {
      room_id: roomId,
      body,
      mentioned_user_ids: options.mentionedUserIds ?? [],
      reply_to_event_id: options.replyTo ?? null,
      emote: options.emote ?? false,
      markdown: options.markdown ?? true,
      // `:shortcode:` pack emoji go out as images other clients can show.
      emoticons: emoji.usedIn(body),
      request_id,
    },
  }));
}

export function editMessage(roomId: string, eventId: string, newBody: string): Promise<void> {
  return request((request_id) => ({
    type: 'EditMessage',
    data: { room_id: roomId, event_id: eventId, new_body: newBody, emoticons: emoji.usedIn(newBody), request_id },
  }));
}

export function redactMessage(roomId: string, eventId: string, reason: string | null = null): Promise<void> {
  return request((request_id) => ({ type: 'RedactEvent', data: { room_id: roomId, event_id: eventId, reason, request_id } }));
}

export function toggleReaction(roomId: string, eventId: string, key: string): Promise<void> {
  return request((request_id) => ({ type: 'ToggleReaction', data: { room_id: roomId, event_id: eventId, key, request_id } }));
}

export function sendSticker(
  roomId: string,
  sticker: { url: string; body: string; width: number | null; height: number | null; mimetype?: string | null },
): Promise<void> {
  return request((request_id) => ({
    type: 'SendSticker',
    data: {
      room_id: roomId,
      url: sticker.url,
      body: sticker.body,
      width: sticker.width,
      height: sticker.height,
      mimetype: sticker.mimetype ?? null,
      request_id,
    },
  }));
}

/** A custom emoji as a sticker (pack images can be both). */
export function stickerOf(emoji: CustomEmoji) {
  return { url: emoji.mxc_url, body: emoji.shortcode, width: emoji.width, height: emoji.height };
}

export function setTyping(roomId: string, typing: boolean): void {
  if (settings.value?.privacy.send_typing_notifications) fire({ type: 'SetTyping', data: { room_id: roomId, typing } });
}

/** Marks the room read; the privacy setting picks the public or the private receipt. */
export function markRoomRead(roomId: string): void {
  fire({ type: 'MarkRoomRead', data: { room_id: roomId, public_receipt: settings.value?.privacy.send_read_receipts ?? false } });
}

export function paginateBackwards(roomId: string): Promise<void> {
  return request((request_id) => ({ type: 'PaginateBackwards', data: { room_id: roomId, request_id } }));
}

export function shrinkTimeline(roomId: string): void {
  fire({ type: 'ShrinkTimeline', data: { room_id: roomId } });
}

export function retrySend(roomId: string): void {
  fire({ type: 'RetrySend', data: { room_id: roomId } });
}

export function refreshEmojiPacks(roomId: string): void {
  fire({ type: 'RefreshEmojiPacks', data: { room_id: roomId } });
}

// --- rooms and people ---

export function openDirectMessage(userId: string): Promise<void> {
  return request((request_id) => ({
    type: 'OpenDirectMessage',
    data: { user_id: userId, encrypted: settings.value?.encryption.encrypt_direct_messages ?? false, request_id },
  }));
}

export function createRoomWith(userId: string): Promise<void> {
  return request((request_id) => ({
    type: 'CreateRoomWith',
    data: { user_id: userId, encrypted: settings.value?.encryption.encrypt_rooms ?? false, request_id },
  }));
}

export function createRoom(): Promise<void> {
  return request((request_id) => ({
    type: 'CreateRoom',
    data: { encrypted: settings.value?.encryption.encrypt_rooms ?? false, request_id },
  }));
}

export function joinRoom(roomIdOrAlias: string, via: string[] = []): Promise<void> {
  return request((request_id) => ({ type: 'JoinRoom', data: { room_id_or_alias: roomIdOrAlias, via, request_id } }));
}

export function knockRoom(roomIdOrAlias: string, via: string[] = []): Promise<void> {
  return request((request_id) => ({ type: 'KnockRoom', data: { room_id_or_alias: roomIdOrAlias, via, request_id } }));
}

export function leaveRoom(roomId: string): Promise<void> {
  return request((request_id) => ({ type: 'LeaveRoom', data: { room_id: roomId, request_id } }));
}

export function forgetRoom(roomId: string): Promise<void> {
  return request((request_id) => ({ type: 'ForgetRoom', data: { room_id: roomId, request_id } }));
}

export function renameRoom(roomId: string, name: string): Promise<void> {
  return request((request_id) => ({ type: 'SetRoomName', data: { room_id: roomId, name, request_id } }));
}

export function setRoomTopic(roomId: string, topic: string): Promise<void> {
  return request((request_id) => ({ type: 'SetRoomTopic', data: { room_id: roomId, topic, request_id } }));
}

export function inviteUser(roomId: string, userId: string): Promise<void> {
  return request((request_id) => ({ type: 'InviteUser', data: { room_id: roomId, user_id: userId, request_id } }));
}

export function ignoreUser(userId: string): Promise<void> {
  return request((request_id) => ({ type: 'IgnoreUser', data: { user_id: userId, request_id } }));
}

export function unignoreUser(userId: string): Promise<void> {
  return request((request_id) => ({ type: 'UnignoreUser', data: { user_id: userId, request_id } }));
}

export function setDisplayName(name: string): Promise<void> {
  return request((request_id) => ({ type: 'SetDisplayName', data: { name, request_id } }));
}

export function searchUsers(query: string) {
  return requestResult((request_id) => ({ type: 'SearchUsers', data: { query, request_id } }), 'UserSearchResults');
}

export function fetchSpaceHierarchy(spaceId: string, from: string | null) {
  return requestResult(
    (request_id) => ({ type: 'FetchSpaceHierarchy', data: { space_id: spaceId, from, request_id } }),
    'SpaceHierarchyFetched',
  );
}

// --- calls ---

export function joinCall(roomId: string): Promise<void> {
  return request((request_id) => ({ type: 'JoinCall', data: { room_id: roomId, request_id } }));
}

export function leaveCall(roomId: string): Promise<void> {
  return request((request_id) => ({ type: 'LeaveCall', data: { room_id: roomId, request_id } }));
}

// --- notifications ---

export function setRoomNotificationMode(roomId: string, mode: NotificationMode): Promise<void> {
  return request((request_id) => ({ type: 'SetRoomNotificationMode', data: { room_id: roomId, mode, request_id } }));
}

export function clearRoomNotificationMode(roomId: string): Promise<void> {
  return request((request_id) => ({ type: 'ClearRoomNotificationMode', data: { room_id: roomId, request_id } }));
}

export function setDefaultNotificationMode(scope: NotificationScope, mode: NotificationMode): Promise<void> {
  return request((request_id) => ({ type: 'SetDefaultNotificationMode', data: { scope, mode, request_id } }));
}

// --- slash commands ---

/**
 * Runs a parsed slash action against `roomId`. Resolves with a short
 * confirmation to show in the composer, or `null` when the action's own effect
 * (an opened DM, a left room) is its own confirmation.
 */
export async function runSlashAction(roomId: string, action: Action): Promise<string | null> {
  switch (action.type) {
    case 'join':
      await joinRoom(action.room);
      return `Joined ${action.room}`;
    case 'knock':
      await knockRoom(action.room);
      return `Asked to join ${action.room}`;
    case 'leave':
      await leaveRoom(roomId);
      return null;
    case 'invite':
      await inviteUser(roomId, action.user);
      return `Invited ${action.user}`;
    case 'dm':
      await openDirectMessage(action.user);
      return null;
    case 'kick':
      await request((request_id) => ({ type: 'KickUser', data: { room_id: roomId, user_id: action.user, reason: action.reason, request_id } }));
      return `Kicked ${action.user}`;
    case 'ban':
      await request((request_id) => ({ type: 'BanUser', data: { room_id: roomId, user_id: action.user, reason: action.reason, request_id } }));
      return `Banned ${action.user}`;
    case 'unban':
      await request((request_id) => ({ type: 'UnbanUser', data: { room_id: roomId, user_id: action.user, request_id } }));
      return `Unbanned ${action.user}`;
    case 'ignore':
      await ignoreUser(action.user);
      return `Ignoring ${action.user}`;
    case 'unignore':
      await unignoreUser(action.user);
      return `No longer ignoring ${action.user}`;
    case 'topic':
      await setRoomTopic(roomId, action.text);
      return 'Topic set';
    case 'nick':
      await setDisplayName(action.name);
      return `You are now ${action.name}`;
    case 'roomname':
      await renameRoom(roomId, action.name);
      return 'Room renamed';
  }
}

/** Posts the game-activity connector's emote into the room on screen, if any. */
export function postActivityEmote(body: string): void {
  const roomId = rooms.selectedId;
  if (!roomId) return;
  sendMessage(roomId, body, { emote: true, markdown: false }).catch((error: unknown) => ui.error(`Couldn't post your activity: ${String(error)}`));
}
