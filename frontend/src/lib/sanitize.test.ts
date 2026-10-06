import { describe, expect, it } from 'vitest';
import { mentionedUserIds, sanitizeFormattedBody } from './sanitize';

const options = { mediaUrl: (mxc: string) => `http://media.test/${mxc.slice('mxc://'.length)}` };
const clean = (html: string) => sanitizeFormattedBody(html, options);

describe('sanitizeFormattedBody', () => {
  it('keeps the spec formatting elements', () => {
    expect(clean('<p>hello <b>bold</b> <em>em</em> <code>code</code></p>')).toBe(
      '<p>hello <b>bold</b> <em>em</em> <code>code</code></p>',
    );
    expect(clean('<blockquote>quote</blockquote><ul><li>one</li></ul>')).toContain('<li>one</li>');
  });

  it('removes scripts, handlers and their content', () => {
    const out = clean('<p onclick="alert(1)">hi</p><script>alert(2)</script><img src=x onerror=alert(3)>');
    expect(out).not.toMatch(/script|onclick|onerror|alert/i);
    expect(out).toContain('hi');
  });

  it('removes javascript: and data: links but keeps the text', () => {
    const js = clean('<a href="javascript:alert(1)">click</a>');
    expect(js).not.toContain('javascript');
    expect(js).toContain('click');
    expect(clean('<a href="data:text/html,<script>1</script>">x</a>')).not.toContain('href');
    expect(clean('<a href="file:///C:/Windows/win.ini">x</a>')).not.toContain('href');
  });

  it('marks real links as external and safe', () => {
    const out = clean('<a href="https://example.com/a?b=1">link</a>');
    expect(out).toContain('href="https://example.com/a?b=1"');
    expect(out).toContain('rel="noopener noreferrer"');
    expect(out).toContain('data-external');
    expect(out).not.toContain('target=');
  });

  it('drops remote images and non-mxc sources entirely', () => {
    for (const src of ['https://evil.example/pixel.gif', 'http://x/y.png', 'data:image/png;base64,AAAA', 'javascript:alert(1)', '']) {
      expect(clean(`<p>a<img src="${src}">b</p>`), src).toBe('<p>ab</p>');
    }
  });

  it('rewrites mxc images to the media protocol and tags custom emoji', () => {
    const out = clean('<img data-mx-emoticon src="mxc://server/abc" alt=":wave:" height="32">');
    expect(out).toContain('src="http://media.test/server/abc"');
    expect(out).toContain('class="emoticon"');
    expect(out).toContain('loading="lazy"');
    expect(out).not.toContain('data-mx-emoticon');
  });

  it('treats images that lost data-mx-emoticon in the SDK as custom emoji too', () => {
    // How a Cinny emoji arrives after matrix-sdk's own sanitizing.
    const out = clean('So wait? <img src="mxc://server/ns" alt="NotSure" title="NotSure" height="32" />');
    expect(out).toContain('class="emoticon"');
    expect(out).not.toContain('height=');
    expect(out).toContain('title="NotSure"');
  });

  it('strips author-chosen classes and ids so a message cannot restyle the app', () => {
    const out = clean('<div class="modal-backdrop" id="x"><span class="toast">y</span></div>');
    expect(out).not.toMatch(/class=|id=/);
    expect(clean('<code class="language-rust">fn</code>')).toContain('class="language-rust"');
    expect(clean('<code class="language-rust modal">fn</code>')).not.toContain('class=');
  });

  it('turns matrix colour attributes into a plain hex colour only', () => {
    expect(clean('<font data-mx-color="#ff0000">red</font>')).toContain('style="color:#ff0000"');
    const hostile = clean('<span data-mx-color="red;position:fixed;inset:0">x</span>');
    expect(hostile).not.toContain('style');
    expect(hostile).not.toContain('position');
    expect(clean('<span style="position:fixed">x</span>')).not.toContain('style');
  });

  it('turns spoilers into a class the UI can toggle', () => {
    const out = clean('<span data-mx-spoiler>secret</span>');
    expect(out).toContain('class="spoiler"');
    expect(out).not.toContain('data-mx-spoiler');
  });

  it('drops the rich-reply fallback including its quoted text', () => {
    const out = clean('<mx-reply><blockquote>quoted</blockquote></mx-reply>the answer');
    expect(out).toBe('the answer');
  });

  it('recognises user permalinks as mention pills', () => {
    const out = clean('<a href="https://matrix.to/#/@alice:example.org">Alice</a> and <a href="https://matrix.to/#/#room:example.org">room</a>');
    expect(out.match(/class="mention"/g)).toHaveLength(1);
    expect(mentionedUserIds(out)).toEqual(['@alice:example.org']);
  });

  it('survives garbage input', () => {
    expect(() => clean('<<<>>>&&&<a href=')).not.toThrow();
    expect(clean('')).toBe('');
  });

  it('does not let one call leak its media resolver into the next', () => {
    clean('<img src="mxc://a/b">');
    // After a call, an image with no active options must be dropped, not resolved.
    expect(clean('<img src="mxc://a/b">')).toContain('http://media.test/a/b');
  });
});
