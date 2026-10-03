// Immutable updates for the keyed maps the stores hold as `$state.raw`
// (replacing the whole object is what notifies readers).

/** Returns a copy of `record` with `key` set to `value`, or removed when `value` is undefined. */
export function withKey<T>(record: Readonly<Record<string, T>>, key: string, value: T | undefined): Record<string, T> {
  const next = { ...record };
  if (value === undefined) delete next[key];
  else next[key] = value;
  return next;
}
