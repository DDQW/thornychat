import { describe, expect, it } from 'vitest';
import { embedUrl, extractLinks, isSteamLink, isTweetLink, parseTimestamp, videoIn } from './embeds';

describe('timestamps', () => {
  it('reads seconds, suffixed seconds and h/m/s', () => {
    expect(parseTimestamp('90')).toBe(90);
    expect(parseTimestamp('90s')).toBe(90);
    expect(parseTimestamp('1h2m3s')).toBe(3723);
    expect(parseTimestamp('2m')).toBe(120);
  });
  it('rejects junk', () => {
    for (const bad of ['', 'abc', '1x', 's', '1h2', '-5']) expect(parseTimestamp(bad), bad).toBeNull();
  });
});

describe('YouTube', () => {
  it('recognises the common URL shapes', () => {
    for (const url of [
      'https://www.youtube.com/watch?v=dQw4w9WgXcQ',
      'https://youtu.be/dQw4w9WgXcQ',
      'https://m.youtube.com/watch?v=dQw4w9WgXcQ&feature=share',
      'https://www.youtube.com/shorts/dQw4w9WgXcQ',
      'https://www.youtube.com/live/dQw4w9WgXcQ',
      'https://www.youtube.com/embed/dQw4w9WgXcQ',
      'https://music.youtube.com/watch?v=dQw4w9WgXcQ',
    ]) {
      expect(videoIn(url)?.id, url).toBe('dQw4w9WgXcQ');
    }
  });

  it('reads a start time from t= or start=', () => {
    expect(videoIn('https://youtu.be/dQw4w9WgXcQ?t=1m30s')?.startSeconds).toBe(90);
    expect(videoIn('https://www.youtube.com/watch?v=dQw4w9WgXcQ&start=42')?.startSeconds).toBe(42);
    expect(videoIn('https://youtu.be/dQw4w9WgXcQ')?.startSeconds).toBe(0);
  });

  it('ignores pages that are not a single video', () => {
    for (const url of ['https://www.youtube.com/', 'https://www.youtube.com/@channel', 'https://www.youtube.com/watch', 'https://www.youtube.com/results?search_query=x']) {
      expect(videoIn(url), url).toBeNull();
    }
  });

  it('builds a nocookie embed', () => {
    const url = embedUrl(videoIn('https://youtu.be/dQw4w9WgXcQ?t=5')!);
    expect(url.startsWith('https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?')).toBe(true);
    expect(url).toContain('start=5');
  });
});

describe('other platforms', () => {
  it('Vimeo: id, path hash, and player-link hash', () => {
    expect(videoIn('https://vimeo.com/123456789')).toMatchObject({ platform: 'vimeo', id: '123456789', vimeoHash: null });
    expect(videoIn('https://vimeo.com/123456789/abcdef0123')?.vimeoHash).toBe('abcdef0123');
    expect(videoIn('https://player.vimeo.com/video/123456789?h=abc123')?.vimeoHash).toBe('abc123');
    expect(videoIn('https://vimeo.com/channels/staffpicks')).toBeNull();
    expect(videoIn('https://vimeo.com/123456789/not valid hash!')?.vimeoHash).toBeNull();
  });

  it('Dailymotion: strips the title slug and supports the short link', () => {
    expect(videoIn('https://www.dailymotion.com/video/x8abcd_some-title')?.id).toBe('x8abcd');
    expect(videoIn('https://dai.ly/x8abcd')?.id).toBe('x8abcd');
    expect(videoIn('https://www.dailymotion.com/playlist/x6hqr0')).toBeNull();
  });

  it('Rumble: single .html watch pages and embed links only', () => {
    expect(videoIn('https://rumble.com/v70bqqu-my-title.html')?.id).toBe('v70bqqu');
    expect(videoIn('https://rumble.com/embed/v70bqqu/')?.id).toBe('v70bqqu');
    expect(videoIn('https://rumble.com/c/SomeChannel')).toBeNull();
    expect(videoIn('https://rumble.com/category/news')).toBeNull();
  });

  it('Kick: live channels only', () => {
    expect(videoIn('https://kick.com/somechannel')?.id).toBe('somechannel');
    expect(videoIn('https://kick.com/somechannel/videos/abc')).toBeNull();
    expect(videoIn('https://kick.com/categories')).toBeNull();
    expect(videoIn('https://kick.com/')).toBeNull();
  });

  it('direct files: only containers Chromium plays', () => {
    expect(videoIn('https://cdn.example.com/clip.mp4')?.platform).toBe('file');
    expect(videoIn('https://cdn.example.com/clip.WEBM?token=1')?.platform).toBe('file');
    expect(videoIn('https://cdn.example.com/clip.mkv')).toBeNull();
    expect(videoIn('https://cdn.example.com/page')).toBeNull();
  });

  it('refuses anything that is not http(s)', () => {
    for (const url of ['javascript:alert(1)', 'file:///C:/clip.mp4', 'ftp://x/clip.mp4', 'not a url', '']) {
      expect(videoIn(url), url).toBeNull();
    }
  });
});

describe('link extraction and classification', () => {
  it('finds links and trims trailing sentence punctuation', () => {
    expect(extractLinks('see https://a.example/x, and (https://b.example/y).')).toEqual(['https://a.example/x', 'https://b.example/y']);
  });

  it('keeps a balanced closing paren that belongs to the URL', () => {
    expect(extractLinks('https://en.wikipedia.org/wiki/Foo_(bar)')).toEqual(['https://en.wikipedia.org/wiki/Foo_(bar)']);
  });

  it('dedupes and honours the limit', () => {
    expect(extractLinks('https://a.test https://a.test https://b.test https://c.test https://d.test')).toEqual(['https://a.test', 'https://b.test', 'https://c.test']);
  });

  it('recognises tweet and Steam links', () => {
    expect(isTweetLink('https://x.com/user/status/1234567890')).toBe(true);
    expect(isTweetLink('https://twitter.com/user/status/1234567890?s=20')).toBe(true);
    expect(isTweetLink('https://x.com/user')).toBe(false);
    expect(isTweetLink('https://example.com/user/status/123')).toBe(false);
    expect(isSteamLink('https://store.steampowered.com/app/220/HalfLife_2/')).toBe(true);
    expect(isSteamLink('https://s.team/a/220')).toBe(true);
    expect(isSteamLink('https://store.steampowered.com/')).toBe(false);
  });
});
