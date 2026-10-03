import type { RecoveryEnableStage, SasState } from '../bindings';
import { fire, request } from '../requests';
import { backend } from '../api';
import { session } from './session.svelte';

export type CrossSigning =
  | { state: 'idle' }
  /** The homeserver wants interactive auth in a browser before it will let us set up. */
  | { state: 'needs-fallback'; url: string }
  | { state: 'failed'; reason: string };

/**
 * End-to-end encryption housekeeping: cross-signing bootstrap, SAS device
 * verification, and key backup / recovery.
 *
 * Backup is deliberately never pushed at the user as a prompt — the
 * "recovery needed" signals are recorded here and shown, calmly, in
 * Settings → Security; only genuinely time-sensitive flows (an incoming
 * verification request, the cross-signing browser step) surface as banners.
 */
class SecurityStore {
  crossSigning = $state.raw<CrossSigning>({ state: 'idle' });
  sas = $state.raw<SasState | null>(null);

  /** Secret storage exists but this device lacks some secrets. */
  needsRecovery = $state(false);
  /** No recovery / secret storage at all yet. */
  setupNeeded = $state(false);
  recoveryStage = $state<RecoveryEnableStage | null>(null);
  /** A freshly generated key, shown exactly once until the user confirms they saved it. */
  recoveryKeyToConfirm = $state<string | null>(null);
  recoveryError = $state('');
  restoreBusy = $state(false);
  restored = $state(false);

  // --- cross-signing ---
  openFallback(): void {
    if (this.crossSigning.state === 'needs-fallback') void backend.openExternal(this.crossSigning.url);
  }

  retryCrossSigning(): void {
    this.crossSigning = { state: 'idle' };
    fire({ type: 'RetryCrossSigningBootstrap' });
  }

  dismissCrossSigning(): void {
    this.crossSigning = { state: 'idle' };
  }

  // --- verification ---
  /** A blank id verifies this device (against another of your sessions). */
  startVerification(userId: string): void {
    const id = userId.trim() || session.info?.user_id;
    if (id) fire({ type: 'StartVerification', data: { user_id: id } });
  }

  accept(): void {
    fire({ type: 'AcceptVerificationRequest' });
  }

  /** Declining (or dismissing a finished flow) ends it on our side too. */
  decline(): void {
    this.sas = null;
    fire({ type: 'VerificationCancel' });
  }

  confirmMatch(): void {
    fire({ type: 'ConfirmSasMatch' });
  }

  /** A mismatch ends the flow; show the cancelled card (with a Dismiss) rather than dead buttons. */
  rejectMatch(): void {
    this.sas = { type: 'Cancelled', data: { reason: 'you indicated the emoji did not match' } };
    fire({ type: 'RejectSasMatch' });
  }

  // --- recovery ---
  async enableRecovery(passphrase: string | null): Promise<void> {
    this.recoveryError = '';
    this.recoveryStage = 'Starting';
    try {
      await request((request_id) => ({
        type: 'EnableRecovery',
        data: { passphrase: passphrase && passphrase.length > 0 ? passphrase : null, request_id },
      }));
    } catch (error) {
      this.recoveryStage = null;
      this.recoveryError = String(error);
    }
  }

  async restoreFromBackup(recoveryKey: string): Promise<boolean> {
    const key = recoveryKey.trim();
    if (!key) return false;
    this.recoveryError = '';
    this.restoreBusy = true;
    try {
      await request((request_id) => ({ type: 'RestoreFromBackup', data: { recovery_key: key, request_id } }));
      this.restored = true;
      this.needsRecovery = false;
      return true;
    } catch (error) {
      this.recoveryError = String(error);
      return false;
    } finally {
      this.restoreBusy = false;
    }
  }

  keySaved(): void {
    this.recoveryKeyToConfirm = null;
    this.setupNeeded = false;
  }

  reset(): void {
    this.crossSigning = { state: 'idle' };
    this.sas = null;
    this.needsRecovery = false;
    this.setupNeeded = false;
    this.recoveryStage = null;
    this.recoveryKeyToConfirm = null;
    this.recoveryError = '';
    this.restoreBusy = false;
    this.restored = false;
  }
}

export const security = new SecurityStore();

export const RECOVERY_STAGE_LABEL: Record<RecoveryEnableStage, string> = {
  Starting: 'Starting…',
  CreatingBackup: 'Creating backup…',
  CreatingRecoveryKey: 'Creating recovery key…',
  BackingUp: 'Backing up room keys…',
};
