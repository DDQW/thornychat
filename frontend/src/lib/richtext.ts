// Message text → the markup the timeline shows. Both plain and formatted bodies
// end up here so they behave identically: bare URLs become links, unicode emoji
// become Twemoji images (the same look on every machine), and a `:shortcode:`
// that names one of the room's pack emoji becomes that image — the form this
// client sent custom emoji in before it sent them as HTML, and what IRC-side
// and older clients still send.
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
// The opening colon and name only: the closing colon is checked by lookahead
// and consumed on a hit, so after a miss ("12:30:sad:") the scan resumes at
// that colon and still finds ":sad:".
const SHORTCODE_SOURCE = String.raw`:([\w+-]{1,64})(?=:)`;
const TOKEN = new RegExp(`(${URL_SOURCE})|(${EMOJI_SOURCE})|${SHORTCODE_SOURCE}`, 'gu');

/** Elements whose text is left alone (code is code; a link already is one). */
const SKIP = new Set(['A', 'CODE', 'PRE', 'SCRIPT', 'STYLE', 'TEXTAREA']);

/** At most this many emoji in an emoji-only message are drawn large. */
const MAX_JUMBO_EMOJI = 12;

export interface CustomEmojiImage {
  /** Loadable image URL (the media protocol, not `mxc://`). */
  url: string;
  shortcode: string;
}

export interface EnrichOptions {
  /** URL of the Twemoji SVG for a codepoint sequence (`1f600`). */
  twemojiUrl: (codepoints: string) => string;
  /** The pack emoji a `:shortcode:` names, if any. Without it shortcodes stay text. */
  customEmoji?: (shortcode: string) => CustomEmojiImage | null;
}

export interface EnrichedBody {
  html: string;
  /** Nothing but a few emoji (unicode or custom): drawn large. */
  jumbo: boolean;
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
  return enrichBody(html, options).html;
}

/** [`enrichHtml`], plus whether the result is emoji-only. */
export function enrichBody(html: string, options: EnrichOptions): EnrichedBody {
  const template = document.createElement('template');
  template.innerHTML = html;
  enrichNode(template.content, options);
  return { html: template.innerHTML, jumbo: isEmojiOnly(template.content) };
}

/**
 * Whether `root` holds nothing but emoji images (Twemoji or custom) and
 * whitespace, with at least one and at most [`MAX_JUMBO_EMOJI`] of them.
 */
function isEmojiOnly(root: DocumentFragment): boolean {
  if (root.textContent?.trim()) return false;
  const images = root.querySelectorAll('img');
  if (images.length === 0 || images.length > MAX_JUMBO_EMOJI) return false;
  for (const image of images) {
    if (!image.classList.contains('twemoji') && !image.classList.contains('emoticon')) return false;
  }
  // Paragraphs and line breaks are fine; anything else (a quote, a list)
  // is a message that happens to contain only emoji, not an emoji message.
  for (const element of root.querySelectorAll('*')) {
    if (!['IMG', 'P', 'BR', 'SPAN', 'DIV'].includes(element.nodeName)) return false;
  }
  return true;
}

function emojiImage(className: 'twemoji' | 'emoticon', src: string, alt: string): HTMLImageElement {
  const image = document.createElement('img');
  image.setAttribute('class', className);
  image.setAttribute('src', src);
  image.setAttribute('alt', alt);
  if (className === 'emoticon') image.setAttribute('title', alt);
  image.setAttribute('draggable', 'false');
  return image;
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
    let fragment: DocumentFragment | null = null;
    let last = 0;
    TOKEN.lastIndex = 0;
    for (let match = TOKEN.exec(value); match; match = TOKEN.exec(value)) {
      let node: Node | null = null;
      let rest = '';
      let end = match.index + match[0].length;
      if (match[1]) {
        const trimmed = trimUrl(match[1]);
        const link = document.createElement('a');
        link.setAttribute('href', trimmed.url);
        link.setAttribute('rel', 'noopener noreferrer');
        link.setAttribute('data-external', '');
        link.textContent = trimmed.url;
        node = link;
        rest = trimmed.rest;
      } else if (match[2]) {
        node = emojiImage('twemoji', options.twemojiUrl(twemojiCodepoints(match[2])), match[2]);
      } else if (match[3]) {
        const emoji = options.customEmoji?.(match[3]);
        if (!emoji) continue; // Not a pack emoji: plain text, resume at the closing colon.
        node = emojiImage('emoticon', emoji.url, `:${emoji.shortcode}:`);
        end += 1; // The closing colon.
        TOKEN.lastIndex = end;
      }
      if (!node) continue;
      fragment ??= document.createDocumentFragment();
      if (match.index > last) fragment.append(value.slice(last, match.index));
      fragment.append(node);
      if (rest) fragment.append(rest);
      last = end;
    }
    if (!fragment) continue;
    if (last < value.length) fragment.append(value.slice(last));
    textNode.replaceWith(fragment);
  }
}

/** A plain-text body as markup: escaped, then enriched. */
export function plainTextToHtml(text: string, options: EnrichOptions): string {
  return enrichHtml(escapeHtml(text), options);
}
