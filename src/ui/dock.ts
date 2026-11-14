/** Floating dock: tool switch, zoom, page navigation — plus pinch-to-zoom. */
import { type Tool, update } from "../store";
import type { Viewer } from "../viewer";
import { byId } from "./dom";

const ZOOM_STEP = 1.15;

export function setTool(tool: Tool): void {
  update((s) => (s.tool = tool));
  document.body.classList.toggle("mode-area", tool === "area");
  for (const b of document.querySelectorAll<HTMLButtonElement>("#tools button")) {
    const active = b.dataset.tool === tool;
    b.classList.toggle("active", active);
    b.setAttribute("aria-pressed", String(active));
  }
}

export const zoomIn = (viewer: Viewer) => viewer.zoomBy(ZOOM_STEP);
export const zoomOut = (viewer: Viewer) => viewer.zoomBy(1 / ZOOM_STEP);

/**
 * Trackpad pinch. Chromium/WebView2 deliver it as ctrl+wheel, WebKit as
 * `gesture*` events — handle both, but never both at once.
 */
function initPinch(viewer: Viewer): void {
  const scroller = viewer.scrollEl;
  let gesture: { scale: number } | null = null;

  scroller.addEventListener(
    "wheel",
    (e) => {
      if (!e.ctrlKey) return; // two-finger scrolling stays native
      e.preventDefault();
      if (gesture) return;
      const delta = e.deltaMode === WheelEvent.DOM_DELTA_LINE ? e.deltaY * 16 : e.deltaY;
      viewer.zoomAt(Math.exp(-delta * 0.01), e.clientX, e.clientY);
    },
    { passive: false },
  );

  type GestureEvent = UIEvent & { scale: number; clientX: number; clientY: number };
  scroller.addEventListener("gesturestart", (e) => {
    e.preventDefault();
    gesture = { scale: 1 };
  });
  scroller.addEventListener("gesturechange", (e) => {
    e.preventDefault();
    const g = e as GestureEvent;
    if (!gesture) return;
    viewer.zoomAt(g.scale / gesture.scale, g.clientX, g.clientY);
    gesture.scale = g.scale;
  });
  scroller.addEventListener("gestureend", (e) => {
    e.preventDefault();
    gesture = null;
  });
}

export function initDock(viewer: Viewer): { onPageChange: (page: number, count: number) => void } {
  const pageInput = byId<HTMLInputElement>("page-input");

  for (const b of document.querySelectorAll<HTMLButtonElement>("#tools button")) {
    b.addEventListener("click", () => setTool(b.dataset.tool as Tool));
  }
  byId("zoom-in").addEventListener("click", () => zoomIn(viewer));
  byId("zoom-out").addEventListener("click", () => zoomOut(viewer));
  byId("zoom-label").addEventListener("click", () => viewer.fit());

  pageInput.addEventListener("focus", () => pageInput.select());
  pageInput.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      const n = Number.parseInt(pageInput.value, 10);
      if (n >= 1 && n <= viewer.pages.length) viewer.scrollToPage(n);
      pageInput.blur();
    } else if (e.key === "Escape") {
      pageInput.blur();
    }
  });
  pageInput.addEventListener("blur", () => {
    pageInput.value = String(viewer.currentPage);
  });

  initPinch(viewer);

  return {
    onPageChange(page, count) {
      if (document.activeElement !== pageInput) pageInput.value = count ? String(page) : "–";
      byId("page-count").textContent = `/ ${count || "–"}`;
    },
  };
}

export function showScale(scale: number): void {
  byId("zoom-label").textContent = `${Math.round(scale * 100)}%`;
}
