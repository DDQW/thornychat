import { backend } from '../api';
import { applyTheme } from '../theme';
import type { Settings, ThemeConfig } from '../types';

/** How long to wait after the last change before writing to disk (sliders fire constantly). */
const SAVE_DEBOUNCE_MS = 400;

class SettingsStore {
  /** `null` until the shell has answered; nothing renders before then. */
  value = $state.raw<Settings | null>(null);
  presets = $state.raw<ThemeConfig[]>([]);
  saveError = $state('');

  #saveTimer: ReturnType<typeof setTimeout> | null = null;

  async load(): Promise<void> {
    const [settings, presets] = await Promise.all([backend.getSettings(), backend.themePresets()]);
    this.presets = presets;
    this.#adopt(settings);
  }

  /**
   * Applies `change` to a copy of the settings, shows the result immediately,
   * and writes it to disk shortly after the last change. The shell's answer
   * (clamped values) replaces the optimistic one.
   */
  update(change: (draft: Settings) => void): void {
    if (!this.value) return;
    const draft = structuredClone($state.snapshot(this.value)) as Settings;
    change(draft);
    this.#adopt(draft);
    if (this.#saveTimer) clearTimeout(this.#saveTimer);
    this.#saveTimer = setTimeout(() => void this.#persist(), SAVE_DEBOUNCE_MS);
  }

  /** Writes any pending change now (before the window closes, or on sign-out). */
  async flush(): Promise<void> {
    if (this.#saveTimer) {
      clearTimeout(this.#saveTimer);
      await this.#persist();
    }
  }

  async #persist(): Promise<void> {
    this.#saveTimer = null;
    if (!this.value) return;
    try {
      const saved = await backend.setSettings($state.snapshot(this.value) as Settings);
      this.saveError = '';
      // Adopt the shell's sanitized copy unless the user has already moved on.
      if (!this.#saveTimer) this.#adopt(saved);
    } catch (error) {
      this.saveError = String(error);
    }
  }

  #adopt(settings: Settings): void {
    this.value = settings;
    applyTheme(settings.theme);
  }
}

export const settings = new SettingsStore();
