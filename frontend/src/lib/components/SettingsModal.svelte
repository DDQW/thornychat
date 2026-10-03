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

  const TABS: { id: SettingsTab; label: string; icon: IconName }[] = [
    { id: 'general', label: 'General', icon: 'settings' },
    { id: 'appearance', label: 'Appearance', icon: 'image' },
    { id: 'notifications', label: 'Notifications', icon: 'bell' },
    { id: 'privacy', label: 'Privacy', icon: 'eye' },
    { id: 'security', label: 'Security', icon: 'shield' },
    { id: 'connectors', label: 'Connectors', icon: 'play' },
    { id: 'manual', label: 'Manual', icon: 'info' },
  ];

  function close() {
    ui.settingsOpen = false;
    void settings.flush();
  }
</script>

<div class="backdrop" role="presentation" onmousedown={(event) => event.target === event.currentTarget && close()}>
  <div class="dialog settings" role="dialog" aria-modal="true" aria-label="Settings">
    <header>
      <span class="grow">Settings</span>
      <button class="icon-btn" onclick={close} aria-label="Close settings"><Icon name="x" /></button>
    </header>
    <div class="layout">
      <div class="tabs" role="tablist" aria-label="Settings sections">
        {#each TABS as tab (tab.id)}
          <button role="tab" class:on={ui.settingsTab === tab.id} aria-selected={ui.settingsTab === tab.id} onclick={() => (ui.settingsTab = tab.id)}>
            <Icon name={tab.icon} size={16} /> {tab.label}
          </button>
        {/each}
      </div>
      <div class="content" role="tabpanel">
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
  .layout {
    display: flex;
    flex: 1 1 auto;
    min-height: 0;
  }
  .tabs {
    display: flex;
    flex: 0 0 180px;
    flex-direction: column;
    gap: 2px;
    padding: 8px;
    border-right: 1px solid var(--border);
    overflow-y: auto;
  }
  .tabs button {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    text-align: left;
  }
  .tabs button:hover {
    background: var(--hover);
  }
  .tabs button.on {
    background: var(--surface-strong);
    font-weight: 600;
  }
  .content {
    flex: 1 1 auto;
    padding: 8px 20px 20px;
    overflow-y: auto;
  }
  :global(.content h2) {
    margin: 20px 0 4px;
    font-size: 14px;
  }
  :global(.content h2:first-child) {
    margin-top: 12px;
  }
  :global(.content .divided > * + *) {
    border-top: 1px solid var(--border);
  }
  @media (max-width: 640px) {
    .tabs {
      flex-basis: 60px;
    }
  }
</style>
