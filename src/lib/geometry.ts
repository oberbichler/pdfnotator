/** Geometry helpers. Rectangles on a page are normalised to `0..1`, origin top-left. */

/** Normalised rectangle `[x, y, width, height]`. */
export type NRect = [x: number, y: number, w: number, h: number];

export interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

export const clamp = (v: number, lo: number, hi: number): number => Math.max(lo, Math.min(hi, v));

/** Rectangle spanned by two normalised points. */
export function rectFromPoints(a: readonly [number, number], b: readonly [number, number]): NRect {
  return [Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.abs(a[0] - b[0]), Math.abs(a[1] - b[1])];
}

/** Express a client-space box relative to a page box. */
export function normalizeBox(box: Box, page: Box): NRect {
  return [
    (box.left - page.left) / page.width,
    (box.top - page.top) / page.height,
    box.width / page.width,
    box.height / page.height,
  ];
}

/** True if `rect` is smaller than `minPx` in either direction on a page of the given size. */
export function isTiny(rect: NRect, page: Pick<Box, "width" | "height">, minPx = 6): boolean {
  return rect[2] * page.width < minPx || rect[3] * page.height < minPx;
}

/**
 * Merge the per-glyph-run rectangles of a text selection into one rectangle
 * per line. Rects on the same line (vertical offset < half a line) that touch
 * or nearly touch (`gap`) are combined.
 */
export function mergeLineRects(rects: readonly NRect[], gap = 0.02): NRect[] {
  const sorted = [...rects].sort((a, b) => a[1] - b[1] || a[0] - b[0]);
  const out: NRect[] = [];
  for (const r of sorted) {
    const prev = out.at(-1);
    if (prev && Math.abs(prev[1] - r[1]) < r[3] * 0.5 && r[0] <= prev[0] + prev[2] + gap) {
      const x1 = Math.min(prev[0], r[0]);
      const y1 = Math.min(prev[1], r[1]);
      const x2 = Math.max(prev[0] + prev[2], r[0] + r[2]);
      const y2 = Math.max(prev[1] + prev[3], r[1] + r[3]);
      out[out.length - 1] = [x1, y1, x2 - x1, y2 - y1];
    } else {
      out.push([...r]);
    }
  }
  return out;
}
