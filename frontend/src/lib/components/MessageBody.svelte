<script lang="ts">
  import { backend } from '../api';
  import { enrichHtml, plainTextToHtml } from '../richtext';
  import { sanitizeFormattedBody } from '../sanitize';

  interface Props {
    /** The plain body. Always present; used when there is no formatted version. */
    text: string;
    /** Sanitized-by-Rust HTML from `formatted_body`; sanitized again here. */
    formatted?: string | null;
  }
  let { text, formatted = null }: Props = $props();

  const enrich = { twemojiUrl: (codepoints: string) => backend.twemojiUrl(codepoints) };

  const html = $derived(
    formatted
      ? enrichHtml(sanitizeFormattedBody(formatted, { mediaUrl: (mxc) => backend.mediaUrl(mxc) }), enrich)
      : plainTextToHtml(text, enrich),
  );

  /** Click a spoiler to reveal it. */
  function reveal(event: MouseEvent) {
    const spoiler = (event.target as Element | null)?.closest('.spoiler');
    if (spoiler) spoiler.classList.toggle('revealed');
  }

  /** A Twemoji image that fails to load (offline, first use) falls back to the real glyph. */
  function emojiFallback(event: Event) {
    const image = event.target;
    if (image instanceof HTMLImageElement && image.classList.contains('twemoji')) {
      image.replaceWith(document.createTextNode(image.alt));
    }
  }
</script>

<!-- eslint-disable-next-line svelte/no-at-html-tags -- sanitized above, twice -->
<div class="body" class:plain={!formatted} onclick={reveal} onerrorcapture={emojiFallback} role="presentation">{@html html}</div>

<style>
  .body {
    overflow-wrap: anywhere;
  }
  .body.plain {
    white-space: pre-wrap;
  }
  .body :global(p) {
    margin: 0 0 6px;
  }
  .body :global(p:last-child) {
    margin-bottom: 0;
  }
  .body :global(blockquote) {
    margin: 4px 0;
    padding: 2px 10px;
    border-left: 3px solid var(--border);
    color: var(--muted);
  }
  .body :global(pre) {
    margin: 6px 0;
    padding: 10px 12px;
    border-radius: var(--radius-sm);
    background: var(--bg);
    overflow-x: auto;
    white-space: pre;
  }
  .body :global(code) {
    padding: 1px 4px;
    border-radius: 4px;
    background: var(--bg);
    font: 0.92em ui-monospace, 'Cascadia Code', Consolas, monospace;
  }
  .body :global(pre code) {
    padding: 0;
    background: none;
  }
  .body :global(ul),
  .body :global(ol) {
    margin: 4px 0;
    padding-left: 24px;
  }
  .body :global(h1),
  .body :global(h2),
  .body :global(h3),
  .body :global(h4) {
    margin: 8px 0 4px;
    line-height: 1.25;
  }
  .body :global(h1) {
    font-size: 1.4em;
  }
  .body :global(h2) {
    font-size: 1.25em;
  }
  .body :global(h3) {
    font-size: 1.1em;
  }
  .body :global(table) {
    margin: 6px 0;
    border-collapse: collapse;
  }
  .body :global(th),
  .body :global(td) {
    padding: 3px 8px;
    border: 1px solid var(--border);
  }
  .body :global(hr) {
    border: 0;
    border-top: 1px solid var(--border);
  }
  .body :global(a.mention) {
    padding: 0 4px;
    border-radius: 4px;
    background: var(--accent-wash);
    color: var(--accent);
    font-weight: 600;
    text-decoration: none;
  }
  .body :global(.spoiler:not(.revealed)) {
    border-radius: 3px;
    background: var(--text);
    color: transparent;
    cursor: pointer;
    user-select: none;
  }
  .body :global(.spoiler:not(.revealed) *) {
    visibility: hidden;
  }
  .body :global(img.twemoji) {
    width: 1.3em;
    height: 1.3em;
    vertical-align: -0.3em;
  }
  /* A message that is only emoji is drawn large, like every chat app does. */
  .body :global(img.emoticon) {
    max-width: 2em;
    max-height: 2em;
    vertical-align: -0.5em;
    object-fit: contain;
  }
</style>
