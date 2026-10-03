// How long to wait before re-trying to restore a saved session when the
// homeserver can't be reached. An unreachable server is not a reason to dump
// someone on a login form: the saved session is still on disk, and launching
// before the network is up — autostart at boot, or right after a resume — is
// routine.

/** Roughly two minutes of trying at the backoff below. */
export const MAX_RESTORE_ATTEMPTS = 6;

/** Backoff before restore attempt `attempt` (1-based): 2s, 4s, 8s, 16s, 30s, 30s. */
export function restoreBackoffSecs(attempt: number): number {
  return Math.min(2 ** Math.min(attempt, 5), 30);
}
