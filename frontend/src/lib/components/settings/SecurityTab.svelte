<script lang="ts">
  import { RECOVERY_STAGE_LABEL, security } from '../../stores/security.svelte';
  import { settings } from '../../stores/settings.svelte';
  import SettingToggle from '../SettingToggle.svelte';

  const value = $derived(settings.value);

  let verifyId = $state('');
  let recoveryInput = $state('');
  let usePassphrase = $state(false);
  let passphrase = $state('');
  let copied = $state(false);

  async function unlock(event: SubmitEvent) {
    event.preventDefault();
    if (await security.restoreFromBackup(recoveryInput)) recoveryInput = '';
  }

  async function copyKey() {
    if (!security.recoveryKeyToConfirm) return;
    await navigator.clipboard.writeText(security.recoveryKeyToConfirm);
    copied = true;
    setTimeout(() => (copied = false), 2000);
  }
</script>

{#if value}
  <h2>New conversations</h2>
  <p class="muted small">Encryption is fixed when a room is created, so these only affect conversations you start from now on.</p>
  <div class="divided">
    <SettingToggle title="Encrypt new direct messages" hint="End-to-end encrypt DMs this client creates." checked={value.encryption.encrypt_direct_messages} onchange={(on) => settings.update((d) => (d.encryption.encrypt_direct_messages = on))} />
    <SettingToggle title="Encrypt new rooms" hint="End-to-end encrypt rooms this client creates." checked={value.encryption.encrypt_rooms} onchange={(on) => settings.update((d) => (d.encryption.encrypt_rooms = on))} />
  </div>

  <h2>Device verification</h2>
  {#if security.sas}
    <p class="muted small">A verification is in progress — see the banner at the top of the window.</p>
  {:else}
    <p class="muted small">Verify another session or a contact so they show as trusted. Leave the field blank to verify this device.</p>
    <form class="row" onsubmit={(e) => { e.preventDefault(); security.startVerification(verifyId); }}>
      <input class="input" placeholder="@user:server (blank = verify this device)" bind:value={verifyId} spellcheck="false" aria-label="User to verify" />
      <button class="btn primary" type="submit">Verify</button>
    </form>
  {/if}

  <h2>Encrypted message backup</h2>
  {#if security.recoveryKeyToConfirm}
    <div class="card key">
      <strong>Save your recovery key</strong>
      <p class="muted small">This is the only time this key will be shown. Store it somewhere safe — you'll need it to read your encrypted message history on a new device.</p>
      <code class="recovery">{security.recoveryKeyToConfirm}</code>
      <div class="row">
        <button class="btn small" onclick={copyKey}>{copied ? 'Copied' : 'Copy'}</button>
        <button class="btn primary small" onclick={() => security.keySaved()}>I've saved it</button>
      </div>
    </div>
  {:else}
    <p class="muted small">
      ThornyChat never prompts for a recovery key on its own. Enter yours here when you want to read older encrypted messages on this device.
      {#if security.needsRecovery}<strong>This device is missing some keys — your recovery key would restore them.</strong>{/if}
    </p>
    <form class="row" onsubmit={unlock}>
      <input class="input" type="password" placeholder="Recovery key or passphrase" bind:value={recoveryInput} autocomplete="off" aria-label="Recovery key or passphrase" />
      <button class="btn primary" type="submit" disabled={security.restoreBusy || !recoveryInput.trim()}>Unlock</button>
    </form>
    {#if security.restored}<p class="success small" role="status">Backup restored. Older messages will decrypt as their keys arrive.</p>{/if}

    <div class="card setup">
      <p class="muted small">{security.setupNeeded ? "You don't have a backup yet." : "Don't have a recovery key?"} Set up a new backup so future messages stay recoverable on your other devices.</p>
      {#if usePassphrase}
        <input class="input" type="password" placeholder="Optional passphrase" bind:value={passphrase} autocomplete="new-password" aria-label="Passphrase" />
      {/if}
      <div class="row">
        <button class="btn" disabled={security.recoveryStage !== null} onclick={() => security.enableRecovery(usePassphrase ? passphrase : null)}>
          {security.recoveryStage ? RECOVERY_STAGE_LABEL[security.recoveryStage] : 'Set up a new backup'}
        </button>
        <button class="btn ghost small" onclick={() => { usePassphrase = !usePassphrase; passphrase = ''; }}>{usePassphrase ? 'No passphrase' : 'Use a passphrase'}</button>
      </div>
    </div>
  {/if}
  {#if security.recoveryError}<p class="error-text small" role="alert">{security.recoveryError}</p>{/if}
{/if}

<style>
  .key,
  .setup {
    display: grid;
    gap: 8px;
    margin-top: 12px;
  }
  .recovery {
    padding: 10px 12px;
    border-radius: var(--radius-sm);
    background: var(--bg);
    font: 14px ui-monospace, Consolas, monospace;
    letter-spacing: 0.04em;
    overflow-wrap: anywhere;
    user-select: all;
  }
  .success {
    color: var(--success);
  }
</style>
