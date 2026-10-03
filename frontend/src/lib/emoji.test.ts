import { describe, expect, it } from 'vitest';
import { buildEmojiData, searchEmoji, twemojiCodepoints } from './emoji';

describe('twemojiCodepoints', () => {
  it('lowercases hex codepoints joined by dashes', () => {
    expect(twemojiCodepoints('😀')).toBe('1f600');
    expect(twemojiCodepoints('🇩🇪')).toBe('1f1e9-1f1ea');
  });

  it('strips the variation selector when there is no joiner', () => {
    expect(twemojiCodepoints('❤️')).toBe('2764');
    expect(twemojiCodepoints('#️⃣')).toBe('23-20e3');
  });

  it('keeps the variation selector inside a ZWJ sequence', () => {
    // Verified against the CDN: the stripped form 404s.
    expect(twemojiCodepoints('🏃‍♂️')).toBe('1f3c3-200d-2642-fe0f');
  });
});

describe('buildEmojiData', () => {
  const raw = [
    { group: 1, hexcode: '1F44B', label: 'waving hand', order: 2, tags: ['hello'], unicode: '👋', skins: [{ hexcode: '1F44B-1F3FB', label: 'waving hand: light skin tone', unicode: '👋🏻' }] },
    { group: 0, hexcode: '1F600', label: 'grinning face', order: 1, tags: ['smile'], unicode: '😀' },
    { group: 2, hexcode: '1F3FB', label: 'light skin tone', order: 3, unicode: '🏻' },
    { hexcode: '1F1E6', label: 'regional indicator A', unicode: '🇦' },
  ];

  it('groups by category in dataset order and drops components and ungrouped entries', () => {
    const data = buildEmojiData(raw, {});
    expect(data.categories.map((c) => c.key)).toEqual(['smileys-emotion', 'people-body']);
    expect(data.all.map((e) => e.label)).toEqual(['grinning face', 'waving hand']);
  });

  it('applies the light skin tone where variants exist', () => {
    const data = buildEmojiData(raw, {});
    expect(data.all.find((e) => e.label === 'waving hand')?.char).toBe('👋🏻');
    expect(data.all.find((e) => e.label === 'grinning face')?.char).toBe('😀');
  });

  it('indexes shortcodes, first claimant wins', () => {
    const data = buildEmojiData(raw, { '1F600': ['grinning_face', 'grin'], '1F44B': 'wave' });
    expect(data.shortcodes.get('grin')?.char).toBe('😀');
    expect(data.shortcodes.get('wave')?.char).toBe('👋🏻');
    expect(data.shortcodes.get('nope')).toBeUndefined();
  });
});

describe('searchEmoji', () => {
  const data = buildEmojiData(
    [
      { group: 0, hexcode: '1', label: 'smiling face', order: 1, tags: ['happy'], unicode: '😊' },
      { group: 0, hexcode: '2', label: 'sad face', order: 2, tags: ['smile-not'], unicode: '😢' },
      { group: 0, hexcode: '3', label: 'party', order: 3, tags: ['celebrate'], unicode: '🥳' },
    ],
    {},
  );

  it('ranks label matches before tag matches', () => {
    expect(searchEmoji(data.all, 'smil').map((e) => e.char)).toEqual(['😊', '😢']);
  });

  it('is case-insensitive and ignores blank queries', () => {
    expect(searchEmoji(data.all, 'PARTY').map((e) => e.char)).toEqual(['🥳']);
    expect(searchEmoji(data.all, '   ')).toEqual([]);
  });
});
