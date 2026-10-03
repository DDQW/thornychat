// Maps the persisted `ThemeConfig` onto CSS custom properties. The rest of the
// UI only ever reads the variables, so a theme change is one `applyTheme` call
// and nothing re-renders.

import type { ThemeConfig } from './types';

/** `#RRGGBB[AA]` → [r, g, b] (0-255), or null if it isn't one. */
export function parseHex(hex: string): [number, number, number] | null {
  const match = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})(?:[0-9a-f]{2})?$/i.exec(hex.trim());
  if (!match) return null;
  return [parseInt(match[1]!, 16), parseInt(match[2]!, 16), parseInt(match[3]!, 16)];
}

/** A translucent version of a theme color, for hover washes and focus rings. */
export function withAlpha(hex: string, alpha: number): string {
  const rgb = parseHex(hex);
  return rgb ? `rgb(${rgb[0]} ${rgb[1]} ${rgb[2]} / ${alpha})` : hex;
}

/** The variables a theme sets, as a plain object so it can be tested. */
export function themeVariables(theme: ThemeConfig): Record<string, string> {
  return {
    '--bg': theme.background,
    '--surface': theme.surface,
    '--surface-strong': theme.surface_strong,
    '--text': theme.text,
    '--muted': theme.muted_text,
    '--accent': theme.accent,
    '--accent-text': theme.accent_text,
    '--success': theme.success,
    '--danger': theme.danger,
    '--emote': theme.emote,
    '--accent-wash': withAlpha(theme.accent, 0.16),
    '--danger-wash': withAlpha(theme.danger, 0.16),
    '--hover': withAlpha(theme.text, 0.07),
    '--border': withAlpha(theme.text, 0.12),
    '--radius': `${theme.corner_radius}px`,
    '--radius-sm': `${Math.max(2, theme.corner_radius * 0.6)}px`,
    // Sender names are drawn in a per-user hue; the theme decides how bright.
    '--name-sat': theme.dark ? '62%' : '70%',
    '--name-light': theme.dark ? '72%' : '34%',
    '--font': `${theme.font_family ? `"${theme.font_family.replace(/"/g, '')}", ` : ''}system-ui, "Segoe UI Variable", "Segoe UI", "ThornyChat CJK Fallback", sans-serif`,
  };
}

export function applyTheme(theme: ThemeConfig, root: HTMLElement = document.documentElement): void {
  for (const [name, value] of Object.entries(themeVariables(theme))) root.style.setProperty(name, value);
  root.style.colorScheme = theme.dark ? 'dark' : 'light';
  root.dataset.theme = theme.dark ? 'dark' : 'light';
}
