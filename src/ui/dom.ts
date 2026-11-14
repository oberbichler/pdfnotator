/** Small DOM helpers. */

export const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);

/** Element by id (throws early when the markup and code disagree). */
export function byId<T extends HTMLElement = HTMLElement>(id: string): T {
  const el = document.getElementById(id);
  if (!el) throw new Error(`#${id} missing in index.html`);
  return el as T;
}

/** Create an element with optional class and text. */
export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className?: string,
  text?: string,
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  if (className) el.className = className;
  if (text !== undefined) el.textContent = text;
  return el;
}

/** Render a shortcut like `Mod+Shift+O` for the current platform. */
export function shortcut(spec: string): string {
  return isMac
    ? spec
        .replace(/Mod\+/g, "⌘")
        .replace(/Shift\+/g, "⇧")
        .replace(/Alt\+/g, "⌥")
    : spec.replace(/Mod\+/g, "Ctrl+");
}

/** Resize a textarea to its content (up to `max` px). */
export function autoGrow(t: HTMLTextAreaElement, max = 180): void {
  t.style.height = "auto";
  t.style.height = `${Math.min(t.scrollHeight, max)}px`;
}

/** True while focus is in an editable field. */
export function isEditing(target: EventTarget | null): boolean {
  return target instanceof Element && !!target.closest("textarea, input, [contenteditable=true]");
}

export const formatTime = (ts: number): string =>
  new Date(ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
