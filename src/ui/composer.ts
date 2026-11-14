/**
 * Floating composer for a new comment, with three modes:
 * comment (free text), replace (edit the quoted text) and delete.
 */
import { commitDraft } from "../actions";
import { type MessageKey, onLocaleChange, t } from "../i18n";
import { type CommentDraft, type ComposerMode, composeInstruction, type Screenshot } from "../lib/comments";
import { clamp } from "../lib/geometry";
import { state, subscribe } from "../store";
import { autoGrow, byId, h } from "./dom";
import { icon } from "./icons";

const MODES: readonly ComposerMode[] = ["comment", "replace", "delete"];
const PLACEHOLDER: Record<ComposerMode, MessageKey> = {
  comment: "composer.placeholder.comment",
  replace: "composer.placeholder.replace",
  delete: "composer.placeholder.delete",
};

export interface ComposerOptions {
  /** Short location label, e.g. "p. 3". */
  label: string;
  quote?: string;
  image?: Promise<Screenshot>;
  mode?: ComposerMode;
  /** Called when the composer closes (e.g. remove the rubber band). */
  onClose?: () => void;
}

let root: HTMLElement;
let input: HTMLTextAreaElement;
let context: HTMLElement;
let draft: CommentDraft | null = null;
let mode: ComposerMode = "comment";
let quote = "";
let onClose: (() => void) | null = null;
/** Called after the composer closed, so pending text-layer swaps can run. */
let afterClose: () => void = () => {};

const modeButtons = () => [...root.querySelectorAll<HTMLButtonElement>("#cmp-modes button")];

/** The composer holds text the user typed (not just the prefilled quote). */
export function hasUnsavedText(): boolean {
  const text = input?.value.trim() ?? "";
  return text !== "" && !(mode === "replace" && text === quote.trim());
}

export function isComposerOpen(): boolean {
  return !root.classList.contains("hidden");
}

export function openComposer(anchor: DOMRect, newDraft: CommentDraft, opts: ComposerOptions): void {
  closeComposer();
  draft = newDraft;
  quote = opts.quote ?? "";
  onClose = opts.onClose ?? null;
  const isArea = newDraft.kind === "area";

  byId("cmp-kind").replaceChildren(icon(isArea ? "area" : "quote", 13), document.createTextNode(opts.label));
  // replacing makes no sense for a screenshot region
  root.querySelector('[data-mode="replace"]')?.classList.toggle("hidden", isArea);

  context.replaceChildren();
  if (opts.image) {
    const placeholder = h("div", "shimmer");
    context.append(placeholder);
    opts.image
      .then(({ preview: b64 }) => {
        const img = h("img");
        img.alt = "";
        img.src = `data:image/png;base64,${b64}`;
        placeholder.replaceWith(img);
      })
      .catch(() => placeholder.remove());
  } else if (quote) {
    context.append(h("blockquote", undefined, quote));
  }

  input.value = "";
  root.classList.remove("hidden");
  setMode(opts.mode ?? "comment");
  position(anchor);
}

function position(anchor: DOMRect): void {
  const w = root.offsetWidth;
  const ht = root.offsetHeight;
  let top = anchor.bottom + 8;
  if (top + ht > window.innerHeight - 8) top = Math.max(52, anchor.top - ht - 8);
  root.style.left = `${clamp(anchor.left, 8, window.innerWidth - w - 8)}px`;
  root.style.top = `${top}px`;
}

export function closeComposer(): void {
  if (!root || !isComposerOpen()) return;
  root.classList.add("hidden");
  onClose?.();
  onClose = null;
  draft = null;
  afterClose();
}

function setMode(next: ComposerMode): void {
  const prev = mode;
  mode = next;
  root.classList.remove(...MODES.map((m) => `mode-${m}`));
  root.classList.add(`mode-${next}`);
  for (const b of modeButtons()) b.classList.toggle("active", b.dataset.mode === next);
  if (next === "replace") {
    if (prev !== "replace" || !input.value.trim()) input.value = quote;
  } else if (prev === "replace" && input.value === quote) {
    input.value = "";
  }
  updateLabels();
  autoGrow(input);
  input.focus();
  if (next === "replace") input.select();
}

function updateLabels(): void {
  input.placeholder = t(PLACEHOLDER[mode]);
  const verb =
    mode === "delete" ? "composer.verb.delete" : state.live ? "composer.verb.send" : "composer.verb.add";
  byId("cmp-action").textContent = t(verb);
}

function cycleMode(step: 1 | -1): void {
  const available = modeButtons()
    .filter((b) => !b.classList.contains("hidden"))
    .map((b) => b.dataset.mode as ComposerMode);
  const i = available.indexOf(mode);
  setMode(available[(i + step + available.length) % available.length] ?? "comment");
}

function submit(): void {
  if (!draft) {
    closeComposer();
    return;
  }
  const text = composeInstruction(mode, draft.kind, input.value, quote);
  if (text === null) {
    input.select(); // replace with unchanged text: nothing to send yet
    return;
  }
  const committed = draft;
  draft = null;
  closeComposer();
  window.getSelection()?.removeAllRanges();
  void commitDraft(committed, text);
}

export function initComposer(options: { afterClose: () => void }): void {
  root = byId("composer");
  input = byId<HTMLTextAreaElement>("cmp-input");
  context = byId("cmp-ctx");
  afterClose = options.afterClose;

  for (const b of modeButtons()) b.addEventListener("click", () => setMode(b.dataset.mode as ComposerMode));
  byId("cmp-send").addEventListener("click", submit);
  byId("cmp-close").addEventListener("click", closeComposer);
  input.addEventListener("input", () => autoGrow(input));
  input.addEventListener("keydown", (e) => {
    if (e.isComposing) return;
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      closeComposer();
    } else if (e.key === "Tab") {
      e.preventDefault();
      cycleMode(e.shiftKey ? -1 : 1);
    }
  });
  // clicking outside closes an empty composer; typed text is never discarded silently
  document.addEventListener("mousedown", (e) => {
    if (isComposerOpen() && !root.contains(e.target as Node) && !input.value.trim()) closeComposer();
  });
  subscribe(() => isComposerOpen() && updateLabels());
  onLocaleChange(() => isComposerOpen() && updateLabels());
}
