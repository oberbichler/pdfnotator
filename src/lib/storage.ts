/**
 * Per-viewer preferences in `localStorage`. Storage may be unavailable
 * (private mode, blocked site data), so every access is guarded.
 */
const PREFIX = "pdfnotator.";

export function load(key: string): string | null {
  try {
    return localStorage.getItem(PREFIX + key);
  } catch {
    return null;
  }
}

export function save(key: string, value: string | null): void {
  try {
    if (value === null) localStorage.removeItem(PREFIX + key);
    else localStorage.setItem(PREFIX + key, value);
  } catch {
    // preferences are a convenience; ignore failures
  }
}

export function loadBool(key: string, fallback: boolean): boolean {
  const v = load(key);
  return v === null ? fallback : v === "1";
}

export function saveBool(key: string, value: boolean): void {
  save(key, value ? "1" : "0");
}
