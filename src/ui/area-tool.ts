/** Rectangle tool: drag a region on a page to comment on it with a screenshot. */
import { t } from "../i18n";
import { clamp, isTiny, type NRect, rectFromPoints } from "../lib/geometry";
import { nextCommentId, state } from "../store";
import type { PageView, Viewer } from "../viewer";
import { closeComposer, openComposer } from "./composer";
import { h } from "./dom";
import { placeRect } from "./marks";

export function attachAreaTool(viewer: Viewer, pv: PageView): void {
  const overlay = pv.overlay;
  let start: [number, number] | null = null;
  let band: HTMLDivElement | null = null;

  const toPage = (e: PointerEvent): [number, number] => {
    const r = pv.el.getBoundingClientRect();
    return [clamp((e.clientX - r.left) / r.width, 0, 1), clamp((e.clientY - r.top) / r.height, 0, 1)];
  };

  overlay.addEventListener("pointerdown", (e) => {
    if (state.tool !== "area" || e.button !== 0) return;
    e.preventDefault();
    closeComposer();
    overlay.setPointerCapture(e.pointerId);
    start = toPage(e);
    band = h("div", "band");
    overlay.append(band);
    placeRect(band, [start[0], start[1], 0, 0]);
  });

  overlay.addEventListener("pointermove", (e) => {
    if (start && band) placeRect(band, rectFromPoints(start, toPage(e)));
  });

  const finish = (e: PointerEvent) => {
    if (!start || !band) return;
    const rect = rectFromPoints(start, toPage(e));
    const bandEl = band;
    start = null;
    band = null;
    if (e.type === "pointercancel" || isTiny(rect, pv.el.getBoundingClientRect())) {
      bandEl.remove();
      return;
    }
    commentArea(viewer, pv, rect, bandEl);
  };
  overlay.addEventListener("pointerup", finish);
  overlay.addEventListener("pointercancel", finish);
}

/** Open the composer for an area comment on `rect` (the `band` is removed when it closes). */
export function commentArea(viewer: Viewer, pv: PageView, rect: NRect, band: HTMLElement): void {
  const imagePromise = viewer.captureRegion(pv.index, rect);
  const draft = {
    id: nextCommentId(),
    kind: "area" as const,
    page: pv.index + 1,
    rect,
    areaText: textInRect(pv, rect),
    imagePromise,
  };
  openComposer(band.getBoundingClientRect(), draft, {
    label: t("composer.area", { page: draft.page }),
    image: imagePromise,
    onClose: () => band.remove(),
  });
}

/**
 * Text of the page's text layer inside `rect`, clipped per character
 * (pdf.js spans usually cover whole lines).
 */
export function textInRect(pv: PageView, rect: NRect): string {
  if (!pv.textLayer) return "";
  const page = pv.el.getBoundingClientRect();
  const x1 = page.left + rect[0] * page.width;
  const y1 = page.top + rect[1] * page.height;
  const x2 = x1 + rect[2] * page.width;
  const y2 = y1 + rect[3] * page.height;
  const range = document.createRange();
  const lines: { top: number; text: string }[] = [];

  for (const span of pv.textLayer.querySelectorAll("span")) {
    if (span.children.length) continue;
    const b = span.getBoundingClientRect();
    if (b.bottom < y1 || b.top > y2 || b.right < x1 || b.left > x2) continue;
    const node = span.firstChild;
    if (node?.nodeType !== Node.TEXT_NODE) continue;
    const str = node.textContent ?? "";
    let inside = "";
    for (let i = 0; i < str.length; i++) {
      range.setStart(node, i);
      range.setEnd(node, i + 1);
      const c = range.getBoundingClientRect();
      const cx = c.left + c.width / 2;
      const cy = c.top + c.height / 2;
      if (cx >= x1 && cx <= x2 && cy >= y1 && cy <= y2) inside += str[i];
    }
    if (!inside.trim()) continue;
    const line = lines.find((l) => Math.abs(l.top - b.top) < b.height * 0.5);
    if (line) line.text += ` ${inside}`;
    else lines.push({ top: b.top, text: inside });
  }
  return lines
    .sort((a, b) => a.top - b.top)
    .map((l) => l.text.replace(/\s+/g, " ").trim())
    .join("\n");
}
