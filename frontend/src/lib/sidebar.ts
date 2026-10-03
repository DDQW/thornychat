// How the room list is organised: spaces with their joined rooms nested under
// them, then direct messages, then the remaining rooms. Pure, so it can be
// tested without a component.

import type { RoomSummary } from './bindings';

export interface SpaceNode {
  space: RoomSummary;
  /** Joined rooms directly under this space. */
  rooms: RoomSummary[];
  /** Joined sub-spaces, each with their own children. */
  spaces: SpaceNode[];
}

export interface SidebarModel {
  spaces: SpaceNode[];
  directMessages: RoomSummary[];
  /** Rooms that sit under no joined space. */
  rooms: RoomSummary[];
  /** True when a filter is active (the view then shows flat lists). */
  filtered: boolean;
}

function matches(room: RoomSummary, needle: string): boolean {
  return room.name.toLowerCase().includes(needle);
}

/**
 * Builds the sidebar from the flat room list. `spaceChildren` maps a space id
 * to every child id (joined or not); only joined children are shown. The
 * input order (the server's, most recent activity first) is preserved.
 *
 * With a filter, nesting is dropped: every matching room appears once in its
 * natural section, so a match is never hidden inside a collapsed space.
 */
export function buildSidebar(
  list: readonly RoomSummary[],
  spaceChildren: Readonly<Record<string, readonly string[]>>,
  filter: string,
): SidebarModel {
  const needle = filter.trim().toLowerCase();

  if (needle) {
    const hits = list.filter((room) => matches(room, needle));
    return {
      spaces: hits.filter((room) => room.is_space).map((space) => ({ space, rooms: [], spaces: [] })),
      directMessages: hits.filter((room) => room.is_dm && !room.is_space),
      rooms: hits.filter((room) => !room.is_dm && !room.is_space),
      filtered: true,
    };
  }

  const byId = new Map(list.map((room) => [room.room_id, room]));
  const order = new Map(list.map((room, index) => [room.room_id, index]));
  const byOrder = (a: RoomSummary, b: RoomSummary) => (order.get(a.room_id) ?? 0) - (order.get(b.room_id) ?? 0);
  /** Every room or space already shown somewhere in the tree; a room appears once, under its first space. */
  const placed = new Set<string>();

  const node = (space: RoomSummary, ancestors: Set<string>): SpaceNode => {
    const children: RoomSummary[] = [];
    const subspaces: SpaceNode[] = [];
    for (const childId of spaceChildren[space.room_id] ?? []) {
      const child = byId.get(childId);
      if (!child || ancestors.has(childId) || placed.has(childId)) continue;
      placed.add(childId);
      if (child.is_space) subspaces.push(node(child, new Set([...ancestors, childId])));
      else children.push(child);
    }
    // Keep the server's ordering within each group, whatever order the
    // space's own child list happens to use.
    children.sort(byOrder);
    subspaces.sort((a, b) => byOrder(a.space, b.space));
    return { space, rooms: children, spaces: subspaces };
  };

  const allSpaces = list.filter((room) => room.is_space);
  const referenced = new Set(
    allSpaces.flatMap((space) => (spaceChildren[space.room_id] ?? []).filter((id) => id !== space.room_id)),
  );
  const spaces: SpaceNode[] = [];
  const place = (space: RoomSummary) => {
    placed.add(space.room_id);
    spaces.push(node(space, new Set([space.room_id])));
  };
  // Spaces nobody contains are the roots. Anything left over is only reachable
  // through a cycle; claim it in list order so it still shows up somewhere.
  for (const space of allSpaces) if (!referenced.has(space.room_id)) place(space);
  for (const space of allSpaces) if (!placed.has(space.room_id)) place(space);

  return {
    spaces,
    directMessages: list.filter((room) => room.is_dm && !room.is_space && !placed.has(room.room_id)),
    rooms: list.filter((room) => !room.is_dm && !room.is_space && !placed.has(room.room_id)),
    filtered: false,
  };
}

/** Total unread across a space's whole subtree (for the collapsed header badge). */
export function unreadIn(node: SpaceNode): number {
  return node.rooms.reduce((sum, room) => sum + room.unread_count, 0) + node.spaces.reduce((sum, child) => sum + unreadIn(child), 0);
}
