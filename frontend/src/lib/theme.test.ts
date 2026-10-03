import { describe, expect, it } from 'vitest';
import { parseHex, themeVariables, withAlpha } from './theme';
import type { ThemeConfig } from './types';

const dark: ThemeConfig = {
  name: 'Test',
  dark: true,
  background: '#1A1A1A',
  surface: '#262626',
  surface_strong: '#333333',
  text: '#F2F2F2',
  muted_text: '#999999',
  accent: '#BDB6EC',
  accent_text: '#2C2843',
  success: '#85E0BA',
  danger: '#E69D9D',
  emote: '#C9A9E0',
  font_family: null,
  ui_scale: 1,
  corner_radius: 10,
};

describe('theme variables', () => {
  it('parses hex with and without alpha, rejects the rest', () => {
    expect(parseHex('#FF8000')).toEqual([255, 128, 0]);
    expect(parseHex('#ff800080')).toEqual([255, 128, 0]);
    expect(parseHex('red')).toBeNull();
    expect(parseHex('#fff')).toBeNull();
  });

  it('builds translucent colors', () => {
    expect(withAlpha('#FF8000', 0.5)).toBe('rgb(255 128 0 / 0.5)');
    expect(withAlpha('not-a-color', 0.5)).toBe('not-a-color');
  });

  it('exposes every themed color as a variable', () => {
    const vars = themeVariables(dark);
    expect(vars['--bg']).toBe('#1A1A1A');
    expect(vars['--accent']).toBe('#BDB6EC');
    expect(vars['--radius']).toBe('10px');
  });

  it('brightens sender names on dark themes and darkens them on light ones', () => {
    expect(themeVariables(dark)['--name-light']).toBe('72%');
    expect(themeVariables({ ...dark, dark: false })['--name-light']).toBe('34%');
  });

  it('quotes a custom font family and strips quote characters from it', () => {
    expect(themeVariables({ ...dark, font_family: 'Cascadia Code' })['--font']).toContain('"Cascadia Code"');
    expect(themeVariables({ ...dark, font_family: 'Evil"; } body { display:none' })['--font']).not.toContain('";');
    expect(themeVariables(dark)['--font']).toContain('system-ui');
  });
});
