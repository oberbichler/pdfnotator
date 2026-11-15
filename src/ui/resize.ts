/**
 * Resizable side panels: a drag handle on the inner edge of the comment
 * sidebar and the navigator sets their width (a CSS variable), remembered per
 * viewer. Double-click resets the default width.
 */
import { onLocaleChange, t } from "../i18n";
import { clamp } from "../lib/geometry";
import { load, save } from "../lib/storage";
import { byId, h } from "./dom";

interface Panel {
  id: string;
  /** CSS variable holding the width. */
  cssVar: string;
  /** Which edge carries the handle (the one facing the document). */
  edge: "left" | "right";
  min: number;
  /** Largest share of the window width. */
  maxShare: number;
}

const PANELS: readonly Panel[] = [
  { id: "sidebar", cssVar: "--sidebar", edge: "left", min: 260, maxShare: 0.5 },
  { id: "navigator", cssVar: "--navigator", edge: "right", min: 150, maxShare: 0.35 },
];

const root = document.documentElement;
const storageKey = (p: Panel) => `width.${p.id}`;
const maxWidth = (p: Panel) => Math.max(p.min, Math.round(window.innerWidth * p.maxShare));

function setWidth(p: Panel, px: number | null): void {
  if (px === null) root.style.removeProperty(p.cssVar);
  else root.style.setProperty(p.cssVar, `${Math.round(clamp(px, p.min, maxWidth(p)))}px`);
}

function attach(p: Panel): void {
  const panel = byId(p.id);
  const handle = h("div", `resize-handle ${p.edge}`);
  handle.setAttribute("role", "separator");
  handle.setAttribute("aria-orientation", "vertical");
  const label = () => {
    handle.title = t("resize.handle");
    handle.setAttribute("aria-label", t("resize.handle"));
  };
  label();
  onLocaleChange(label);
  panel.append(handle);

  handle.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    handle.setPointerCapture(e.pointerId);
    const startX = e.clientX;
    const startW = panel.getBoundingClientRect().width;
    const sign = p.edge === "left" ? -1 : 1; // dragging the left edge leftwards widens
    document.body.classList.add("resizing");
    const move = (ev: PointerEvent) => setWidth(p, startW + sign * (ev.clientX - startX));
    const end = () => {
      handle.removeEventListener("pointermove", move);
      document.body.classList.remove("resizing");
      save(storageKey(p), root.style.getPropertyValue(p.cssVar) || null);
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", end, { once: true });
    handle.addEventListener("pointercancel", end, { once: true });
  });
  handle.addEventListener("dblclick", () => {
    setWidth(p, null);
    save(storageKey(p), null);
  });
}

export function initResize(): void {
  for (const p of PANELS) {
    const saved = Number.parseFloat(load(storageKey(p)) ?? "");
    if (Number.isFinite(saved)) setWidth(p, saved);
    attach(p);
  }
  // keep panels within bounds when the window shrinks
  window.addEventListener("resize", () => {
    for (const p of PANELS) {
      const saved = Number.parseFloat(load(storageKey(p)) ?? "");
      if (Number.isFinite(saved)) setWidth(p, saved);
    }
  });
}
