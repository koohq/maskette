/**
 * localStorage access for small UI preferences. Storage can be missing or throw
 * (privacy-restricted contexts, tests), so reads fall back to null and writes
 * are dropped.
 *
 * Keys used to carry the names of the projects Maskette came from
 * (`kigcraft.*`, `kig-preview.*`). Reading with the old key as `legacyKey`
 * moves a saved value to the new key the first time, so nobody's settings reset.
 */
export function readStored(key: string, legacyKey?: string | undefined): string | null {
  if (typeof window === "undefined") return null;
  let storage: Storage;
  let current: string | null;
  try {
    storage = window.localStorage;
    current = storage.getItem(key);
  } catch {
    return null;
  }
  if (current !== null || legacyKey === undefined) return current;

  let legacy: string | null;
  try {
    legacy = storage.getItem(legacyKey);
  } catch {
    return null;
  }
  if (legacy === null) return null;
  try {
    storage.setItem(key, legacy);
    storage.removeItem(legacyKey);
  } catch {
    // Keep using the old value this time; the move is tried again next read.
  }
  return legacy;
}

export function writeStored(key: string, value: string): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // Storage can be unavailable in privacy-restricted browser contexts.
  }
}
