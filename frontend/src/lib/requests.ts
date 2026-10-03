// Correlates commands with their outcomes.
//
// Every command that expects an answer carries a `request_id`; the worker
// answers with `CommandSucceeded` / `CommandFailed` for that id, or — for the
// few commands that return data — with a dedicated result event that carries
// it. `request()` hands back a promise for whichever arrives.

import type { ClientCommand, ClientEvent } from './bindings';
import { backend } from './api';

/**
 * A command the worker refused or couldn't carry out. Its string form is the
 * bare message ("invalid username or password"), not "Error: …", so callers can
 * put `String(error)` straight on screen — the same shape a failed Tauri
 * `invoke` already has, where the rejection is a plain string.
 */
export class CommandError extends Error {
  override toString(): string {
    return this.message;
  }
}

type ResultType = 'UserSearchResults' | 'SearchResults' | 'SpaceHierarchyFetched';
type EventOf<T extends ClientEvent['type']> = Extract<ClientEvent, { type: T }>;
type ResultData<T extends ResultType> = EventOf<T> extends { data: infer D } ? D : never;

interface Pending {
  expect: ResultType | null;
  resolve: (value: unknown) => void;
  reject: (error: CommandError) => void;
}

const pending = new Map<string, Pending>();

export function newRequestId(): string {
  if (typeof crypto.randomUUID === 'function') return crypto.randomUUID();
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6]! & 0x0f) | 0x40;
  bytes[8] = (bytes[8]! & 0x3f) | 0x80;
  const hex = [...bytes].map((b) => b.toString(16).padStart(2, '0')).join('');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

/** Sends a command that ends in `CommandSucceeded` / `CommandFailed`. */
export function request(build: (requestId: string) => ClientCommand): Promise<void> {
  return send(build, null) as Promise<void>;
}

/** Sends a command whose answer is a result event carrying the request id. */
export function requestResult<T extends ResultType>(
  build: (requestId: string) => ClientCommand,
  resultType: T,
): Promise<ResultData<T>> {
  return send(build, resultType) as Promise<ResultData<T>>;
}

function send(build: (requestId: string) => ClientCommand, expect: ResultType | null): Promise<unknown> {
  const requestId = newRequestId();
  return new Promise((resolve, reject) => {
    pending.set(requestId, { expect, resolve, reject });
    backend.dispatch(build(requestId)).catch((error: unknown) => {
      pending.delete(requestId);
      reject(new CommandError(String(error)));
    });
  });
}

/**
 * For commands that don't go through `dispatch` (an attachment upload carries
 * a raw body), the outcome still arrives as `CommandSucceeded`/`CommandFailed`
 * for the id the caller chose. Register interest first, then send; call
 * `abort` if the send itself fails so the entry doesn't leak.
 */
export function expectOutcome(requestId: string): { outcome: Promise<void>; abort: (error: CommandError) => void } {
  let abort: (error: CommandError) => void = () => {};
  const outcome = new Promise<void>((resolve, reject) => {
    pending.set(requestId, { expect: null, resolve: () => resolve(), reject });
    abort = (error) => {
      if (pending.delete(requestId)) reject(error);
    };
  });
  return { outcome, abort };
}

/** Sends a command nobody waits on (typing, mark-read, open/close room…). */
export function fire(command: ClientCommand): void {
  backend.dispatch(command).catch((error: unknown) => console.warn('command failed to dispatch', command.type, error));
}

/** Called by the event router for every event; settles the matching request. */
export function settle(event: ClientEvent): void {
  switch (event.type) {
    case 'CommandSucceeded': {
      const entry = pending.get(event.data.request_id);
      if (entry && entry.expect === null) {
        pending.delete(event.data.request_id);
        entry.resolve(undefined);
      }
      break;
    }
    case 'CommandFailed': {
      const entry = pending.get(event.data.request_id);
      if (entry) {
        pending.delete(event.data.request_id);
        entry.reject(new CommandError(event.data.error));
      }
      break;
    }
    case 'UserSearchResults':
    case 'SearchResults':
    case 'SpaceHierarchyFetched': {
      const entry = pending.get(event.data.request_id);
      if (entry && entry.expect === event.type) {
        pending.delete(event.data.request_id);
        entry.resolve(event.data);
      }
      break;
    }
    default:
      break;
  }
}

/** Fails everything in flight (sign-out: the worker that would answer is gone). */
export function rejectAll(reason: string): void {
  for (const [id, entry] of pending) {
    pending.delete(id);
    entry.reject(new CommandError(reason));
  }
}
