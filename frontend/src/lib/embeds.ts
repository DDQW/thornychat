// Recognising links that deserve a richer card than a plain URL: video
// platforms (inline player), direct video files, tweets, Steam store pages.
// Matching is purely on the URL's shape — no network round-trip just to decide
// whether a play button belongs on a card. Ported from the iced build's
// `ui::video_player` URL parsers, rule for rule.

export type VideoPlatform = 'youtube' | 'vimeo' | 'dailymotion' | 'rumble' | 'kick' | 'file';

export interface EmbedVideo {
  platform: VideoPlatform;
  id: string;
  startSeconds: number;
  vimeoHash: string | null;
  sourceUrl: string;
}

export const PLATFORM_LABEL: Record<VideoPlatform, string> = {
  youtube: 'YouTube',
  vimeo: 'Vimeo',
  dailymotion: 'Dailymotion',
  rumble: 'Rumble',
  kick: 'Kick',
  file: 'Video',
};

/** Approximate brand colour for the card's edge strip. */
export const PLATFORM_ACCENT: Record<VideoPlatform, string> = {
  youtube: '#ff3333',
  vimeo: '#1ab7ea',
  dailymotion: '#4a8cff',
  rumble: '#85c742',
  kick: '#53fc18',
  file: '#8a8a8a',
};

/** Splits a URL into (lowercased host without a leading "www.", path, query), or null for non-http(s). */
function split(url: string): { host: string; segments: string[]; query: URLSearchParams } | null {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return null;
  return {
    host: parsed.hostname.toLowerCase().replace(/^www\./, ''),
    segments: parsed.pathname.split('/').filter(Boolean),
    query: parsed.searchParams,
  };
}

const validId = (id: string | undefined, maxLength: number): id is string =>
  !!id && id.length <= maxLength && /^[A-Za-z0-9_-]+$/.test(id);

/** `t=90`, `t=90s`, and `t=1h2m3s` all appear in shared links. */
export function parseTimestamp(value: string): number | null {
  if (value === '') return null;
  if (/^\d+$/.test(value)) return Number(value);
  let total = 0;
  let digits = '';
  for (const char of value) {
    if (char >= '0' && char <= '9') {
      digits += char;
    } else if (char === 'h' || char === 'm' || char === 's') {
      if (digits === '') return null;
      total += Number(digits) * (char === 'h' ? 3600 : char === 'm' ? 60 : 1);
      digits = '';
    } else {
      return null;
    }
  }
  return digits === '' ? total : null;
}

function video(platform: VideoPlatform, id: string, startSeconds: number, sourceUrl: string, vimeoHash: string | null = null): EmbedVideo {
  return { platform, id, startSeconds, vimeoHash, sourceUrl };
}

/**
 * The common YouTube shapes: `watch?v=`, `youtu.be/`, `/shorts/`, `/live/`,
 * `/embed/`, `/v/` — with an optional `t=`/`start=` timestamp.
 */
function youtube(url: string): EmbedVideo | null {
  const parts = split(url);
  if (!parts) return null;
  let id: string | undefined;
  if (parts.host === 'youtu.be') {
    id = parts.segments[0];
  } else if (['youtube.com', 'm.youtube.com', 'music.youtube.com', 'youtube-nocookie.com'].includes(parts.host)) {
    const [first, second] = parts.segments;
    if (first === 'watch') id = parts.query.get('v') ?? undefined;
    else if (first === 'shorts' || first === 'live' || first === 'embed' || first === 'v') id = second;
  }
  if (!validId(id, 20)) return null;
  const start = parts.query.get('t') ?? parts.query.get('start');
  return video('youtube', id, (start && parseTimestamp(start)) || 0, url);
}

/**
 * `vimeo.com/{id}` (optionally `/{hash}` for unlisted videos) and
 * `player.vimeo.com/video/{id}` (`?h=` instead of a path hash). Channel/group/
 * showcase URLs are rejected naturally since their first segment isn't numeric.
 */
function vimeo(url: string): EmbedVideo | null {
  const parts = split(url);
  if (!parts) return null;
  let id: string | undefined;
  let hash: string | null | undefined;
  if (parts.host === 'vimeo.com') {
    [id, hash] = parts.segments;
  } else if (parts.host === 'player.vimeo.com') {
    if (parts.segments[0] !== 'video') return null;
    id = parts.segments[1];
    hash = parts.query.get('h');
  } else {
    return null;
  }
  if (!id || !/^\d+$/.test(id)) return null;
  return video('vimeo', id, 0, url, hash && /^[A-Za-z0-9]+$/.test(hash) ? hash : null);
}

/** `dailymotion.com/video/{id}[_slug]` and the `dai.ly/{id}` short link. */
function dailymotion(url: string): EmbedVideo | null {
  const parts = split(url);
  if (!parts) return null;
  let raw: string | undefined;
  if (parts.host === 'dailymotion.com' || parts.host === 'm.dailymotion.com') {
    if (parts.segments[0] === 'video') raw = parts.segments[1];
  } else if (parts.host === 'dai.ly') {
    raw = parts.segments[0];
  }
  const id = raw?.split('_')[0];
  if (!validId(id, 20)) return null;
  const start = Number(parts.query.get('start'));
  return video('dailymotion', id, Number.isInteger(start) && start > 0 ? start : 0, url);
}

/**
 * A Rumble watch page is exactly one path segment ending in `.html`
 * (`v70bqqu-my-title.html`); the id is the part before the first `-`.
 * Already-embed links (`rumble.com/embed/{id}/`) are also recognised.
 */
function rumble(url: string): EmbedVideo | null {
  const parts = split(url);
  if (!parts || parts.host !== 'rumble.com') return null;
  const [first, second, third] = parts.segments;
  let id: string | undefined;
  if (first === 'embed') {
    id = second;
  } else if (first && second === undefined && first.endsWith('.html')) {
    id = first.slice(0, -'.html'.length).split('-')[0];
  }
  void third;
  return validId(id, 24) ? video('rumble', id, 0, url) : null;
}

const KICK_RESERVED = new Set([
  'categories', 'browse', 'search', 'following', 'messages', 'dashboard', 'settings', 'subscriptions',
  'wallet', 'moderator', 'explore', 'discover', 'signup', 'login', 'terms', 'privacy', 'about', 'contact', 'app',
]);

/**
 * Live-channel pages only: `kick.com/{channel}` with no further path segments.
 * Kick has no documented way to embed a VOD or clip by id, so those links fall
 * through to the regular preview instead of a misleading player.
 */
function kick(url: string): EmbedVideo | null {
  const parts = split(url);
  if (!parts || parts.host !== 'kick.com') return null;
  const [channel, extra] = parts.segments;
  if (extra !== undefined || !channel || KICK_RESERVED.has(channel) || !validId(channel, 25)) return null;
  return video('kick', channel, 0, url);
}

/**
 * A direct link to a video *file*, matched on the extension only. Restricted to
 * containers WebView2's Chromium decodes out of the box: `.mov`/`.mkv`/`.avi`
 * would silently fail to play instead of degrading to a normal link.
 */
function directFile(url: string): EmbedVideo | null {
  const parts = split(url);
  if (!parts) return null;
  const name = parts.segments.at(-1);
  const extension = name?.includes('.') ? name.split('.').pop()?.toLowerCase() : undefined;
  return extension && ['mp4', 'm4v', 'webm', 'ogv'].includes(extension) ? video('file', '', 0, url) : null;
}

/** Tries each platform's recogniser in turn. */
export function videoIn(url: string): EmbedVideo | null {
  return youtube(url) ?? vimeo(url) ?? dailymotion(url) ?? rumble(url) ?? kick(url) ?? directFile(url);
}

/** The iframe `src` (or media URL, for a file) that plays the video. */
export function embedUrl(video: EmbedVideo): string {
  switch (video.platform) {
    case 'youtube': {
      // youtube-nocookie.com is YouTube's own reduced-tracking embed host.
      // `rel=0` keeps end-screen suggestions to the same channel (the ceiling
      // YouTube allows); `modestbranding`/`iv_load_policy=3` trim branding and
      // annotations; `playsinline` keeps playback inline. Deliberately no
      // `fs=0`, which would remove the fullscreen button.
      const start = video.startSeconds > 0 ? `&start=${video.startSeconds}` : '';
      return `https://www.youtube-nocookie.com/embed/${video.id}?autoplay=1&rel=0&modestbranding=1&iv_load_policy=3&playsinline=1${start}`;
    }
    case 'vimeo': {
      // `title/byline/portrait=0` drop the overlay chrome; `dnt=1` opts out of
      // Vimeo's tracking cookies. `#t=` is a fragment, not a query parameter,
      // and must come last.
      const hash = video.vimeoHash ? `&h=${video.vimeoHash}` : '';
      const start = video.startSeconds > 0 ? `#t=${video.startSeconds}s` : '';
      return `https://player.vimeo.com/video/${video.id}?autoplay=1&title=0&byline=0&portrait=0&dnt=1${hash}${start}`;
    }
    case 'dailymotion': {
      // The sharing/end-screen/queue params drop the share button and the
      // "up next" grid; `geo.dailymotion.com/player.html?video=` needs no
      // partner player id.
      const start = video.startSeconds > 0 ? `&start=${video.startSeconds}` : '';
      return `https://geo.dailymotion.com/player.html?video=${video.id}&autoplay=1&sharing-enable=false&endscreen-enable=false&queue-enable=false&ui-logo=false${start}`;
    }
    case 'rumble':
      return `https://rumble.com/embed/${video.id}/?autoplay=2`;
    case 'kick':
      return `https://player.kick.com/${video.id}?autoplay=true`;
    case 'file':
      return video.sourceUrl;
  }
}

/** Whether the player is an iframe (as opposed to a plain `<video>` tag). */
export function isIframePlayer(video: EmbedVideo): boolean {
  return video.platform !== 'file';
}

/** The host an iframe player loads from; the CSP's `frame-src` must list each. */
export const IFRAME_HOSTS = [
  'https://www.youtube-nocookie.com',
  'https://player.vimeo.com',
  'https://geo.dailymotion.com',
  'https://rumble.com',
  'https://player.kick.com',
];

export function videoMimeFor(filename: string): string {
  switch (filename.split('.').pop()?.toLowerCase()) {
    case 'webm':
      return 'video/webm';
    case 'ogv':
    case 'ogg':
      return 'video/ogg';
    case 'mov':
      return 'video/quicktime';
    case 'mkv':
      return 'video/x-matroska';
    case 'm4v':
      return 'video/x-m4v';
    default:
      return 'video/mp4';
  }
}

/** URLs in message text, trailing punctuation trimmed. */
export function extractLinks(text: string, limit = 3): string[] {
  const found: string[] = [];
  for (const match of text.matchAll(/https?:\/\/[^\s<>"']+/gi)) {
    let url = match[0].replace(/[.,;:!?)\]}]+$/, '');
    // Unbalanced trailing ")" belongs to the sentence, a balanced one to the URL.
    if (match[0].endsWith(')') && (url.match(/\(/g)?.length ?? 0) > (url.match(/\)/g)?.length ?? 0)) url += ')';
    if (!found.includes(url)) found.push(url);
    if (found.length >= limit) break;
  }
  return found;
}

export function isTweetLink(url: string): boolean {
  const parts = split(url);
  if (!parts) return false;
  const tweetHosts = ['x.com', 'twitter.com', 'mobile.twitter.com', 'mobile.x.com', 'fxtwitter.com', 'vxtwitter.com', 'fixupx.com', 'xcancel.com'];
  const status = parts.segments.indexOf('status');
  return tweetHosts.includes(parts.host) && status >= 0 && /^\d+/.test(parts.segments[status + 1] ?? '');
}

export function isSteamLink(url: string): boolean {
  const parts = split(url);
  if (!parts) return false;
  if (parts.host === 'store.steampowered.com') {
    const app = parts.segments.indexOf('app');
    return app >= 0 && /^\d+$/.test(parts.segments[app + 1] ?? '');
  }
  return parts.host === 's.team' && parts.segments[0] === 'a' && /^\d+$/.test(parts.segments[1] ?? '');
}
