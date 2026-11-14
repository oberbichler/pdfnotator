/**
 * Draw tool: sketch on a page (freehand, arrows, rectangles), then type to
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
} from "../lib/sketch";
import { nextCommentId, state, update } from "../store";
import type { PageView, Viewer } from "../viewer";
import { textInRect } from "./area-tool";
import { closeComposer, isComposerOpen, openComposer } from "./composer";
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
  for (const s of shapes) ctx.stroke(new Path2D(shapePath(s, page)));
}

/** A sketch is waiting for its comment. */
export function hasSketch(): boolean {
  if (pending && !pending.layer.isConnected) clearSketch(); // the page was re-created
  return !!pending?.shapes.length;
}

export function clearSketch(): void {
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
  update((s) => (s.shape = shape));
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
  pending.hint.hidden = !done.length || isComposerOpen();
  if (done.length) {
    const [x, y, , hgt] = sketchBounds(done, page, 6);
    pending.hint.style.left = `${x * 100}%`;
    pending.hint.style.top = `${(y + hgt) * 100}%`;
  }
}

/** Start a shape at normalised point `p` (also used by the demo). */
export function beginShape(pv: PageView, p: Point): void {
  closeComposer(); // keeps the sketch: keep drawing, type again to comment
  if (pending && (pending.pv !== pv || !pending.layer.isConnected)) clearSketch();
  if (!pending) {
    const layer = sketchSvg([], pageSize(pv), "sketch-layer");
    const hint = h("div", "sketch-hint", t("sketch.hint"));
    hint.hidden = true;
    pv.overlay.append(layer, hint);
    pending = { pv, shapes: [], layer, hint };
  }
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

export function attachSketchTool(pv: PageView): void {
  const overlay = pv.overlay;
  const toPage = (e: PointerEvent): Point => {
    const r = pv.el.getBoundingClientRect();
    return [clamp((e.clientX - r.left) / r.width, 0, 1), clamp((e.clientY - r.top) / r.height, 0, 1)];
  };

  overlay.addEventListener("pointerdown", (e) => {
    if (state.tool !== "draw" || e.button !== 0) return;
    e.preventDefault();
    overlay.setPointerCapture(e.pointerId);
    beginShape(pv, toPage(e));
  });
  overlay.addEventListener("pointermove", (e) => extendCurrent(toPage(e)));
  overlay.addEventListener("pointerup", () => endShape());
  overlay.addEventListener("pointercancel", () => endShape(true));
}

/** Open the composer for the pending sketch. Returns false if there is none. */
export function commentSketch(initialText = ""): boolean {
  if (!hasSketch() || !pending || current) return false;
  const { pv } = pending;
  const shapes = pending.shapes.map((s) => ({ type: s.type, points: [...s.points] }));
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
