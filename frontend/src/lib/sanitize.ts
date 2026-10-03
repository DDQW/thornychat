// Sanitizing for message HTML (`formatted_body`).
//
// This is the second of two layers. The Rust side already runs the Matrix
// spec's allow-list over every formatted body before it reaches the page
// (client-core `formatted_body_of`); this layer exists because this webview has
// IPC access, so one miss upstream must not become code execution here. The CSP
// (no inline script, no remote images, no remote connections) is the third.
//
// What survives: the spec's formatting elements, `http(s)`/`mailto`/`matrix`
// links, and `mxc://` images (custom emoji), which are rewritten to the media
// protocol. Everything else — scripts, handlers, remote images, author-chosen
// classes and ids (they could style our own UI: a message must not be able to
// say `class="modal-backdrop"`) — is dropped.

import DOMPurify from 'dompurify';

const ALLOWED_TAGS = [
  'font', 'del', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'blockquote', 'p', 'a', 'ul', 'ol', 'sup', 'sub',
  'li', 'b', 'i', 'u', 'strong', 'em', 'strike', 's', 'code', 'hr', 'br', 'div', 'table', 'thead',
  'tbody', 'tr', 'th', 'td', 'caption', 'pre', 'span', 'img', 'details', 'summary',
];

const ALLOWED_ATTR = [
  'href', 'src', 'alt', 'title', 'width', 'height', 'start', 'class', 'color',
  'data-mx-color', 'data-mx-bg-color', 'data-mx-spoiler', 'data-mx-emoticon',
];

const LINK_SCHEMES = /^(?:https?|mailto|matrix):/i;
const COLOR = /^#[0-9a-f]{6}$/i;
const CODE_CLASS = /^language-[\w+#.-]{1,32}$/;
const USER_PERMALINK = /^https:\/\/matrix\.to\/#\/(@[^/?#\s]+:[^/?#\s]+)/i;

export interface SanitizeOptions {
  /** Maps an `mxc://` URI to a URL the webview can load (the `tcmedia` protocol). */
  mediaUrl: (mxcUrl: string) => string;
}

let purifier: ReturnType<typeof DOMPurify> | null = null;
let activeOptions: SanitizeOptions | null = null;

function getPurifier() {
  if (purifier) return purifier;
  const instance = DOMPurify(window);

  // The rich-reply fallback (`<mx-reply>`) quotes the parent message; the UI
  // draws its own reply preview, so drop the element *and* its text (an
  // unknown tag would otherwise be unwrapped, leaving the quote behind).
  instance.addHook('beforeSanitizeElements', (node) => {
    if (node.nodeName === 'MX-REPLY') node.parentNode?.removeChild(node);
  });

  instance.addHook('uponSanitizeAttribute', (node, data) => {
    if (data.attrName === 'class' && !(node.nodeName === 'CODE' && CODE_CLASS.test(data.attrValue))) {
      data.keepAttr = false;
    }
  });

  instance.addHook('afterSanitizeAttributes', (node) => {
    if (!(node instanceof Element)) return;
    const options = activeOptions;

    if (node.nodeName === 'A') {
      const href = node.getAttribute('href') ?? '';
      if (!LINK_SCHEMES.test(href)) {
        node.removeAttribute('href');
      } else {
        node.setAttribute('rel', 'noopener noreferrer');
        node.setAttribute('data-external', '');
        if (USER_PERMALINK.test(href)) node.setAttribute('class', 'mention');
      }
    }

    if (node.nodeName === 'IMG') {
      const src = node.getAttribute('src') ?? '';
      if (!src.startsWith('mxc://') || !options) {
        // A remote image is a tracking beacon (and the CSP would block it
        // anyway); an image with no source is nothing.
        node.parentNode?.removeChild(node);
        return;
      }
      node.setAttribute('src', options.mediaUrl(src));
      node.setAttribute('loading', 'lazy');
      node.setAttribute('decoding', 'async');
      if (node.hasAttribute('data-mx-emoticon')) {
        node.setAttribute('class', 'emoticon');
        node.removeAttribute('data-mx-emoticon');
      }
    }

    // Matrix colour attributes become an inline colour, but only a plain hex
    // value — never a free-form style string.
    const color = node.getAttribute('data-mx-color') ?? node.getAttribute('color');
    if (color && COLOR.test(color)) {
      const background = node.getAttribute('data-mx-bg-color');
      node.setAttribute('style', `color:${color}${background && COLOR.test(background) ? `;background-color:${background}` : ''}`);
    } else {
      const background = node.getAttribute('data-mx-bg-color');
      if (background && COLOR.test(background)) node.setAttribute('style', `background-color:${background}`);
    }
    node.removeAttribute('data-mx-color');
    node.removeAttribute('data-mx-bg-color');
    node.removeAttribute('color');

    if (node.hasAttribute('data-mx-spoiler')) {
      node.removeAttribute('data-mx-spoiler');
      node.setAttribute('class', 'spoiler');
    }
  });

  purifier = instance;
  return instance;
}

/** Returns markup that is safe to hand to `{@html}`. */
export function sanitizeFormattedBody(html: string, options: SanitizeOptions): string {
  const instance = getPurifier();
  activeOptions = options;
  try {
    return instance.sanitize(html, {
      ALLOWED_TAGS,
      ALLOWED_ATTR,
      ALLOWED_URI_REGEXP: /^(?:(?:https?|mailto|matrix|mxc):|[^a-z]|[a-z+.-]+(?:[^a-z+.\-:]|$))/i,
      ALLOW_DATA_ATTR: false,
      ALLOW_ARIA_ATTR: false,
      KEEP_CONTENT: true,
    });
  } finally {
    activeOptions = null;
  }
}

/** The user ids a sanitized body mentions (pill links), for highlight logic. */
export function mentionedUserIds(sanitizedHtml: string): string[] {
  const ids = new Set<string>();
  const template = document.createElement('template');
  template.innerHTML = sanitizedHtml;
  for (const link of template.content.querySelectorAll('a.mention')) {
    const match = USER_PERMALINK.exec(link.getAttribute('href') ?? '');
    if (match?.[1]) ids.add(decodeURIComponent(match[1]));
  }
  return [...ids];
}
