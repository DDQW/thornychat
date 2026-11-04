<script lang="ts">
  import { ui, type SettingsTab } from '../stores/ui.svelte';
  import AppearanceTab from './settings/AppearanceTab.svelte';
  import ConnectorsTab from './settings/ConnectorsTab.svelte';
  import GeneralTab from './settings/GeneralTab.svelte';
  import ManualTab from './settings/ManualTab.svelte';
  import NotificationsTab from './settings/NotificationsTab.svelte';
  import PrivacyTab from './settings/PrivacyTab.svelte';
  import SecurityTab from './settings/SecurityTab.svelte';
  import Icon, { type IconName } from './Icon.svelte';
  import { settings } from '../stores/settings.svelte';
  import { arrowTarget, trapFocus } from '../focus';

  const TABS: { id: SettingsTab; label: string; icon: IconName }[] = [
    { id: 'general', label: 'General', icon: 'settings' },
    { id: 'appearance', label: 'Appearance', icon: 'image' },
    { id: 'notifications', label: 'Notifications', icon: 'bell' },
    { id: 'privacy', label: 'Privacy', icon: 'eye' },
    { id: 'security', label: 'Security', icon: 'shield' },
    { id: 'connectors', label: 'Connectors', icon: 'play' },
    { id: 'manual', label: 'Manual', icon: 'info' },
  ];

  /** Up/Down (and Home/End) move between sections, as in any vertical tab list. */
  function ontabkeydown(event: KeyboardEvent, index: number) {
    const next = arrowTarget(event.key, index, TABS.length);
    if (next === null) return;
    event.preventDefault();
    ui.settingsTab = TABS[next]!.id;
    document.getElementById(`settings-tab-${TABS[next]!.id}`)?.focus();
  }

  function close() {
    ui.settingsOpen = false;
    void settings.flush();
  }
</script>

<div class="backdrop" role="presentation" onmousedown={(event) => event.target === event.currentTarget && close()}>
  <div class="dialog settings" role="dialog" aria-modal="true" aria-labelledby="settings-title" use:trapFocus={{ initial: '[role=tab][aria-selected=true]' }}>
    <header>
      <h2 class="grow title" id="settings-title">Settings</h2>
      <button class="icon-btn" onclick={close} aria-label="Close settings"><Icon name="x" /></button>
    </header>
    <div class="layout">
      <div class="tabs" role="tablist" aria-label="Settings sections" aria-orientation="vertical">
        {#each TABS as tab, index (tab.id)}
          <button
            role="tab"
            id="settings-tab-{tab.id}"
            class:on={ui.settingsTab === tab.id}
            aria-selected={ui.settingsTab === tab.id}
            aria-controls="settings-panel"
            tabindex={ui.settingsTab === tab.id ? 0 : -1}
            title={tab.label}
            onclick={() => (ui.settingsTab = tab.id)}
            onkeydown={(event) => ontabkeydown(event, index)}
          >
            <Icon name={tab.icon} size={16} /> <span class="label">{tab.label}</span>
          </button>
        {/each}
      </div>
      <div class="content" role="tabpanel" id="settings-panel" aria-labelledby="settings-tab-{ui.settingsTab}" tabindex="-1">
        {#if settings.saveError}<p class="error-text" role="alert">{settings.saveError}</p>{/if}
        {#if ui.settingsTab === 'general'}<GeneralTab />
        {:else if ui.settingsTab === 'appearance'}<AppearanceTab />
        {:else if ui.settingsTab === 'notifications'}<NotificationsTab />
        {:else if ui.settingsTab === 'privacy'}<PrivacyTab />
        {:else if ui.settingsTab === 'security'}<SecurityTab />
        {:else if ui.settingsTab === 'connectors'}<ConnectorsTab />
        {:else}<ManualTab />{/if}
      </div>
    </div>
  </div>
</div>

<style>
  .settings {
    width: min(820px, 100%);
    height: min(640px, 100%);
  }
  .title {
    margin: 0;
    font-size: inherit;
  }
  .layout {
    display: flex;
    flex: 1 1 auto;
    min-height: 0;
  }
  .tabs {
    display: flex;
    flex: 0 0 196px;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-1) var(--space-3) var(--space-3);
    overflow-y: auto;
  }
  .tabs button {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 34px;
    padding: 0 10px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: color-mix(in srgb, var(--text), var(--muted) 35%);
    text-align: left;
  }
  .tabs button:hover {
    background: var(--hover);
  }
  .tabs button.on {
    background: var(--surface-strong);
    color: var(--text);
    font-weight: 600;
  }
  .content {
    flex: 1 1 auto;
    padding: 0 var(--space-6) var(--space-6) var(--space-3);
    overflow-y: auto;
  }
  .content :global(h2) {
    margin: var(--space-6) 0 var(--space-1);
    font-size: var(--text-base);
    font-weight: 600;
  }
  .content :global(h2:first-child) {
    margin-top: var(--space-2);
  }
  /* Narrow windows: the section list shrinks to its icons (named by their titles). */
  @media (max-width: 640px) {
    .tabs {
      flex-basis: 52px;
    }
    .tabs .label {
      display: none;
    }
  }
</style>
