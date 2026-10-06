import { describe, expect, it } from 'vitest';
import { enrichBody, enrichHtml, escapeHtml, plainTextToHtml } from './richtext';

const options = { twemojiUrl: (codepoints: string) => `http://emoji.test/${codepoints}.svg` };

describe('escapeHtml', () => {
  it('escapes markup characters', () => {
    expect(escapeHtml('<b>"a" & b</b>')).toBe('&lt;b&gt;&quot;a&quot; &amp; b&lt;/b&gt;');
  });
});

describe('plainTextToHtml', () => {
  it('never lets plain text become markup', () => {
    const out = plainTextToHtml('<img src=x onerror=alert(1)> <script>alert(2)</script>', options);
    expect(out).not.toContain('<img src=x');
    expect(out).not.toContain('<script');
    expect(out).toContain('&lt;script&gt;');
  });

  it('links bare URLs and trims trailing punctuation', () => {
    const out = plainTextToHtml('see https://example.com/a?b=1, ok.', options);
    expect(out).toContain('<a href="https://example.com/a?b=1" rel="noopener noreferrer" data-external="">https://example.com/a?b=1</a>, ok.');
  });

  it('keeps a balanced closing paren inside the URL', () => {
    const out = plainTextToHtml('(https://en.wikipedia.org/wiki/Foo_(bar))', options);
    expect(out).toContain('href="https://en.wikipedia.org/wiki/Foo_(bar)"');
    expect(out.endsWith(')')).toBe(true);
  });

  it('replaces emoji with Twemoji images that keep the glyph as alt text', () => {
    const out = plainTextToHtml('hi 😀!', options);
    expect(out).toContain('src="http://emoji.test/1f600.svg"');
    expect(out).toContain('alt="😀"');
    expect(out).toContain('class="twemoji"');
  });

  it('treats ZWJ sequences, flags, keycaps and skin tones as single emoji', () => {
    const count = (text: string) => (plainTextToHtml(text, options).match(/<img/g) ?? []).length;
    expect(count('👨‍👩‍👧')).toBe(1);
    expect(count('🇩🇪')).toBe(1);
    expect(count('1️⃣')).toBe(1);
    expect(count('👋🏽')).toBe(1);
    expect(count('😀😀')).toBe(2);
    expect(count('plain 123 #1 *')).toBe(0);
  });

  it('uses the right Twemoji filename for ZWJ sequences', () => {
    expect(plainTextToHtml('🏃‍♂️', options)).toContain('1f3c3-200d-2642-fe0f.svg');
  });
});

describe('enrichHtml', () => {
  it('leaves code, pre and existing links alone', () => {
    const html = '<pre><code>https://x.test 😀</code></pre><a href="https://a.test">https://a.test 😀</a>';
    const out = enrichHtml(html, options);
    expect(out).toBe(html);
  });

  it('enriches text next to formatting elements', () => {
    const out = enrichHtml('<p><b>bold</b> 😀 https://x.test</p>', options);
    expect(out).toContain('<b>bold</b>');
    expect(out).toContain('<img');
    expect(out).toContain('href="https://x.test"');
  });

  it('returns input without tokens unchanged', () => {
    const html = '<p>nothing special here</p>';
    expect(enrichHtml(html, options)).toBe(html);
  });
});

describe('custom emoji shortcodes', () => {
  const pack: Record<string, string> = { scratch: 'mxc-scratch', notsure: 'mxc-notsure' };
  const withPack = {
    ...options,
    customEmoji: (code: string) => {
      const url = pack[code.toLowerCase()];
      return url ? { url: `http://media.test/${url}`, shortcode: code === 'notsure' ? 'NotSure' : code } : null;
    },
  };

  it('turns a pack shortcode into its image, keeping the shortcode as alt text', () => {
    const out = plainTextToHtml('we could all play a mud :scratch:', withPack);
    expect(out).toBe('we could all play a mud <img class="emoticon" src="http://media.test/mxc-scratch" alt=":scratch:" title=":scratch:" draggable="false">');
  });

  it('leaves unknown shortcodes, times and code alone', () => {
    expect(plainTextToHtml('12:30 and :nope:', withPack)).toBe('12:30 and :nope:');
    // A miss doesn't swallow the colon that opens the next shortcode.
    expect(plainTextToHtml('at 12:30:scratch:', withPack)).toContain('at 12:30<img class="emoticon"');
    const code = '<p><code>:scratch:</code></p>';
    expect(enrichBody(code, withPack).html).toBe(code);
  });

  it('stays text without a pack lookup', () => {
    expect(plainTextToHtml(':scratch:', options)).toBe(':scratch:');
  });
});

describe('emoji-only messages', () => {
  const withPack = { ...options, customEmoji: () => ({ url: 'http://media.test/e', shortcode: 'e' }) };

  it('are flagged for large display', () => {
    expect(enrichBody('🫡', options).jumbo).toBe(true);
    expect(enrichBody(' 😀 😀 ', options).jumbo).toBe(true);
    expect(enrichBody(':e:', withPack).jumbo).toBe(true);
    // Cinny's custom emoji, after sanitizing.
    expect(enrichBody('<img class="emoticon" src="http://media.test/x" alt="Disagree">', options).jumbo).toBe(true);
  });

  it('do not include text, quotes or walls of emoji', () => {
    expect(enrichBody('ok 😀', options).jumbo).toBe(false);
    expect(enrichBody('plain', options).jumbo).toBe(false);
    expect(enrichBody('<blockquote>😀</blockquote>', options).jumbo).toBe(false);
    expect(enrichBody('😀'.repeat(13), options).jumbo).toBe(false);
  });
});
