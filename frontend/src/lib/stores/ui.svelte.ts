export type SettingsTab =
  | 'general'
  | 'appearance'
  | 'notifications'
  | 'privacy'
  | 'security'
  | 'connectors'
  | 'manual';

export interface Toast {
  id: number;
  text: string;
  kind: 'info' | 'error' | 'success';
}

export interface MenuItem {
  label: string;
  onselect: () => void;
  danger?: boolean;
  disabled?: boolean;
  /** A choice among several (drawn with a check mark when true); undefined for a plain action. */
  checked?: boolean;
  /** Draw a divider above this item. */
  separator?: boolean;
}

export interface MenuState {
  x: number;
  y: number;
  items: MenuItem[];
}

export interface LightboxState {
  mxcUrl: string;
  /** Real filename, offered when saving. */
  name: string;
}

export type RoomAction = { kind: 'leave' | 'forget' | 'rename'; roomId: string } | { kind: 'join' };

let nextToastId = 1;
const toastTimers = new Map<number, { remaining: number; started: number; timer: ReturnType<typeof setTimeout> | undefined }>();

class UiStore {
  settingsOpen = $state(false);
  settingsTab = $state<SettingsTab>('general');
  /** The space whose children the explorer is showing; closed when null. */
  spaceExplorer = $state<string | null>(null);
  dmSearchOpen = $state(false);
  lightbox = $state.raw<LightboxState | null>(null);
  toasts = $state.raw<Toast[]>([]);
  menu = $state.raw<MenuState | null>(null);
  roomAction = $state.raw<RoomAction | null>(null);
  /** Set by the timeline when the user asks to jump to a message in the open room. */
  highlightEvent = $state<string | null>(null);

  /** A modal is up: the page behind it is inert, and the app's shortcuts are off. */
  get modalOpen(): boolean {
    return this.settingsOpen || !!this.spaceExplorer || this.dmSearchOpen || !!this.roomAction || !!this.lightbox;
  }

  openSettings(tab?: SettingsTab): void {
    if (tab) this.settingsTab = tab;
    this.settingsOpen = true;
  }

  toast(text: string, kind: Toast['kind'] = 'info', durationMs = 5000): void {
    const toast: Toast = { id: nextToastId++, text, kind };
    this.toasts = [...this.toasts, toast].slice(-4);
    toastTimers.set(toast.id, { remaining: durationMs, started: performance.now(), timer: this.toastsPaused ? undefined : setTimeout(() => this.dismissToast(toast.id), durationMs) });
  }

  /** Toasts don't time out while the pointer or focus is on them, so they can be read (and clicked). */
  toastsPaused = false;
  pauseToasts(paused: boolean): void {
    if (paused === this.toastsPaused) return;
    this.toastsPaused = paused;
    const now = performance.now();
    for (const [id, entry] of toastTimers) {
      if (paused) {
        clearTimeout(entry.timer);
        entry.timer = undefined;
        entry.remaining = Math.max(0, entry.remaining - (now - entry.started));
      } else {
        entry.started = now;
        // A moment to move away before it goes.
        entry.timer = setTimeout(() => this.dismissToast(id), Math.max(entry.remaining, 1500));
      }
    }
  }

  error(text: string): void {
    this.toast(text, 'error', 8000);
  }

  dismissToast(id: number): void {
    clearTimeout(toastTimers.get(id)?.timer);
    toastTimers.delete(id);
    this.toasts = this.toasts.filter((toast) => toast.id !== id);
  }

  showMenu(event: MouseEvent, items: MenuItem[]): void {
    event.preventDefault();
    event.stopPropagation();
    // Opened from the keyboard (Enter on a button, the Menu key, Shift+F10)
    // there is no pointer position: the menu goes under the element instead.
    const anchor = event.currentTarget instanceof Element ? event.currentTarget : event.target instanceof Element ? event.target : null;
    if (anchor && ((event.type === 'click' && event.detail === 0) || (event.clientX === 0 && event.clientY === 0))) {
      const rect = anchor.getBoundingClientRect();
      this.menu = { x: rect.left, y: rect.bottom + 2, items };
      return;
    }
    this.menu = { x: event.clientX, y: event.clientY, items };
  }

  closeMenu(): void {
    this.menu = null;
  }

  /** Escape closes the topmost overlay; returns whether it closed something. */
  closeTopmost(): boolean {
    if (this.menu) return void (this.menu = null), true;
    if (this.lightbox) return void (this.lightbox = null), true;
    if (this.roomAction) return void (this.roomAction = null), true;
    if (this.dmSearchOpen) return void (this.dmSearchOpen = false), true;
    if (this.spaceExplorer) return void (this.spaceExplorer = null), true;
    if (this.settingsOpen) return void (this.settingsOpen = false), true;
    return false;
  }

  reset(): void {
    this.settingsOpen = false;
    this.spaceExplorer = null;
    this.dmSearchOpen = false;
    this.lightbox = null;
    this.menu = null;
    this.roomAction = null;
    this.highlightEvent = null;
  }
}

export const ui = new UiStore();
