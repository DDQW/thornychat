import { backend } from '../api';
import type { SyncState } from '../bindings';
import { fire, rejectAll } from '../requests';
import { MAX_RESTORE_ATTEMPTS, restoreBackoffSecs } from '../restore';
import type { AppInfo, OwnProfile, SessionInfo } from '../types';
import { settings } from './settings.svelte';

export type Phase = 'booting' | 'login' | 'main';

class SessionStore {
  phase = $state<Phase>('booting');
  info = $state.raw<SessionInfo | null>(null);
  profile = $state.raw<OwnProfile | null>(null);
  app = $state.raw<AppInfo | null>(null);
  sync = $state.raw<SyncState>({ type: 'Connecting' });
  /** The homeserver rejected the access token; the worker has stopped. */
  expired = $state(false);
  /** Status line on the login screen ("Can't reach the homeserver — retrying in 4s…"). */
  note = $state('');
  loginError = $state('');
  loginBusy = $state(false);
  signingOut = $state(false);

  /** Called by the event router once rooms/timelines should be torn down. */
  onSignedOut: () => void = () => {};
  /** Called after a session starts, to let other stores load per-profile state. */
  onSignedIn: () => void = () => {};

  async boot(): Promise<void> {
    try {
      await settings.load();
      this.app = await backend.appInfo();
    } catch (error) {
      this.loginError = `Couldn't read settings: ${String(error)}`;
    }
    await this.#restore(0);
  }

  async #restore(attempt: number): Promise<void> {
    let outcome;
    try {
      outcome = await backend.sessionRestore();
    } catch (error) {
      this.#toLogin(String(error));
      return;
    }
    switch (outcome.status) {
      case 'started':
        this.note = '';
        this.#adopt(outcome.session);
        break;
      case 'none':
        this.#toLogin('');
        break;
      case 'failed': {
        // An unreachable homeserver is not a reason to dump someone on a login
        // form: the saved session is still on disk. Back off and try again.
        if (outcome.retryable && attempt < MAX_RESTORE_ATTEMPTS) {
          const next = attempt + 1;
          const secs = restoreBackoffSecs(next);
          this.phase = 'login';
          this.note = `Can't reach the homeserver — retrying in ${secs}s…`;
          await new Promise((resolve) => setTimeout(resolve, secs * 1000));
          if (this.phase === 'login') await this.#restore(next);
        } else {
          this.#toLogin(outcome.message);
        }
        break;
      }
    }
  }

  #toLogin(error: string): void {
    this.note = '';
    this.loginError = error;
    this.phase = 'login';
  }

  async loginPassword(homeserver: string, username: string, password: string): Promise<void> {
    await this.#login(() => backend.loginPassword(homeserver, username, password));
  }

  async loginSso(homeserver: string, providerId: string | null): Promise<void> {
    await this.#login(() => backend.loginSso(homeserver, providerId));
  }

  async #login(run: () => Promise<SessionInfo>): Promise<void> {
    this.loginBusy = true;
    this.loginError = '';
    try {
      this.#adopt(await run());
    } catch (error) {
      this.loginError = String(error);
    } finally {
      this.loginBusy = false;
    }
  }

  #adopt(info: SessionInfo): void {
    this.info = info;
    this.expired = false;
    this.signingOut = false;
    this.sync = { type: 'Connecting' };
    this.phase = 'main';
    void this.refreshProfile();
    this.onSignedIn();
  }

  async refreshProfile(): Promise<void> {
    try {
      this.profile = await backend.ownProfile();
    } catch {
      // The row falls back to the user id; the next refresh may succeed.
    }
  }

  logout(): void {
    this.signingOut = true;
    fire({ type: 'Logout' });
  }

  /** The worker confirmed sign-out (or the token died and the user chose to sign in again). */
  handleLoggedOut(): void {
    rejectAll('signed out');
    this.info = null;
    this.profile = null;
    this.signingOut = false;
    this.expired = false;
    this.onSignedOut();
    this.#toLogin('');
  }
}

export const session = new SessionStore();
