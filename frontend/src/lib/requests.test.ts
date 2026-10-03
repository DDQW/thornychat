import { describe, expect, it } from 'vitest';
import { CommandError, newRequestId, rejectAll, request, requestResult, settle } from './requests';

// The mock backend ignores commands it has no handler for, so a request stays
// pending until the test settles it — exactly like a worker that hasn't answered.
function start() {
  let id = '';
  const promise = request((request_id) => {
    id = request_id;
    return { type: 'RefreshEmojiPacks', data: { room_id: '!r:s' } };
  });
  return { promise, id: () => id };
}

describe('request correlation', () => {
  it('resolves when the worker reports success for that id', async () => {
    const { promise, id } = start();
    settle({ type: 'CommandSucceeded', data: { request_id: id() } });
    await expect(promise).resolves.toBeUndefined();
  });

  it('rejects with the worker’s bare message, without an "Error:" prefix', async () => {
    const { promise, id } = start();
    settle({ type: 'CommandFailed', data: { request_id: id(), error: 'invalid username or password' } });
    const error = await promise.then(
      () => null,
      (e: unknown) => e,
    );
    expect(error).toBeInstanceOf(Error);
    expect(error).toBeInstanceOf(CommandError);
    expect(String(error)).toBe('invalid username or password');
  });

  it('ignores outcomes for other ids', async () => {
    const { promise, id } = start();
    settle({ type: 'CommandSucceeded', data: { request_id: 'someone-else' } });
    let settled = false;
    void promise.then(() => (settled = true));
    await Promise.resolve();
    expect(settled).toBe(false);
    settle({ type: 'CommandSucceeded', data: { request_id: id() } });
    await promise;
  });

  it('a result-carrying request resolves with the result event, not the generic success', async () => {
    let id = '';
    const promise = requestResult((request_id) => {
      id = request_id;
      return { type: 'FetchSpaceHierarchy', data: { space_id: '!s:s', from: null, request_id } };
    }, 'SpaceHierarchyFetched');
    // A stray CommandSucceeded for the same id must not stand in for the data.
    settle({ type: 'CommandSucceeded', data: { request_id: id } });
    let early = false;
    void promise.then(() => (early = true));
    await Promise.resolve();
    expect(early).toBe(false);

    settle({ type: 'SpaceHierarchyFetched', data: { request_id: id, space_id: '!s:s', children: [], next_batch: 'tok' } });
    await expect(promise).resolves.toMatchObject({ next_batch: 'tok' });
  });

  it('signing out fails everything still in flight', async () => {
    const { promise } = start();
    rejectAll('signed out');
    await expect(promise).rejects.toThrow('signed out');
  });

  it('a settled request cannot be settled twice', async () => {
    const { promise, id } = start();
    settle({ type: 'CommandSucceeded', data: { request_id: id() } });
    await promise;
    // Would throw or double-resolve if the entry were still registered.
    expect(() => settle({ type: 'CommandFailed', data: { request_id: id(), error: 'late' } })).not.toThrow();
  });
});

describe('newRequestId', () => {
  it('produces distinct v4 UUIDs the Rust side can parse', () => {
    const ids = new Set(Array.from({ length: 50 }, newRequestId));
    expect(ids.size).toBe(50);
    for (const id of ids) expect(id).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
  });
});
