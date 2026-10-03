// Per-viewer conveniences kept in the browser's own storage (a collapsed
// section, a draft). Never anything that must survive a profile wipe or be
// read by the Rust side. Every access is guarded: storage can be unavailable or
// throw, and the page has to render correctly without it.

export function readPref<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(`thornychat:${key}`);
    return raw === null ? fallback : (JSON.parse(raw) as T);
  } catch {
    return fallback;
  }
}

export function writePref(key: string, value: unknown): void {
  try {
    localStorage.setItem(`thornychat:${key}`, JSON.stringify(value));
  } catch {
    // Not remembering is fine.
  }
}
