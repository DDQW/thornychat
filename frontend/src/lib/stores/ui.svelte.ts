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

  openSettings(tab?: SettingsTab): void {
    if (tab) this.settingsTab = tab;
    this.settingsOpen = true;
  }

  toast(text: string, kind: Toast['kind'] = 'info', durationMs = 5000): void {
    const toast: Toast = { id: nextToastId++, text, kind };
    this.toasts = [...this.toasts, toast].slice(-4);
    setTimeout(() => this.dismissToast(toast.id), durationMs);
  }

  error(text: string): void {
    this.toast(text, 'error', 8000);
  }

  dismissToast(id: number): void {
    this.toasts = this.toasts.filter((toast) => toast.id !== id);
  }

  showMenu(event: MouseEvent, items: MenuItem[]): void {
    event.preventDefault();
    event.stopPropagation();
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
