// Who is who: display names and avatars for user ids, from the open room's
// roster. Reads reactive store state, so calling these in a component keeps it
// up to date as the roster changes.

import type { ReplyPreview, RoomMember } from './bindings';
import { friendlyUserId } from './format';
import { tagColor, tagFor } from './members';
import { rooms } from './stores/rooms.svelte';
import { settings } from './stores/settings.svelte';

const lookups = new WeakMap<RoomMember[], Map<string, RoomMember>>();

/** The roster of `roomId` as a map, built once per roster update. */
export function memberMap(roomId: string | null): Map<string, RoomMember> | undefined {
  if (!roomId) return undefined;
  const members = rooms.members[roomId];
  if (!members) return undefined;
  let map = lookups.get(members);
  if (!map) {
    map = new Map(members.map((member) => [member.user_id, member]));
    lookups.set(members, map);
  }
  return map;
}

/**
 * The name to show for `userId`: the room display name, else whatever the
 * timeline item carried, else a tidied version of the id (bridged IRC users
 * lose their `irc_` prefix).
 */
export function nameOf(userId: string, roomId: string | null = rooms.selectedId, fallback?: string | null): string {
  const member = memberMap(roomId)?.get(userId);
  return member?.display_name || fallback || friendlyUserId(userId);
}

/**
 * Who wrote a quote the server won't hand over, when that is someone this
 * account ignores: the server hides their messages as if never sent, and the
 * reply mentions whoever it quotes. Null for every other quote.
 */
export function ignoredAuthor(reply: ReplyPreview): string | null {
  if (!reply.gone) return null;
  return reply.mentions.find((userId) => rooms.ignored.includes(userId)) ?? null;
}

export function avatarOf(userId: string, roomId: string | null = rooms.selectedId): string | null {
  return memberMap(roomId)?.get(userId)?.avatar_url ?? null;
}

/**
 * The colour of the team (MSC3949 power-level tag) `userId` is on in `roomId`,
 * as the members panel draws it, or null when the room gives their level no
 * coloured tag: then the name keeps its own per-user colour.
 */
export function teamColor(userId: string, roomId: string): string | null {
  const tags = rooms.powerTags[roomId];
  if (!tags?.length) return null;
  const level = memberMap(roomId)?.get(userId)?.power_level;
  if (level === undefined) return null;
  return tagColor(tagFor(level, tags)?.color ?? null, settings.value?.theme.dark ?? true);
}
