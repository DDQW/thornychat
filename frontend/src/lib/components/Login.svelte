<script lang="ts">
  import { backend } from '../api';
  import { session } from '../stores/session.svelte';
  import type { LoginFlows } from '../types';

  // A two-step wizard, like Element: first the homeserver, whose advertised
  // login flows decide what the second step shows — a password form, one
  // button per SSO provider, or both. Many homeservers delegate auth and only
  // support SSO, and never show a password field at all.
  let homeserver = $state('');
  let username = $state('');
  let password = $state('');
  let flows = $state<LoginFlows | null>(null);
  let discovering = $state(false);
  let error = $state('');

  const busy = $derived(discovering || session.loginBusy);
  const shownError = $derived(error || session.loginError);

  async function discover(event: SubmitEvent) {
    event.preventDefault();
    const server = homeserver.trim();
    if (!server) {
      error = 'Enter a homeserver to continue.';
      return;
    }
    error = '';
    session.loginError = '';
    discovering = true;
    try {
      flows = await backend.discoverHomeserver(server);
    } catch (e) {
      error = String(e);
    } finally {
      discovering = false;
    }
  }

  async function signIn(event: SubmitEvent) {
    event.preventDefault();
    if (!username.trim()) {
      error = 'Username is required.';
      return;
    }
    error = '';
    await session.loginPassword(homeserver.trim(), username.trim(), password);
    // Never keep the password in memory longer than the attempt.
    password = '';
  }

  function sso(providerId: string | null) {
    error = '';
    void session.loginSso(homeserver.trim(), providerId);
  }

  function back() {
    flows = null;
    error = '';
    session.loginError = '';
  }
</script>

<main class="login">
  <div class="panel card">
    <h1>Sign in to ThornyChat</h1>

    {#if session.note}
      <p class="muted status" role="status"><span class="spinner"></span> {session.note}</p>
    {/if}

    {#if !flows}
      <p class="muted">Enter your homeserver to sign in</p>
      <form onsubmit={discover}>
        <div class="field">
          <label for="homeserver">Homeserver</label>
          <!-- svelte-ignore a11y_autofocus -->
          <input id="homeserver" class="input" placeholder="matrix.org" bind:value={homeserver} autofocus autocomplete="url" inputmode="url" spellcheck="false" disabled={busy} aria-invalid={error ? true : undefined} aria-describedby={shownError ? 'login-error' : undefined} />
        </div>
        <button class="btn primary wide" type="submit" disabled={busy}>Continue</button>
      </form>
      {#if discovering}<p class="muted status" role="status"><span class="spinner"></span> Checking homeserver…</p>{/if}
    {:else}
      <p class="muted">Signing in to <strong>{homeserver.trim()}</strong></p>

      {#if flows.supports_password}
        <form onsubmit={signIn}>
          <div class="field">
            <label for="username">Username</label>
            <!-- svelte-ignore a11y_autofocus -->
            <input id="username" class="input" bind:value={username} autofocus autocomplete="username" autocapitalize="none" spellcheck="false" disabled={busy} aria-describedby={shownError ? 'login-error' : undefined} />
          </div>
          <div class="field">
            <label for="password">Password</label>
            <input id="password" class="input" type="password" bind:value={password} autocomplete="current-password" disabled={busy} aria-describedby={shownError ? 'login-error' : undefined} />
          </div>
          <button class="btn primary wide" type="submit" disabled={busy}>Sign in</button>
        </form>
      {/if}

      {#if flows.supports_sso}
        {#if flows.supports_password}<p class="muted or">or</p>{/if}
        <div class="col">
          {#if flows.sso_providers.length === 0}
            <button class="btn wide" onclick={() => sso(null)} disabled={busy}>Continue with SSO</button>
          {:else}
            {#each flows.sso_providers as provider (provider.id)}
              <button class="btn wide" onclick={() => sso(provider.id)} disabled={busy}>Continue with {provider.name}</button>
            {/each}
          {/if}
        </div>
        {#if session.loginBusy}<p class="muted status" role="status">Finish signing in in your browser…</p>{/if}
      {/if}

      <button class="btn ghost back" onclick={back} disabled={busy}>Back</button>
    {/if}

    {#if shownError}<p class="error-text" id="login-error" role="alert">{shownError}</p>{/if}
  </div>
</main>

<style>
  .login {
    display: grid;
    height: 100%;
    place-items: center;
    padding: 24px;
  }
  .panel {
    width: min(380px, 100%);
    padding: 28px;
  }
  h1 {
    margin: 0 0 4px;
    font-size: 22px;
  }
  .wide {
    width: 100%;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .or {
    margin: 12px 0;
    text-align: center;
  }
  .back {
    margin-top: 12px;
  }
</style>
