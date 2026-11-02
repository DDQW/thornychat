// Keyboard focus for overlays: a dialog takes focus when it opens, keeps Tab
// inside itself while it is open, and hands focus back to whatever opened it
// when it closes. Without that, Tab walked out of a modal into the (hidden)
// page behind it, and closing one dropped focus on <body>, so a keyboard user
// had to start again from the top of the window.

const FOCUSABLE = [
  'a[href]',
  'button:not([disabled])',
  'input:not([disabled]):not([type="hidden"])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  'video[controls]',
  'iframe',
  '[tabindex]:not([tabindex="-1"])',
  '[contenteditable="true"]',
].join(',');

/** What had focus when the context menu opened (see `currentOpener`). */
let menuOpener: HTMLElement | null = null;
export function rememberMenuOpener(element: HTMLElement | null): void {
  menuOpener = element;
}

/**
 * Whatever focus should return to when an overlay opening now closes. A menu
 * item that opens a dialog has focus at that moment, but the menu is about to
 * go: the dialog hands focus back to what the menu was opened from instead.
 */
function currentOpener(): HTMLElement | null {
  const active = document.activeElement;
  // On <body>: the menu item that was chosen is already gone.
  if (!(active instanceof HTMLElement) || active === document.body) return menuOpener;
  return active.closest('[role=menu]') ? menuOpener : active;
}

/**
 * Where focus returns to: the opener, or — when the opener went away while the
 * overlay was up (a message's action bar only exists while the message has
 * focus or the pointer) — the nearest focusable thing that held it.
 */
export function returnPoint(opener: HTMLElement | null): () => HTMLElement | null {
  const fallback = opener?.parentElement?.closest<HTMLElement>('[tabindex]') ?? null;
  return () => (opener?.isConnected ? opener : fallback?.isConnected ? fallback : null);
}

/** Moves focus back, unless something else has taken it on purpose since. */
export function giveBack(back: () => HTMLElement | null, overlay: Element | null): void {
  const active = document.activeElement;
  if (active && active !== document.body && !overlay?.contains(active)) return;
  // After the rest of this update: the page behind a modal is inert until
  // then, and an inert element ignores focus().
  queueMicrotask(() => {
    const now = document.activeElement;
    if (now && now !== document.body && !overlay?.contains(now)) return;
    back()?.focus({ preventScroll: true });
  });
}

/** The elements Tab can reach inside `root`, in order. */
export function tabbables(root: Element): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
    (element) => !element.hidden && !element.closest('[inert]') && element.getClientRects().length > 0,
  );
}

/**
 * Svelte action for a modal surface. `initial` picks what gets focus first
 * (default: an `[autofocus]` element, else the first tabbable one, else the
 * surface itself).
 */
export function trapFocus(node: HTMLElement, options: { initial?: string } = {}) {
  const back = returnPoint(currentOpener());
  if (!node.hasAttribute('tabindex')) node.tabIndex = -1;

  // After the children's own `autofocus` has run, so that one wins.
  queueMicrotask(() => {
    if (node.contains(document.activeElement)) return;
    const wanted = options.initial ? node.querySelector<HTMLElement>(options.initial) : null;
    (wanted ?? node.querySelector<HTMLElement>('[autofocus]') ?? tabbables(node)[0] ?? node).focus();
  });

  function onkeydown(event: KeyboardEvent) {
    if (event.key !== 'Tab') return;
    const list = tabbables(node);
    if (list.length === 0) return void event.preventDefault();
    const first = list[0]!;
    const last = list.at(-1)!;
    const at = document.activeElement;
    if (event.shiftKey && (at === first || at === node)) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && at === last) {
      event.preventDefault();
      first.focus();
    }
  }
  node.addEventListener('keydown', onkeydown);

  return {
    destroy() {
      node.removeEventListener('keydown', onkeydown);
      giveBack(back, node);
    },
  };
}

/**
 * Svelte action for a popover that isn't modal (the emoji picker): focus goes
 * back to the opener when it closes, and Escape closes it without reaching the
 * window's handler, which would close whatever modal is underneath instead.
 */
export function popover(node: HTMLElement, onclose: () => void) {
  const back = returnPoint(currentOpener());
  let close = onclose;
  function onkeydown(event: KeyboardEvent) {
    if (event.key !== 'Escape') return;
    event.stopPropagation();
    event.preventDefault();
    close();
  }
  node.addEventListener('keydown', onkeydown);
  // Svelte's `autofocus` only acts when nothing has focus, and the button that
  // opened this does.
  queueMicrotask(() => {
    if (!node.contains(document.activeElement)) node.querySelector<HTMLElement>('[autofocus]')?.focus();
  });
  return {
    update(next: () => void) {
      close = next;
    },
    destroy() {
      node.removeEventListener('keydown', onkeydown);
      giveBack(back, node);
    },
  };
}

/**
 * Arrow-key movement through a set of items (a menu, a tab list, a grid):
 * returns the index to move to, or null when the key isn't a movement key.
 * `columns` > 1 makes Up/Down jump a row.
 */
export function arrowTarget(key: string, index: number, count: number, { columns = 1, wrap = true, horizontal = false } = {}): number | null {
  if (count === 0) return null;
  const step = (delta: number) => {
    const next = index + delta;
    if (wrap) return (next + count) % count;
    return Math.max(0, Math.min(count - 1, next));
  };
  switch (key) {
    case 'Home':
      return 0;
    case 'End':
      return count - 1;
    case 'ArrowDown':
      return columns > 1 ? Math.min(count - 1, index + columns) : horizontal ? null : step(1);
    case 'ArrowUp':
      return columns > 1 ? Math.max(0, index - columns) : horizontal ? null : step(-1);
    case 'ArrowRight':
      return columns > 1 || horizontal ? step(1) : null;
    case 'ArrowLeft':
      return columns > 1 || horizontal ? step(-1) : null;
    default:
      return null;
  }
}
