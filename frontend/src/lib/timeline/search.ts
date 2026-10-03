// In-room message search: a client-side filter over what is already loaded
// (the homeserver's search isn't used). A message matches when the query is a
// case-insensitive substring of its text, caption, file name or sender's name.

import type { TimelineItem } from '../bindings';

/** The searchable text of an item, lowercased. Dividers and system lines have none. */
function haystack(item: TimelineItem, senderName: string): string | null {
  const content = item.content;
  let text: string;
  switch (content.type) {
    case 'Text':
    case 'Emote':
      text = content.data;
      break;
    case 'Image':
      text = content.data.caption ?? '';
      break;
    case 'Sticker':
      text = content.data.body;
      break;
    case 'File':
    case 'Video':
      text = `${content.data.filename} ${content.data.caption ?? ''}`;
      break;
    default:
      return null;
  }
  return `${senderName} ${text}`.toLowerCase();
}

/** Whether `item` matches `query`. An empty query matches nothing (the caller shows everything instead). */
export function matchesQuery(item: TimelineItem, query: string, senderName: string): boolean {
  const needle = query.trim().toLowerCase();
  if (!needle) return false;
  return haystack(item, senderName)?.includes(needle) ?? false;
}
