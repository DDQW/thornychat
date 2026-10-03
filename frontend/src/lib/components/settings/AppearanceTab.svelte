<script lang="ts">
  import { settings } from '../../stores/settings.svelte';
  import { parseHex } from '../../theme';
  import type { ThemeConfig } from '../../types';

  type ColorRole = 'background' | 'surface' | 'surface_strong' | 'text' | 'muted_text' | 'accent' | 'accent_text' | 'success' | 'danger' | 'emote';
  const ROLES: { id: ColorRole; label: string }[] = [
    { id: 'background', label: 'Background' },
    { id: 'surface', label: 'Surface (panels)' },
    { id: 'surface_strong', label: 'Surface (hover/selected)' },
    { id: 'text', label: 'Text' },
    { id: 'muted_text', label: 'Muted text' },
    { id: 'accent', label: 'Accent' },
    { id: 'accent_text', label: 'Text on accent' },
    { id: 'success', label: 'Success' },
    { id: 'danger', label: 'Danger' },
    { id: 'emote', label: 'Emote / action text' },
  ];

  const theme = $derived(settings.value?.theme);
  let importError = $state('');
  let fontDraft = $state('');

  $effect(() => {
    fontDraft = settings.value?.theme.font_family ?? '';
  });

  function setColor(role: ColorRole, hex: string) {
    // Only commit a complete, valid colour; typing "#1" shouldn't repaint the app.
    if (parseHex(hex)) settings.update((d) => (d.theme[role] = hex.toUpperCase()));
  }

  function usePreset(preset: ThemeConfig) {
    // Keeps the font and scale the user chose; a preset is a colour scheme.
    settings.update((d) => {
      d.theme = { ...preset, font_family: d.theme.font_family, ui_scale: d.theme.ui_scale, corner_radius: d.theme.corner_radius };
    });
  }

  function resetToDefault() {
    const dark = settings.presets[0];
    if (dark) settings.update((d) => (d.theme = { ...dark }));
  }

  /** Themes are plain JSON, so they can be shared and dropped back in. */
  async function importTheme(file: File | undefined) {
    importError = '';
    if (!file) return;
    try {
      const parsed = JSON.parse(await file.text()) as Partial<ThemeConfig>;
      const base = settings.presets[0];
      if (!base || typeof parsed !== 'object' || parsed === null) throw new Error('not a theme file');
      const merged: ThemeConfig = { ...base, ...parsed };
      for (const role of ROLES) if (!parseHex(String(merged[role.id]))) throw new Error(`“${role.label}” is not a valid colour`);
      settings.update((d) => (d.theme = merged));
    } catch (error) {
      importError = `Couldn't import theme: ${error instanceof Error ? error.message : String(error)}`;
    }
  }

  function exportTheme() {
    if (!theme) return;
    const blob = new Blob([JSON.stringify(theme, null, 2)], { type: 'application/json' });
    const link = document.createElement('a');
    link.href = URL.createObjectURL(blob);
    link.download = `${theme.name.replace(/[^\w.-]+/g, '-') || 'theme'}.json`;
    link.click();
    setTimeout(() => URL.revokeObjectURL(link.href), 1000);
  }
</script>

{#if theme}
  <h2>Theme</h2>
  <div class="row wrap presets">
    {#each settings.presets as preset (preset.name)}
      <button class="btn small" class:primary={theme.name === preset.name} onclick={() => usePreset(preset)}>{preset.name}</button>
    {/each}
  </div>

  <h2>Colours</h2>
  <div class="colors">
    {#each ROLES as role (role.id)}
      <div class="color">
        <input id="color-{role.id}" type="color" value={theme[role.id].slice(0, 7)} oninput={(e) => setColor(role.id, e.currentTarget.value)} />
        <label for="color-{role.id}">{role.label}</label>
        <input class="input hex" aria-label="{role.label} hex" value={theme[role.id]} onchange={(e) => setColor(role.id, e.currentTarget.value)} spellcheck="false" />
      </div>
    {/each}
  </div>

  <h2>Type and density</h2>
  <div class="setting">
    <div class="what"><label for="font">Font family</label><div class="hint">Leave empty for the system font. Use a font installed on this PC.</div></div>
    <input
      id="font"
      class="input fit"
      placeholder="Default"
      bind:value={fontDraft}
      onchange={() => settings.update((d) => (d.theme.font_family = fontDraft.trim() || null))}
      spellcheck="false"
    />
  </div>
  <div class="setting">
    <div class="what"><label for="scale">UI scale: {Math.round(theme.ui_scale * 100)}%</label></div>
    <input id="scale" type="range" min="0.8" max="1.5" step="0.05" value={theme.ui_scale} oninput={(e) => settings.update((d) => (d.theme.ui_scale = Number(e.currentTarget.value)))} />
  </div>
  <div class="setting">
    <div class="what"><label for="radius">Corner radius: {Math.round(theme.corner_radius)}px</label></div>
    <input id="radius" type="range" min="0" max="16" step="1" value={theme.corner_radius} oninput={(e) => settings.update((d) => (d.theme.corner_radius = Number(e.currentTarget.value)))} />
  </div>

  <h2>Share</h2>
  <div class="row wrap">
    <label class="btn small">
      Import theme…
      <input type="file" accept=".json,application/json" hidden onchange={(e) => { void importTheme(e.currentTarget.files?.[0]); e.currentTarget.value = ''; }} />
    </label>
    <button class="btn small" onclick={exportTheme}>Export theme…</button>
    <button class="btn small ghost" onclick={resetToDefault}>Reset to default</button>
  </div>
  {#if importError}<p class="error-text small" role="alert">{importError}</p>{/if}
{/if}

<style>
  .wrap {
    flex-wrap: wrap;
  }
  .colors {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(250px, 1fr));
    gap: 8px 16px;
  }
  .color {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .color label {
    flex: 1 1 auto;
    font-size: 13px;
  }
  .color input[type='color'] {
    width: 30px;
    height: 26px;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: none;
  }
  .hex {
    width: 92px;
    padding: 4px 6px;
    font-family: ui-monospace, Consolas, monospace;
    font-size: 12px;
  }
  .fit {
    width: 200px;
  }
  input[type='range'] {
    width: 200px;
    accent-color: var(--accent);
  }
</style>
