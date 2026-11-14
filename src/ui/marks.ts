/**
 * Comment marks drawn on top of the PDF pages. Resolved comments are hidden
 * unless selected in the sidebar.
 */
import { displayNumber } from "../lib/comments";
import type { NRect } from "../lib/geometry";
import { shapeBounds } from "../lib/sketch";
import { activeDoc, type State, subscribe } from "../store";
import type { Viewer } from "../viewer";
import { h } from "./dom";
import { sketchSvg } from "./sketch-tool";

export function placeRect(el: HTMLElement, r: NRect): void {
  el.style.left = `${r[0] * 100}%`;
  el.style.top = `${r[1] * 100}%`;
  el.style.width = `${r[2] * 100}%`;
  el.style.height = `${r[3] * 100}%`;
}

export function renderMarks(viewer: Viewer, s: State): void {
  for (const pv of viewer.pages) {
    for (const m of pv.overlay.querySelectorAll(".mark")) m.remove();
  }
  const doc = activeDoc(s);
  for (const c of doc?.comments ?? []) {
    if (c.status === "resolved" && c.id !== doc?.selectedId) continue;
    const pv = viewer.pages[c.page - 1];
    if (!pv) continue;
    if (c.sketch && c.rect) {
      // the strokes themselves, plus an invisible box carrying the pin
      const strokes = sketchSvg(c.sketch, { w: pv.w, h: pv.h }, `mark sketch-strokes ${c.status}`);
      strokes.classList.toggle("selected", c.id === doc?.selectedId);
      strokes.dataset.id = c.id;
      // pin at the top-left corner of the first shape, like area marks
      const [px, py] = c.sketch[0] ? shapeBounds(c.sketch[0]) : c.rect;
      const box = h("div", `mark sketch ${c.status}`);
      box.dataset.id = c.id;
      placeRect(box, [px, py, 0, 0]);
      box.append(h("span", "pin", displayNumber(c.id)));
      pv.overlay.append(strokes, box);
      continue;
    }
    const rects = c.kind === "area" && c.rect ? [c.rect] : (c.rects ?? []);
    rects.forEach((r, i) => {
      const mark = h("div", `mark ${c.kind} ${c.status}`);
      mark.classList.toggle("selected", c.id === doc?.selectedId);
      mark.dataset.id = c.id;
      placeRect(mark, r);
      if (i === 0) mark.append(h("span", "pin", displayNumber(c.id)));
      pv.overlay.append(mark);
    });
  }
}

/** Highlight the marks of a comment (sidebar hover). */
export function focusMarks(id: string, on: boolean): void {
  for (const m of document.querySelectorAll(`.mark[data-id="${CSS.escape(id)}"]`)) {
    m.classList.toggle("focus", on);
  }
}

export function flashMarks(id: string): void {
  focusMarks(id, true);
  setTimeout(() => focusMarks(id, false), 900);
}

export function initMarks(viewer: Viewer): void {
  subscribe((s) => renderMarks(viewer, s));
}
