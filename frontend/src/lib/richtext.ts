// Message text → the markup the timeline shows. Both plain and formatted bodies
// end up here so they behave identically: bare URLs become links and unicode
// emoji become Twemoji images (the same look on every machine).
//
// The input is always either escaped plain text or the output of
// `sanitizeFormattedBody`, and everything this module adds is built with DOM
// APIs (never string concatenation of untrusted text), so it cannot reintroduce
// markup.

import { twemojiCodepoints } from './emoji';

/** Escapes text for use as HTML. */
export function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

const URL_SOURCE = String.raw`https?:\/\/[^\s<>"']+`;
// One pictographic character, optionally with a variation selector or skin
// tone, joined to others by ZWJ; flags (two regional indicators); keycaps; and
// subdivision flags (a tag sequence).
const EMOJI_SOURCE = String.raw`\p{Regional_Indicator}{2}|[#*0-9]️?⃣|\p{Extended_Pictographic}(?:️|\p{Emoji_Modifier})?(?:‍\p{Extended_Pictographic}(?:️|\p{Emoji_Modifier})?)*(?:[\u{E0020}-\u{E007E}]+\u{E007F})?`;
const TOKEN = new RegExp(`(${URL_SOURCE})|(${EMOJI_SOURCE})`, 'gu');

/** Elements whose text is left alone (code is code; a link already is one). */
const SKIP = new Set(['A', 'CODE', 'PRE', 'SCRIPT', 'STYLE', 'TEXTAREA']);

export interface EnrichOptions {
  /** URL of the Twemoji SVG for a codepoint sequence (`1f600`). */
  twemojiUrl: (codepoints: string) => string;
}

/** Splits `url` from the sentence punctuation that trails it. */
function trimUrl(raw: string): { url: string; rest: string } {
  let url = raw.replace(/[.,;:!?)\]}'"]+$/, '');
  // A closing paren that balances an opening one belongs to the URL.
  if (raw.startsWith(url) && raw[url.length] === ')' && (url.match(/\(/g)?.length ?? 0) > (url.match(/\)/g)?.length ?? 0)) {
    url += ')';
  }
  return { url, rest: raw.slice(url.length) };
}

/** Linkifies URLs and replaces emoji with images in `html`; returns new markup. */
export function enrichHtml(html: string, options: EnrichOptions): string {
  const template = document.createElement('template');
  template.innerHTML = html;
  enrichNode(template.content, options);
  return template.innerHTML;
}

function enrichNode(root: ParentNode, options: EnrichOptions): void {
  const texts: Text[] = [];
  const walker = document.createTreeWalker(root as Node, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      for (let parent = node.parentElement; parent; parent = parent.parentElement) {
        if (SKIP.has(parent.nodeName)) return NodeFilter.FILTER_REJECT;
      }
      return NodeFilter.FILTER_ACCEPT;
    },
  });
  for (let node = walker.nextNode(); node; node = walker.nextNode()) texts.push(node as Text);

  for (const textNode of texts) {
    const value = textNode.data;
    TOKEN.lastIndex = 0;
    if (!TOKEN.test(value)) continue;

    const fragment = document.createDocumentFragment();
    let last = 0;
    TOKEN.lastIndex = 0;
    for (const match of value.matchAll(TOKEN)) {
      const index = match.index ?? 0;
      if (index > last) fragment.append(value.slice(last, index));
      if (match[1]) {
        const { url, rest } = trimUrl(match[1]);
        const link = document.createElement('a');
        link.setAttribute('href', url);
        link.setAttribute('rel', 'noopener noreferrer');
        link.setAttribute('data-external', '');
        link.textContent = url;
        fragment.append(link);
        if (rest) fragment.append(rest);
      } else if (match[2]) {
        const glyph = match[2];
        const image = document.createElement('img');
        image.setAttribute('class', 'twemoji');
        image.setAttribute('src', options.twemojiUrl(twemojiCodepoints(glyph)));
        image.setAttribute('alt', glyph);
        image.setAttribute('draggable', 'false');
        fragment.append(image);
      }
      last = index + match[0].length;
    }
    if (last < value.length) fragment.append(value.slice(last));
    textNode.replaceWith(fragment);
  }
}

/** A plain-text body as markup: escaped, then enriched. */
export function plainTextToHtml(text: string, options: EnrichOptions): string {
  return enrichHtml(escapeHtml(text), options);
}
