import { describe, expect, it } from 'vitest';
import { formatBytes, formatDay, friendlyUserId, hashString, initialsOf, nameHue, plural, truncate } from './format';

describe('friendlyUserId', () => {
  it('strips the @ and the server', () => {
    expect(friendlyUserId('@alice:matrix.org')).toBe('alice');
  });

  it('strips an irc bridge-ghost prefix', () => {
    expect(friendlyUserId('@irc_alice:matrix.org')).toBe('alice');
  });

  it('never yields an empty string for a bare irc_ prefix', () => {
    expect(friendlyUserId('@irc_:server')).toBe('irc_');
  });

  it('tolerates input without the usual shape', () => {
    expect(friendlyUserId('alice')).toBe('alice');
    expect(friendlyUserId('')).toBe('');
  });
});

describe('initialsOf', () => {
  it('takes one or two letters', () => {
    expect(initialsOf('alice')).toBe('A');
    expect(initialsOf('Alice Smith')).toBe('AS');
    expect(initialsOf('#general')).toBe('G');
    expect(initialsOf('')).toBe('?');
  });

  it('handles astral characters without splitting a surrogate pair', () => {
    expect(initialsOf('😀 party')).toBe('😀P');
  });
});

describe('nameHue', () => {
  it('is stable and within range', () => {
    expect(nameHue('@alice:b')).toBe(nameHue('@alice:b'));
    for (const id of ['@a:b', '@b:b', '@zzz:example.org', '']) {
      const hue = nameHue(id);
      expect(hue).toBeGreaterThanOrEqual(0);
      expect(hue).toBeLessThan(360);
    }
  });

  it('spreads different users across the wheel', () => {
    const hues = new Set(Array.from({ length: 40 }, (_, i) => nameHue(`@user${i}:server`)));
    expect(hues.size).toBeGreaterThan(20);
  });

  it('matches the reference FNV-1a values', () => {
    expect(hashString('')).toBe(0x811c9dc5);
    expect(hashString('a')).toBe(0xe40c292c);
  });
});

describe('small formatters', () => {
  it('formats byte counts', () => {
    expect(formatBytes(512)).toBe('512 bytes');
    expect(formatBytes(2048)).toBe('2.0 KB');
    expect(formatBytes(1_048_576)).toBe('1.0 MB');
  });

  it('truncates by character', () => {
    expect(truncate('abcdef', 3)).toBe('abc…');
    expect(truncate('abc', 3)).toBe('abc');
    expect(truncate('😀😀😀😀', 2)).toBe('😀😀…');
  });

  it('pluralizes', () => {
    expect(plural(1, 'file')).toBe('1 file');
    expect(plural(3, 'file')).toBe('3 files');
    expect(plural(2, 'person', 'people')).toBe('2 people');
  });
});

describe('formatDay', () => {
  // Local wall-clock times, so these hold in any time zone.
  const at = (y: number, m: number, d: number, h = 12, min = 0) => new Date(y, m - 1, d, h, min).getTime();
  const now = at(2026, 10, 25, 12);

  it('names today and yesterday by the local calendar day', () => {
    expect(formatDay(at(2026, 10, 25, 0, 30), now)).toBe('Today');
    expect(formatDay(at(2026, 10, 24, 23, 59), now)).toBe('Yesterday');
    expect(formatDay(at(2026, 10, 24, 0, 1), now)).toBe('Yesterday');
  });

  it('dates anything older, with the year only when it differs', () => {
    expect(formatDay(at(2026, 10, 23), now)).not.toMatch(/Today|Yesterday|2026/);
    expect(formatDay(at(2025, 12, 31), now)).toMatch(/2025/);
  });

  it('survives a day with a clock change in it', () => {
    // 2026-10-25 is when Central Europe falls back an hour.
    expect(formatDay(at(2026, 10, 24, 1), at(2026, 10, 25, 23))).toBe('Yesterday');
  });
});
