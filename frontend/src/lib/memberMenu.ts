// The right-click menu for a person, shared by the roster, the sender name in
// the timeline, and avatars.

import { createRoomWith, ignoreUser, openDirectMessage, unignoreUser } from './actions';
import { composer } from './stores/composer.svelte';
import { rooms } from './stores/rooms.svelte';
import { session } from './stores/session.svelte';
import { ui, type MenuItem } from './stores/ui.svelte';

export function memberMenu(userId: string, displayName: string): MenuItem[] {
  const isMe = userId === session.info?.user_id;
  const ignored = rooms.ignored.includes(userId);
  const report = (what: string) => (error: unknown) => ui.error(`Couldn't ${what}: ${String(error)}`);

  const items: MenuItem[] = [{ label: 'Mention', onselect: () => composer.mention(userId, displayName) }];
  if (!isMe) {
    items.push(
      { label: 'Send message', onselect: () => openDirectMessage(userId).catch(report('open the conversation')) },
      { label: 'New room with them', onselect: () => createRoomWith(userId).catch(report('create the room')) },
    );
  }
  items.push({
    label: 'Copy user ID',
    separator: true,
    onselect: () => void navigator.clipboard.writeText(userId).then(() => ui.toast('User ID copied', 'success', 2000)),
  });
  if (!isMe) {
    items.push(
      ignored
        ? { label: 'Stop ignoring', onselect: () => unignoreUser(userId).catch(report('stop ignoring')) }
        : { label: 'Ignore', danger: true, onselect: () => ignoreUser(userId).catch(report('ignore')) },
    );
  }
  return items;
}
