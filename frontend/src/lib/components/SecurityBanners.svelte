<script lang="ts">
  import { friendlyUserId } from '../format';
  import { security } from '../stores/security.svelte';
  import { session } from '../stores/session.svelte';
  import Icon from './Icon.svelte';

  // Only genuinely interactive, time-sensitive flows live here: a session that
  // can't continue, the cross-signing browser step, and a verification someone
  // is waiting on. Backup and recovery are never pushed at the user — they
  // live, calmly, in Settings → Security.
  const sas = $derived(security.sas);
  const cross = $derived(security.crossSigning);
</script>

<div class="banners">
  {#if session.expired}
    <div class="banner danger" role="alert">
      <Icon name="info" />
      <span class="grow">Your session has expired. Sign in again to keep chatting.</span>
      <button class="btn small" onclick={() => session.logout()}>Sign in again</button>
    </div>
  {/if}

  {#if cross.state === 'failed'}
    <div class="banner danger" role="alert">
      <Icon name="shield-alert" />
      <div class="grow"><strong>Couldn't set up secure messaging</strong><div class="small">{cross.reason}</div></div>
      <button class="btn small" onclick={() => security.dismissCrossSigning()}>Dismiss</button>
    </div>
  {:else if cross.state === 'needs-fallback'}
    <div class="banner" role="status">
      <Icon name="shield" />
      <div class="grow"><strong>Finish setting up secure messaging</strong><div class="small">Your homeserver needs you to confirm this in your browser.</div></div>
      <button class="btn small" onclick={() => security.openFallback()}>Open browser</button>
      <button class="btn primary small" onclick={() => security.retryCrossSigning()}>I've done that, continue</button>
    </div>
  {/if}

  {#if sas}
    <div class="banner" role="status">
      <Icon name="shield" />
      <div class="grow">
        {#if sas.type === 'RequestReceived'}
          <strong>Verification request</strong>
          <div class="small">{friendlyUserId(sas.data.from_user_id)} ({sas.data.from_user_id}) wants to verify.</div>
        {:else if sas.type === 'RequestSent'}
          <strong>Waiting for the other device…</strong>
        {:else if sas.type === 'Ready'}
          <strong>Starting verification…</strong>
        {:else if sas.type === 'EmojisReady'}
          <strong>Do these emoji match on both devices?</strong>
          <div class="emoji">
            {#each sas.data as [glyph, label] (label)}<span><span class="g">{glyph}</span><span class="small muted">{label}</span></span>{/each}
          </div>
        {:else if sas.type === 'WaitingForOtherPartyConfirmation'}
          <strong>Waiting for the other device to confirm…</strong>
        {:else if sas.type === 'Done'}
          <strong>Verification complete</strong>
        {:else}
          <strong>Verification cancelled</strong>
          <div class="small">{sas.data.reason}</div>
        {/if}
      </div>
      {#if sas.type === 'RequestReceived'}
        <button class="btn primary small" onclick={() => security.accept()}>Accept</button>
        <button class="btn small" onclick={() => security.decline()}>Decline</button>
      {:else if sas.type === 'EmojisReady'}
        <button class="btn primary small" onclick={() => security.confirmMatch()}>They match</button>
        <button class="btn small" onclick={() => security.rejectMatch()}>They don't match</button>
      {:else if sas.type === 'Done' || sas.type === 'Cancelled'}
        <button class="btn small" onclick={() => security.decline()}>Dismiss</button>
      {:else}
        <button class="btn small" onclick={() => security.decline()}>Cancel</button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .banners:empty {
    display: none;
  }
  .banner {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--border);
    background: var(--accent-wash);
  }
  .banner.danger {
    background: var(--danger-wash);
    color: var(--danger);
  }
  .banner.danger .grow {
    color: var(--text);
  }
  .emoji {
    display: flex;
    flex-wrap: wrap;
    gap: 14px;
    margin-top: 6px;
  }
  .emoji > span {
    display: grid;
    justify-items: center;
  }
  .g {
    font-size: 26px;
    line-height: 1.2;
  }
</style>
