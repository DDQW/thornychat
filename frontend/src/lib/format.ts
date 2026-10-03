// Small presentation helpers shared across components. Pure, so testable.

/**
 * Best-effort display label for a user id when no display name is set. Strips
 * a leading `@` and the trailing `:server`, then an `irc_` bridge-ghost prefix
 * if one is present — matrix-appservice-irc puppets users as `@irc_<nick>:server`,
 * so a bridged sender with no Matrix display name would otherwise render as the
 * raw `@irc_alice:matrix.org` instead of just `alice`.
 *
 * Only ever the *last-resort* fallback, so a genuine user whose localpart
 * happens to start with `irc_` and who never set a display name has it stripped
 * too — accepted as a rare, cosmetic-only false positive (the real id is
 * untouched). The `irc_` strip is skipped when nothing would remain.
 */
export function friendlyUserId(userId: string): string {
  const withoutAt = userId.startsWith('@') ? userId.slice(1) : userId;
  const localpart = withoutAt.split(':')[0] ?? withoutAt;
  if (localpart.startsWith('irc_') && localpart.length > 4) return localpart.slice(4);
  return localpart;
}

/** One or two letters for an avatar placeholder. */
export function initialsOf(name: string): string {
  const cleaned = name.replace(/^[@#!]/, '').trim();
  if (!cleaned) return '?';
  const words = cleaned.split(/[\s._-]+/).filter(Boolean);
  const first = [...(words[0] ?? cleaned)][0] ?? '?';
  const second = words.length > 1 ? ([...(words[1] ?? '')][0] ?? '') : '';
  return (first + second).toUpperCase();
}

/** FNV-1a: a stable, dependency-free string hash. */
export function hashString(text: string): number {
  let hash = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0;
}

/**
 * The hue (0-359) a user's name is drawn in. The same id always gets the same
 * hue, on every device; lightness/saturation come from the theme so the result
 * stays legible on dark and light backgrounds alike.
 */
export function nameHue(userId: string): number {
  return hashString(userId) % 360;
}

export function formatTime(timestampMs: number): string {
  return new Date(timestampMs).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

export function formatDateTime(timestampMs: number): string {
  return new Date(timestampMs).toLocaleString([], {
    weekday: 'short',
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  });
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} bytes`;
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
}

/** "1.2K" style counts for tweet engagement numbers. */
export function compactNumber(value: number): string {
  return new Intl.NumberFormat('en', { notation: 'compact', maximumFractionDigits: 1 }).format(value);
}

/** Truncates to `limit` characters (not UTF-16 units) with an ellipsis. */
export function truncate(text: string, limit: number): string {
  const chars = [...text];
  return chars.length > limit ? `${chars.slice(0, limit).join('')}…` : text;
}

export function plural(count: number, one: string, many = `${one}s`): string {
  return `${count} ${count === 1 ? one : many}`;
}
