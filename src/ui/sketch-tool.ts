/**
 * Draw tool: sketch on a page (freehand, arrows, rectangles, typed notes), then type to
 * comment. The comment carries a screenshot with the sketch, the region
 * without it, and a description of each shape (text it touches, size in mm).
 */
import { t } from "../i18n";
import type { CommentDraft } from "../lib/comments";
import { clamp } from "../lib/geometry";
import {
  describeShapes,
  extendShape,
  isAccidental,
  type PageSize,
  type Point,
  SHAPE_TYPES,
  type Shape,
  type ShapeType,
  SKETCH_COLOR,
  STROKE_PT,
  shapePath,
  sketchBounds,
  TEXT_FONT,
  TEXT_PT,
  textOrigin,
  textShape,
} from "../lib/sketch";
import { nextCommentId, state, update } from "../store";
import type { PageView, Viewer } from "../viewer";
import { textInRect } from "./area-tool";
import { closeComposer, isComposerOpen, openComposer } from "./composer";
import { setTool } from "./dock";
import { h } from "./dom";

const SVG_NS = "http://www.w3.org/2000/svg";

interface Pending {
  pv: PageView;
  shapes: Shape[];
  layer: SVGSVGElement;
  hint: HTMLElement;
}

let viewer: Viewer;
let pending: Pending | null = null;
let current: Shape | null = null;
/** A pen (Apple Pencil) was used: from then on fingers scroll instead of drawing. */
let penSeen = false;
/** Input for a note being typed, with the point it was placed at. */
let note: { input: HTMLInputElement; at: Point } | null = null;

const pageSize = (pv: PageView): PageSize => ({ w: pv.w, h: pv.h });

/** SVG covering a page, in page points; one path per shape. */
export function sketchSvg(shapes: readonly Shape[], page: PageSize, cls: string): SVGSVGElement {
  const svg = document.createElementNS(SVG_NS, "svg");
  svg.setAttribute("class", cls);
  svg.setAttribute("viewBox", `0 0 ${page.w} ${page.h}`);
  svg.setAttribute("preserveAspectRatio", "none");
  fillSvg(svg, shapes, page);
  return svg;
}

function fillSvg(svg: SVGSVGElement, shapes: readonly Shape[], page: PageSize): void {
  svg.replaceChildren(
    ...shapes.map((s) => {
      if (s.type === "text") {
        const text = document.createElementNS(SVG_NS, "text");
        const [x, y] = textOrigin(s, page);
        text.setAttribute("x", String(x));
        text.setAttribute("y", String(y));
        text.style.font = TEXT_FONT;
        text.textContent = s.label ?? "";
        return text;
      }
      const path = document.createElementNS(SVG_NS, "path");
      path.setAttribute("d", shapePath(s, page));
      return path;
    }),
  );
}

/** Paint shapes on a canvas whose transform maps page points. */
function paint(ctx: CanvasRenderingContext2D, shapes: readonly Shape[], page: PageSize): void {
  ctx.strokeStyle = SKETCH_COLOR;
  ctx.lineWidth = STROKE_PT;
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.fillStyle = SKETCH_COLOR;
  ctx.font = TEXT_FONT;
  for (const s of shapes) {
    if (s.type === "text") ctx.fillText(s.label ?? "", ...textOrigin(s, page));
    else ctx.stroke(new Path2D(shapePath(s, page)));
  }
}

/** Width of a note in points. */
function measure(label: string): number {
  const ctx = document.createElement("canvas").getContext("2d");
  if (!ctx) return label.length * TEXT_PT * 0.6;
  ctx.font = TEXT_FONT;
  return ctx.measureText(label).width;
}

/** A sketch is waiting for its comment. */
export function hasSketch(): boolean {
  if (pending && !pending.layer.isConnected) clearSketch(); // the page was re-created
  return !!pending?.shapes.length;
}

export function clearSketch(): void {
  dropNote();
  pending?.layer.remove();
  pending?.hint.remove();
  pending = null;
  current = null;
}

export function undoShape(): void {
  if (!pending) return;
  pending.shapes.pop();
  if (pending.shapes.length) redraw();
  else clearSketch();
}

export function setShape(shape: ShapeType): void {
  if (shape !== "text") commitNote();
  update((s) => (s.shape = shape));
  document.body.classList.toggle("shape-text", shape === "text");
  for (const b of document.querySelectorAll<HTMLButtonElement>("#shapes button")) {
    const active = b.dataset.shape === shape;
    b.classList.toggle("active", active);
    b.setAttribute("aria-pressed", String(active));
  }
}

export function cycleShape(step: 1 | -1): void {
  const i = SHAPE_TYPES.indexOf(state.shape);
  setShape(SHAPE_TYPES[(i + step + SHAPE_TYPES.length) % SHAPE_TYPES.length] ?? "pen");
}

function redraw(): void {
  if (!pending) return;
  const page = pageSize(pending.pv);
  fillSvg(pending.layer, pending.shapes, page);
  const done = pending.shapes.filter((s) => s !== current);
  pending.hint.hidden = !done.length || isComposerOpen() || !!note;
  if (done.length) {
    const [x, y, , hgt] = sketchBounds(done, page, 6);
    pending.hint.style.left = `${x * 100}%`;
    pending.hint.style.top = `${(y + hgt) * 100}%`;
  }
}

/** The sketch on `pv`, started if needed. */
function sketchOn(pv: PageView): Pending {
  if (pending && (pending.pv !== pv || !pending.layer.isConnected)) clearSketch();
  if (!pending) {
    const layer = sketchSvg([], pageSize(pv), "sketch-layer");
    const hint = h("button", "sketch-hint", t(penSeen ? "sketch.hintTouch" : "sketch.hint"));
    hint.type = "button";
    hint.hidden = true;
    // tablets have no keyboard to start typing: tap the hint instead
    hint.addEventListener("pointerdown", (e) => e.stopPropagation());
    hint.addEventListener("click", (e) => {
      e.stopPropagation();
      commentSketch();
    });
    pv.overlay.append(layer, hint);
    pending = { pv, shapes: [], layer, hint };
  }
  return pending;
}

/** A note is being typed. */
export function isTypingNote(): boolean {
  return !!note;
}

function dropNote(): void {
  const input = note?.input;
  note = null;
  input?.remove();
  if (input) redraw();
}

/** Add the note being typed to the sketch (or drop it if empty). */
export function commitNote(): void {
  if (!note || !pending) {
    dropNote();
    return;
  }
  const label = note.input.value.trim();
  const at = note.at;
  dropNote();
  if (label) {
    pending.shapes.push(textShape(at, label, measure(label), pageSize(pending.pv)));
    redraw();
  } else if (!pending.shapes.length) clearSketch();
}

/** Place a note at normalised point `p` and focus its input. */
export function placeNote(pv: PageView, p: Point): void {
  closeComposer();
  commitNote();
  const sketch = sketchOn(pv);
  const input = h("input", "sketch-note");
  input.type = "text";
  input.spellcheck = false;
  input.style.left = `${p[0] * 100}%`;
  input.style.top = `${p[1] * 100}%`;
  // the same size on screen as on paper
  input.style.fontSize = `${(TEXT_PT * pv.el.getBoundingClientRect().width) / pv.w}px`;
  input.addEventListener("input", () => (input.style.width = `${input.value.length + 1}ch`));
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.isComposing) {
      e.preventDefault();
      commitNote();
    } else if (e.key === "Escape") {
      e.preventDefault();
      dropNote();
      if (!pending?.shapes.length) clearSketch();
    }
  });
  input.addEventListener("blur", () => {
    if (note?.input === input) commitNote();
  });
  sketch.pv.overlay.append(input);
  note = { input, at: p };
  redraw();
  input.focus({ preventScroll: true });
}

/** Start a shape at normalised point `p` (also used by the demo). */
export function beginShape(pv: PageView, p: Point): void {
  closeComposer(); // keeps the sketch: keep drawing, type again to comment
  commitNote();
  sketchOn(pv);
  if (!pending) return;
  current = { type: state.shape, points: [p] };
  pending.shapes.push(current);
  redraw();
}

export function extendCurrent(p: Point): void {
  if (!current || !pending) return;
  extendShape(current, p, pageSize(pending.pv));
  redraw();
}

/** Finish the shape being drawn; `cancel` drops it. */
export function endShape(cancel = false): void {
  if (!current || !pending) return;
  const shape = current;
  current = null;
  if (cancel || isAccidental(shape, pageSize(pending.pv))) pending.shapes.pop();
  if (pending.shapes.length) redraw();
  else clearSketch();
}

/** Keep receiving a pointer's events while it leaves the page (best effort). */
function capture(el: Element, pointerId: number): void {
  try {
    el.setPointerCapture(pointerId);
  } catch {
    // the pointer is already gone (or synthetic): drawing works without capture
  }
}

export function attachSketchTool(pv: PageView): void {
  const overlay = pv.overlay;
  const toPage = (e: PointerEvent): Point => {
    const r = pv.el.getBoundingClientRect();
    return [clamp((e.clientX - r.left) / r.width, 0, 1), clamp((e.clientY - r.top) / r.height, 0, 1)];
  };

  // Safari scrolls (and selects text) under an Apple Pencil unless its touches are cancelled;
  // pointer events still arrive. Finger touches keep scrolling and zooming.
  pv.el.addEventListener(
    "touchstart",
    (e) => {
      if ([...e.touches].some((touch) => (touch as Touch & { touchType?: string }).touchType === "stylus")) {
        e.preventDefault();
      }
    },
    { passive: false },
  );
  // a pen always draws, whatever tool is active
  pv.el.addEventListener(
    "pointerdown",
    (e) => {
      if (e.pointerType !== "pen" || e.button !== 0) return;
      if (e.target instanceof Element && e.target.closest(".sketch-note, .sketch-hint")) return;
      if (!penSeen) {
        penSeen = true;
        document.body.classList.add("pen");
      }
      if (state.tool !== "draw") setTool("draw");
      e.preventDefault();
      e.stopPropagation();
      if (state.shape === "text") {
        placeNote(pv, toPage(e));
        return;
      }
      capture(overlay, e.pointerId);
      beginShape(pv, toPage(e));
    },
    { capture: true },
  );

  overlay.addEventListener("pointerdown", (e) => {
    if (state.tool !== "draw" || e.button !== 0) return;
    if (e.pointerType === "touch" && penSeen) return; // fingers scroll, and a resting palm does nothing
    if (e.target instanceof Element && e.target.closest(".sketch-note")) return; // clicks into the note's input
    e.preventDefault();
    if (state.shape === "text") {
      placeNote(pv, toPage(e));
      return;
    }
    capture(overlay, e.pointerId);
    beginShape(pv, toPage(e));
  });
  overlay.addEventListener("pointermove", (e) => {
    if (e.pointerType === "touch" && penSeen) return;
    extendCurrent(toPage(e));
  });
  overlay.addEventListener("pointerup", (e) => {
    if (e.pointerType === "touch" && penSeen) return;
    endShape();
  });
  overlay.addEventListener("pointercancel", (e) => {
    if (e.pointerType === "touch" && penSeen) return;
    endShape(true);
  });
}

/** Open the composer for the pending sketch. Returns false if there is none. */
export function commentSketch(initialText = ""): boolean {
  commitNote();
  if (!hasSketch() || !pending || current) return false;
  const { pv } = pending;
  const shapes = pending.shapes.map((s) => ({ ...s, points: [...s.points] }));
  const page = pageSize(pv);
  const rect = sketchBounds(shapes, page);
  const imagePromise = viewer.captureRegion(pv.index, rect, (ctx) => paint(ctx, shapes, page));
  const draft: CommentDraft = {
    id: nextCommentId(),
    kind: "area",
    page: pv.index + 1,
    rect,
    areaText: textInRect(pv, rect),
    sketch: shapes,
    sketchInfo: describeShapes(shapes, page, (r) => textInRect(pv, r, true)),
    imagePromise,
  };
  const box = pv.el.getBoundingClientRect();
  const anchor = new DOMRect(
    box.left + rect[0] * box.width,
    box.top + rect[1] * box.height,
    rect[2] * box.width,
    rect[3] * box.height,
  );
  const sketch = pending;
  sketch.hint.hidden = true;
  openComposer(anchor, draft, {
    label: t("composer.sketch", { page: draft.page }),
    icon: "draw",
    image: imagePromise,
    initialText,
    onClose: (sent) => {
      if (sent && pending === sketch) clearSketch();
      else redraw();
    },
  });
  return true;
}

export function initSketchTool(v: Viewer): void {
  viewer = v;
  for (const b of document.querySelectorAll<HTMLButtonElement>("#shapes button")) {
    b.addEventListener("click", () => setShape(b.dataset.shape as ShapeType));
  }
}
