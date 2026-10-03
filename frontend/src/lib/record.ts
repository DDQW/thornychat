// Immutable updates for the keyed maps the stores hold as `$state.raw`
// (replacing the whole object is what notifies readers).

/**
 * `next` with every element that is field-for-field equal to its counterpart
 * in `previous` (matched by `key`) replaced by that previous object — and
 * `previous` itself when nothing changed at all. For lists the worker re-sends
 * whole (freshly deserialized, so every object is new) when one entry moved:
 * keyed `{#each}` blocks then re-render only what actually changed.
 * Elements are compared one level deep, which is all the flat summaries need.
 */
export function reuseUnchanged<T extends object>(next: readonly T[], previous: readonly T[], key: (item: T) => string): T[] {
  if (previous.length === 0) return next as T[];
  const byKey = new Map(previous.map((item) => [key(item), item]));
  let identical = next.length === previous.length;
  const merged = next.map((item, index) => {
    const old = byKey.get(key(item));
    const same = old !== undefined && shallowEqual(old, item);
    if (!same || previous[index] !== old) identical = false;
    return same ? old : item;
  });
  return identical ? (previous as T[]) : merged;
}

function shallowEqual(a: object, b: object): boolean {
  const left = a as Record<string, unknown>;
  const right = b as Record<string, unknown>;
  const keys = Object.keys(left);
  return keys.length === Object.keys(right).length && keys.every((name) => Object.is(left[name], right[name]));
}

/** Returns a copy of `record` with `key` set to `value`, or removed when `value` is undefined. */
export function withKey<T>(record: Readonly<Record<string, T>>, key: string, value: T | undefined): Record<string, T> {
  const next = { ...record };
  if (value === undefined) delete next[key];
  else next[key] = value;
  return next;
}
