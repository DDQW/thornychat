<script lang="ts">
  import { onMount } from 'svelte';
  import { backend } from '../../api';
  import type { LogLevel } from '../../types';
  import { session } from '../../stores/session.svelte';
  import { settings } from '../../stores/settings.svelte';
  import { spelling } from '../../stores/spelling.svelte';
  import { ui } from '../../stores/ui.svelte';
  import SettingToggle from '../SettingToggle.svelte';

  const value = $derived(settings.value);

  let confirmSignOut = $state(false);
  let autostart = $state(false);
  let autostartError = $state('');
  let logNote = $state('');
  let confirmClear = $state(false);
  let logBusy = $state(false);

  const LEVELS: { id: LogLevel; label: string }[] = [
    { id: 'off', label: 'Off — write nothing' },
    { id: 'error', label: 'Errors only' },
    { id: 'warn', label: 'Warnings and errors' },
    { id: 'info', label: 'Normal (info)' },
    { id: 'debug', label: 'Detailed (debug)' },
    { id: 'trace', label: 'Everything (trace)' },
  ];

  // The real registry state, not an assumption: it may have been removed by
  // hand (or an uninstaller) since this was last open.
  onMount(() => {
    spelling.loadStatus();
    backend.autostartEnabled().then(
      (enabled) => (autostart = enabled),
      () => {},
    );
  });

  async function setAutostart(enabled: boolean) {
    autostartError = '';
    try {
      await backend.setAutostart(enabled);
      autostart = enabled;
    } catch (error) {
      autostartError = String(error);
    }
  }

  async function copyLog() {
    logBusy = true;
    logNote = '';
    try {
      const text = await backend.readLog();
      await navigator.clipboard.writeText(text);
      logNote = `Copied ${text.length.toLocaleString()} characters to the clipboard.`;
    } catch (error) {
      logNote = `Couldn't copy the log: ${String(error)}`;
    } finally {
      logBusy = false;
    }
  }

  async function clearLogs() {
    confirmClear = false;
    logBusy = true;
    try {
      logNote = await backend.clearLogs();
    } catch (error) {
      logNote = `Couldn't delete the logs: ${String(error)}`;
    } finally {
      logBusy = false;
    }
  }
</script>

{#if value}
  <h2>Account</h2>
  <dl class="info">
    <dt>User ID</dt><dd>{session.info?.user_id ?? '—'}</dd>
    <dt>Homeserver</dt><dd>{session.info?.homeserver ?? '—'}</dd>
    <dt>Device ID</dt><dd>{session.info?.device_id ?? '—'}</dd>
  </dl>
  {#if confirmSignOut}
    <div class="row">
      <span>Sign out of {session.info?.user_id ?? 'this account'}?</span>
      <button class="btn danger small" onclick={() => session.logout()} disabled={session.signingOut}>Yes, sign out</button>
      <button class="btn small" onclick={() => (confirmSignOut = false)}>No</button>
    </div>
  {:else}
    <button class="btn small" onclick={() => (confirmSignOut = true)}>Sign out</button>
  {/if}

  <h2>Startup</h2>
  <SettingToggle title="Start with Windows" hint="Launch ThornyChat minimized when you sign in to Windows." checked={autostart} onchange={setAutostart} />
  {#if autostartError}<p class="error-text small">{autostartError}</p>{/if}

  <h2>Timeline</h2>
  <SettingToggle
    title="Show join, leave and invite messages"
    hint="Membership changes as compact lines in the timeline. Turn off in rooms bridged to IRC, where join/leave churn is constant."
    checked={value.chat.show_membership_events}
    onchange={(on) => settings.update((d) => (d.chat.show_membership_events = on))}
  />

  <h2>Spelling</h2>
  <SettingToggle
    title="Check spelling"
    hint="Underline misspelled words as you type, using the Windows speller and your personal dictionary. Click into a marked word, or right-click it, for suggestions."
    checked={value.spellcheck.enabled}
    onchange={(on) => settings.update((d) => (d.spellcheck.enabled = on))}
  />
  <SettingToggle
    title="Fix obvious typos as you type"
    hint="When you finish a word, a fix the speller is sure of replaces it (“teh” becomes “the”). Press Backspace right after to undo it; that word is then left alone for the rest of the message."
    checked={value.spellcheck.autocorrect}
    disabled={!value.spellcheck.enabled}
    onchange={(on) => settings.update((d) => (d.spellcheck.autocorrect = on))}
  />
  {#if value.spellcheck.enabled && spelling.status}
    <p class="muted small">
      {spelling.status.available ? `Dictionary: ${spelling.status.language ?? 'the Windows default'}, from your Windows language settings.` : "The Windows speller isn't available on this PC, so nothing is checked."}
    </p>
  {/if}

  <h2>Diagnostics</h2>
  <div class="setting">
    <div class="what">
      <label for="log-level">How much to log</label>
      <div class="hint">Takes effect the next time you start ThornyChat. “Off” writes nothing at all.</div>
    </div>
    <select id="log-level" class="select fit" value={value.log.level} onchange={(e) => settings.update((d) => (d.log.level = e.currentTarget.value as LogLevel))}>
      {#each LEVELS as level (level.id)}<option value={level.id}>{level.label}</option>{/each}
    </select>
  </div>
  <div class="row wrap">
    <button class="btn small" onclick={copyLog} disabled={logBusy || !session.app?.logging_enabled}>Copy log to clipboard</button>
    {#if confirmClear}
      <span>Delete the stored log files? This can't be undone.</span>
      <button class="btn danger small" onclick={clearLogs} disabled={logBusy}>Delete</button>
      <button class="btn small" onclick={() => (confirmClear = false)}>Cancel</button>
    {:else}
      <button class="btn small" onclick={() => (confirmClear = true)} disabled={logBusy}>Delete log files…</button>
    {/if}
  </div>
  {#if logNote}<p class="muted small" role="status">{logNote}</p>{/if}

  <h2>About</h2>
  <p class="muted small">ThornyChat {session.app?.version ?? ''}{session.app?.profile && session.app.profile !== 'default' ? ` — profile “${session.app.profile}”` : ''}. Licensed GPL-3.0-or-later.</p>
  <button class="btn small ghost" onclick={() => ui.openSettings('manual')}>Read the manual</button>
{/if}

<style>
  .info {
    display: grid;
    grid-template-columns: 110px 1fr;
    gap: 4px 12px;
    margin: 8px 0 12px;
  }
  dt {
    color: var(--muted);
    font-size: 12px;
  }
  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .fit {
    width: auto;
    min-width: 200px;
  }
  .wrap {
    flex-wrap: wrap;
  }
</style>
