import { describe, expect, it } from 'vitest';
import { MAX_RESTORE_ATTEMPTS, restoreBackoffSecs } from './restore';

describe('restore backoff', () => {
  it('grows, then settles', () => {
    const secs = Array.from({ length: MAX_RESTORE_ATTEMPTS }, (_, i) => restoreBackoffSecs(i + 1));
    expect(secs).toEqual([2, 4, 8, 16, 30, 30]);
  });

  it('never hammers or stalls, however long the outage', () => {
    // Every wait is long enough not to spin on a dead network, and short
    // enough that a returning connection is picked up promptly.
    for (let attempt = 1; attempt <= 100; attempt++) {
      const secs = restoreBackoffSecs(attempt);
      expect(secs).toBeGreaterThanOrEqual(2);
      expect(secs).toBeLessThanOrEqual(30);
    }
  });
});
