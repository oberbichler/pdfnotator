/**
 * Sketches drawn on a page: shape model, geometry, SVG paths and the
 * description sent to the agent. Points are normalised page coordinates
 * (`0..1`, origin top-left); paths and sizes use PDF points (1/72 in).
 */
import { clamp, type NRect } from "./geometry";

export type ShapeType = "pen" | "arrow" | "rect";
export const SHAPE_TYPES: readonly ShapeType[] = ["pen", "arrow", "rect"];

export type Point = [x: number, y: number];

export interface Shape {
  type: ShapeType;
  /** Pen: the whole stroke; arrow and rectangle: start and end. */
  points: Point[];
}

/** Page size in PDF points. */
export interface PageSize {
  w: number;
  h: number;
}

/** Stroke width on paper, in points. */
export const STROKE_PT = 2;
export const SKETCH_COLOR = "#e11d48";

const MM_PER_PT = 25.4 / 72;
const mm = (pt: number) => Math.round(pt * MM_PER_PT);

/** Wire format of one shape (`FeedbackShape` in Rust). Positions in mm from the page's top-left corner. */
export interface ShapePayload {
  type: ShapeType;
  /** Bounding box `[x, y, w, h]` in mm. */
  boxMm: [number, number, number, number];
  fromMm?: [number, number];
  toMm?: [number, number];
  /** Pen stroke that ends where it started (encircles something). */
  closed?: boolean;
  /** Text inside the shape (pen, rectangle). */
  text?: string;
  fromText?: string;
  toText?: string;
}

/** Extend the shape being drawn by `p` (pen: append unless too close; others: move the end point). */
export function extendShape(shape: Shape, p: Point, page: PageSize, minPt = 1): void {
  if (shape.type !== "pen") {
    shape.points = [shape.points[0] ?? p, p];
    return;
  }
  const last = shape.points.at(-1);
  if (!last || distPt(last, p, page) >= minPt) shape.points.push(p);
}

const distPt = (a: Point, b: Point, page: PageSize) =>
  Math.hypot((a[0] - b[0]) * page.w, (a[1] - b[1]) * page.h);

/** Shapes smaller than `minPt` in every direction are accidental clicks. */
export function isAccidental(shape: Shape, page: PageSize, minPt = 3): boolean {
  const [, , w, h] = shapeBounds(shape);
  return w * page.w < minPt && h * page.h < minPt;
}

export function shapeBounds(shape: Shape): NRect {
  const xs = shape.points.map((p) => p[0]);
  const ys = shape.points.map((p) => p[1]);
  const x = Math.min(...xs);
  const y = Math.min(...ys);
  return [x, y, Math.max(...xs) - x, Math.max(...ys) - y];
}

/** Grow a normalised rect by `pt` on every side, clamped to the page. */
export function padRect(r: NRect, pt: number, page: PageSize): NRect {
  const x1 = clamp(r[0] - pt / page.w, 0, 1);
  const y1 = clamp(r[1] - pt / page.h, 0, 1);
  const x2 = clamp(r[0] + r[2] + pt / page.w, 0, 1);
  const y2 = clamp(r[1] + r[3] + pt / page.h, 0, 1);
  return [x1, y1, x2 - x1, y2 - y1];
}

/** Region shown in the screenshot: all shapes plus some surrounding page. */
export function sketchBounds(shapes: readonly Shape[], page: PageSize, padPt = 24): NRect {
  const boxes = shapes.map(shapeBounds);
  const x1 = Math.min(...boxes.map((b) => b[0]));
  const y1 = Math.min(...boxes.map((b) => b[1]));
  const x2 = Math.max(...boxes.map((b) => b[0] + b[2]));
  const y2 = Math.max(...boxes.map((b) => b[1] + b[3]));
  return padRect([x1, y1, x2 - x1, y2 - y1], padPt, page);
}

/** A pen stroke whose ends meet, relative to its size (a loop around something). */
export function isClosed(shape: Shape, page: PageSize): boolean {
  if (shape.type !== "pen" || shape.points.length < 8) return false;
  const [, , w, h] = shapeBounds(shape);
  const size = Math.max(w * page.w, h * page.h);
  const first = shape.points[0] as Point;
  const last = shape.points.at(-1) as Point;
  return size > 10 && distPt(first, last, page) < size * 0.3;
}

const f = (v: number) => Number(v.toFixed(2));

/** SVG path of a shape in page points (`viewBox="0 0 w h"`). */
export function shapePath(shape: Shape, page: PageSize): string {
  const pts = shape.points.map(([x, y]) => [f(x * page.w), f(y * page.h)] as const);
  const [a, b = a] = [pts[0], pts.at(-1)];
  if (!a || !b) return "";
  switch (shape.type) {
    case "rect":
      return `M${a[0]} ${a[1]}H${b[0]}V${b[1]}H${a[0]}Z`;
    case "arrow": {
      const len = Math.hypot(b[0] - a[0], b[1] - a[1]);
      const head = clamp(len * 0.35, 4, 12);
      const angle = Math.atan2(b[1] - a[1], b[0] - a[0]);
      const wing = (side: number) => {
        const t = angle + Math.PI + side * 0.45;
        return `${f(b[0] + head * Math.cos(t))} ${f(b[1] + head * Math.sin(t))}`;
      };
      return `M${a[0]} ${a[1]}L${b[0]} ${b[1]}M${wing(-1)}L${b[0]} ${b[1]}L${wing(1)}`;
    }
    case "pen": {
      if (pts.length < 3) return `M${a[0]} ${a[1]}L${b[0] + (pts.length === 1 ? 0.01 : 0)} ${b[1]}`;
      // quadratic curves through the midpoints smooth out pointer jitter
      let d = `M${a[0]} ${a[1]}`;
      for (let i = 1; i < pts.length - 1; i++) {
        const p = pts[i] as (typeof pts)[number];
        const q = pts[i + 1] as (typeof pts)[number];
        d += `Q${p[0]} ${p[1]} ${f((p[0] + q[0]) / 2)} ${f((p[1] + q[1]) / 2)}`;
      }
      return `${d}L${b[0]} ${b[1]}`;
    }
  }
}

/** Distance in points around an arrow end that counts as "pointing at" text there. */
const ARROW_REACH_PT = 14;

/**
 * Describe the shapes for the agent. `textIn` returns the page text inside a
 * normalised rectangle.
 */
export function describeShapes(
  shapes: readonly Shape[],
  page: PageSize,
  textIn: (r: NRect) => string,
): ShapePayload[] {
  const toMm = (p: Point): [number, number] => [mm(p[0] * page.w), mm(p[1] * page.h)];
  const around = (p: Point) => padRect([p[0], p[1], 0, 0], ARROW_REACH_PT, page);
  const nonEmpty = (s: string) => s.replace(/\s+/g, " ").trim() || undefined;

  return shapes.map((shape) => {
    const box = shapeBounds(shape);
    const out: ShapePayload = {
      type: shape.type,
      boxMm: [mm(box[0] * page.w), mm(box[1] * page.h), mm(box[2] * page.w), mm(box[3] * page.h)],
    };
    if (shape.type === "arrow") {
      const from = shape.points[0] as Point;
      const to = shape.points.at(-1) as Point;
      out.fromMm = toMm(from);
      out.toMm = toMm(to);
      out.fromText = nonEmpty(textIn(around(from)));
      out.toText = nonEmpty(textIn(around(to)));
    } else {
      // a few points of slack: underlines and strike-throughs have almost no height
      out.text = nonEmpty(textIn(padRect(box, 4, page)));
      if (isClosed(shape, page)) out.closed = true;
    }
    return out;
  });
}

/**
 * The words of `str` that have at least one character marked in `hit`
 * (a shape that touches part of a word means the whole word).
 */
export function touchedWords(str: string, hit: readonly boolean[]): string {
  const words: string[] = [];
  for (const m of str.matchAll(/\S+/g)) {
    const start = m.index ?? 0;
    if (hit.slice(start, start + m[0].length).some(Boolean)) words.push(m[0]);
  }
  return words.join(" ");
}
