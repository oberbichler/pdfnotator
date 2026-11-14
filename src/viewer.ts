/**
 * Continuous-scroll PDF viewer built on pdf.js.
 *
 * Reloads are seamless: a new version of the document is rendered off-screen
 * page by page and swapped in place, keeping scroll position and zoom. Text
 * layers are only swapped when the user is not selecting text on that page.
 */

import type { PDFDocumentProxy, RenderTask } from "pdfjs-dist";
import * as pdfjs from "pdfjs-dist";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";
import type { NRect } from "./lib/geometry";

pdfjs.GlobalWorkerOptions.workerSrc = workerUrl;

export type { NRect };

export interface PageView {
  index: number;
  el: HTMLDivElement;
  canvas: HTMLCanvasElement | null;
  textLayer: HTMLDivElement | null;
  overlay: HTMLDivElement;
  w: number; // PDF points at scale 1
  h: number;
  renderedKey: string;
  renderingKey: string;
  task: RenderTask | null;
  pendingText: HTMLDivElement | null; // text layer waiting to be swapped in
}

interface Anchor {
  index: number;
  frac: number;
}

/** Zoom and scroll position — saved per tab and restored when switching back. */
export interface ViewState {
  scale: number;
  fitWidth: boolean;
  page: number;
  frac: number;
}

/** Entry of the PDF outline (bookmarks), resolved to a page. */
export interface OutlineItem {
  title: string;
  /** 0-based page index, or -1 if the destination could not be resolved. */
  page: number;
  /** Vertical position on the page (0 = top). */
  frac: number;
  children: OutlineItem[];
}

const PAGE_GAP = 16;
const PADDING = 24;
export const MIN_SCALE = 0.25;
export const MAX_SCALE = 6;
/** Longest side of area previews sent to the agent by default (≈ 450 image tokens). */
export const PREVIEW_SIZE = 640;
/** Re-render this long after the last pinch step (canvases are CSS-stretched meanwhile). */
const ZOOM_RENDER_DELAY = 140;

export class Viewer {
  doc: PDFDocumentProxy | null = null;
  pages: PageView[] = [];
  scale = 1;
  fitWidth = true;
  currentPage = 1;

  private worker = new pdfjs.PDFWorker();
  private docKey = 0;
  private loadGen = 0;
  private visible = new Set<PageView>();
  private observer: IntersectionObserver;
  private renderScheduled = false;
  /** while true, text layer swaps are deferred so selections survive reloads */
  textSwapBlocked: (textLayer: HTMLElement) => boolean = () => false;

  constructor(
    public scrollEl: HTMLElement,
    private pagesEl: HTMLElement,
    private hooks: {
      onPageChange?: (page: number, count: number) => void;
      onPageCreated?: (pv: PageView) => void;
      onScaleChange?: (scale: number) => void;
      /** A document finished loading (`isReload`: same document, new version). */
      onDocumentLoaded?: (isReload: boolean) => void;
      /** A page's text layer was (re)placed — overlays depending on text can refresh. */
      onTextLayer?: (pv: PageView) => void;
    } = {},
  ) {
    this.observer = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          const pv = this.pages.find((p) => p.el === e.target);
          if (!pv) continue;
          if (e.isIntersecting) this.visible.add(pv);
          else this.visible.delete(pv);
        }
        this.scheduleRender();
      },
      { root: scrollEl, rootMargin: "75% 0px" },
    );
    scrollEl.addEventListener("scroll", () => this.updateCurrentPage(), { passive: true });
    new ResizeObserver(() => {
      if (this.fitWidth && this.doc) this.setScale(this.computeFitScale(), true);
    }).observe(scrollEl);
  }

  /** Load a PDF. On reload the visible pages are re-rendered off-screen and swapped in place. */
  async load(data: Uint8Array, isReload: boolean, restore?: ViewState | null): Promise<void> {
    const gen = ++this.loadGen;
    const doc = await pdfjs.getDocument({ data, worker: this.worker }).promise;
    if (gen !== this.loadGen) {
      doc.destroy();
      return;
    }
    const sizes: [number, number][] = [];
    for (let i = 1; i <= doc.numPages; i++) {
      const vp = (await doc.getPage(i)).getViewport({ scale: 1 });
      sizes.push([vp.width, vp.height]);
    }
    if (gen !== this.loadGen) {
      doc.destroy();
      return;
    }

    const sizesChanged =
      sizes.length !== this.pages.length ||
      sizes.some(([w, h], i) => this.pages[i].w !== w || this.pages[i].h !== h);
    const anchor = isReload && sizesChanged ? this.captureAnchor() : null;

    const old = this.doc;
    this.doc = doc;
    this.docKey++;

    if (!isReload) {
      for (const pv of this.pages) {
        pv.task?.cancel();
        this.observer.unobserve(pv.el);
        pv.el.remove();
      }
      this.pages = [];
      this.visible.clear();
    }

    // grow / shrink page list
    for (const pv of this.pages.splice(sizes.length)) {
      pv.task?.cancel();
      this.observer.unobserve(pv.el);
      this.visible.delete(pv);
      pv.el.remove();
    }
    sizes.forEach(([w, h], i) => {
      let pv = this.pages[i];
      if (!pv) {
        pv = this.createPage(i, w, h);
        this.pages.push(pv);
      }
      pv.w = w;
      pv.h = h;
    });

    if (!isReload) {
      this.fitWidth = restore?.fitWidth ?? true;
      this.scale = this.fitWidth || !restore ? this.computeFitScale() : restore.scale;
      this.hooks.onScaleChange?.(this.scale);
    }
    this.layout();
    if (anchor) this.restoreAnchor(anchor);
    if (!isReload) {
      if (restore) this.restoreAnchor({ index: restore.page, frac: restore.frac });
      else this.scrollEl.scrollTop = 0;
    }

    this.textCache = null;
    await this.renderVisibleNow();
    // old document's canvases are already painted; safe to free it
    old?.destroy();
    this.updateCurrentPage();
    this.hooks.onDocumentLoaded?.(isReload);
  }

  private createPage(index: number, w: number, h: number): PageView {
    const el = document.createElement("div");
    el.className = "page";
    el.dataset.page = String(index + 1);
    const overlay = document.createElement("div");
    overlay.className = "overlay";
    el.append(overlay);
    this.pagesEl.append(el);
    const pv: PageView = {
      index,
      el,
      canvas: null,
      textLayer: null,
      overlay,
      w,
      h,
      renderedKey: "",
      renderingKey: "",
      task: null,
      pendingText: null,
    };
    this.observer.observe(el);
    this.hooks.onPageCreated?.(pv);
    return pv;
  }

  private layout() {
    for (const pv of this.pages) {
      pv.el.style.width = `${Math.floor(pv.w * this.scale)}px`;
      pv.el.style.height = `${Math.floor(pv.h * this.scale)}px`;
      pv.el.style.setProperty("--total-scale-factor", String(this.scale));
      pv.el.style.setProperty("--scale-round-x", "1px");
      pv.el.style.setProperty("--scale-round-y", "1px");
    }
  }

  computeFitScale(): number {
    if (!this.pages.length) return 1;
    const maxW = Math.max(...this.pages.map((p) => p.w));
    const avail = this.scrollEl.clientWidth - PADDING * 2;
    return Math.max(MIN_SCALE, Math.min(4, avail / maxW));
  }

  setScale(scale: number, keepFit = false) {
    scale = Math.max(MIN_SCALE, Math.min(MAX_SCALE, scale));
    if (!keepFit) this.fitWidth = false;
    if (Math.abs(scale - this.scale) < 0.001) return;
    const anchor = this.captureAnchor();
    this.scale = scale;
    this.layout();
    if (anchor) this.restoreAnchor(anchor);
    this.hooks.onScaleChange?.(scale);
    this.scheduleRender();
  }

  zoomBy(f: number) {
    this.setScale(this.scale * f);
  }

  private zoomRenderTimer = 0;

  /**
   * Zoom around a point on screen (pinch / ctrl+wheel). The document point under
   * the cursor stays under the cursor. Re-rendering is deferred until the gesture
   * pauses; meanwhile the existing canvases are stretched by CSS.
   */
  zoomAt(factor: number, clientX: number, clientY: number) {
    const scale = Math.max(MIN_SCALE, Math.min(MAX_SCALE, this.scale * factor));
    if (Math.abs(scale - this.scale) < 0.0005 || !this.pages.length) return;
    this.fitWidth = false;
    const box = this.scrollEl.getBoundingClientRect();
    const vx = clientX - box.left;
    const vy = clientY - box.top;
    const cx = this.scrollEl.scrollLeft + vx;
    const cy = this.scrollEl.scrollTop + vy;
    // page under the cursor (or nearest in vertical direction)
    let pv = this.pages[this.pages.length - 1];
    for (const p of this.pages) {
      if (p.el.offsetTop + p.el.offsetHeight + PAGE_GAP / 2 >= cy) {
        pv = p;
        break;
      }
    }
    const fx = (cx - pv.el.offsetLeft) / pv.el.offsetWidth;
    const fy = (cy - pv.el.offsetTop) / pv.el.offsetHeight;

    this.scale = scale;
    this.layout();
    this.scrollEl.scrollLeft = pv.el.offsetLeft + fx * pv.el.offsetWidth - vx;
    this.scrollEl.scrollTop = pv.el.offsetTop + fy * pv.el.offsetHeight - vy;
    this.hooks.onScaleChange?.(scale);

    clearTimeout(this.zoomRenderTimer);
    this.zoomRenderTimer = window.setTimeout(() => this.scheduleRender(), ZOOM_RENDER_DELAY);
  }

  fit() {
    this.fitWidth = true;
    this.setScale(this.computeFitScale(), true);
  }

  /** Current zoom and scroll position. */
  viewState(): ViewState | null {
    const a = this.captureAnchor();
    return a ? { scale: this.scale, fitWidth: this.fitWidth, page: a.index, frac: a.frac } : null;
  }

  /** Version of the loaded document; changes on every (re)load. */
  get version(): number {
    return this.docKey;
  }

  /** Remove all pages (no document open). */
  clear(): void {
    this.loadGen++;
    for (const pv of this.pages) {
      pv.task?.cancel();
      this.observer.unobserve(pv.el);
      pv.el.remove();
    }
    this.pages = [];
    this.visible.clear();
    void this.doc?.destroy();
    this.doc = null;
    this.docKey++;
    this.textCache = null;
    this.hooks.onPageChange?.(0, 0);
  }

  private captureAnchor(): Anchor | null {
    const top = this.scrollEl.scrollTop;
    for (const pv of this.pages) {
      const bottom = pv.el.offsetTop + pv.el.offsetHeight + PAGE_GAP;
      if (bottom > top) {
        return { index: pv.index, frac: (top - pv.el.offsetTop) / pv.el.offsetHeight };
      }
    }
    return null;
  }

  private restoreAnchor(a: Anchor) {
    const pv = this.pages[Math.min(a.index, this.pages.length - 1)];
    if (!pv) return;
    this.scrollEl.scrollTop = pv.el.offsetTop + a.frac * pv.el.offsetHeight;
  }

  private updateCurrentPage() {
    const mid = this.scrollEl.scrollTop + this.scrollEl.clientHeight / 3;
    let cur = 1;
    for (const pv of this.pages) {
      if (pv.el.offsetTop <= mid) cur = pv.index + 1;
      else break;
    }
    if (cur !== this.currentPage || !this.doc) {
      this.currentPage = cur;
    }
    this.hooks.onPageChange?.(cur, this.pages.length);
  }

  scrollToPage(page: number, frac = 0) {
    const pv = this.pages[page - 1];
    if (pv)
      this.scrollEl.scrollTo({ top: pv.el.offsetTop + frac * pv.el.offsetHeight - 40, behavior: "smooth" });
  }

  scheduleRender() {
    if (this.renderScheduled) return;
    this.renderScheduled = true;
    requestAnimationFrame(() => {
      this.renderScheduled = false;
      void this.renderVisibleNow();
    });
  }

  private async renderVisibleNow() {
    const viewMid = this.scrollEl.scrollTop + this.scrollEl.clientHeight / 2;
    const list = [...this.visible].sort(
      (a, b) =>
        Math.abs(a.el.offsetTop + a.el.offsetHeight / 2 - viewMid) -
        Math.abs(b.el.offsetTop + b.el.offsetHeight / 2 - viewMid),
    );
    await Promise.all(list.map((pv) => this.renderPage(pv)));
  }

  private async renderPage(pv: PageView) {
    const doc = this.doc;
    if (!doc) return;
    const scale = this.scale;
    const key = `${this.docKey}@${scale}`;
    if (pv.renderedKey === key || pv.renderingKey === key) return;
    pv.renderingKey = key;
    pv.task?.cancel();

    try {
      const page = await doc.getPage(pv.index + 1);
      if (pv.renderingKey !== key) return;
      const vp = page.getViewport({ scale });
      const dpr = window.devicePixelRatio || 1;
      const canvas = document.createElement("canvas");
      canvas.width = Math.floor(vp.width * dpr);
      canvas.height = Math.floor(vp.height * dpr);
      const task = page.render({
        canvas,
        viewport: vp,
        transform: dpr !== 1 ? [dpr, 0, 0, dpr, 0, 0] : undefined,
      });
      pv.task = task;
      await task.promise;
      if (pv.renderingKey !== key) return;

      const tl = document.createElement("div");
      tl.className = "textLayer";
      const textLayer = new pdfjs.TextLayer({
        textContentSource: page.streamTextContent(),
        container: tl,
        viewport: vp,
      });
      await textLayer.render();
      if (pv.renderingKey !== key) return;
      const eoc = document.createElement("div");
      eoc.className = "endOfContent";
      tl.append(eoc);

      // swap canvas in a single frame — no blank state in between
      if (pv.canvas) pv.canvas.replaceWith(canvas);
      else pv.el.prepend(canvas);
      pv.canvas = canvas;
      pv.pendingText = tl;
      this.flushTextSwap(pv);
      pv.renderedKey = key;
    } catch (e: unknown) {
      if ((e as { name?: string })?.name !== "RenderingCancelledException") console.warn(e);
    } finally {
      if (pv.renderingKey === key) pv.renderingKey = "";
    }
  }

  /** Swap pending text layers unless the user is selecting text on that page. */
  flushTextSwap(pv?: PageView) {
    const list = pv ? [pv] : this.pages;
    for (const p of list) {
      if (!p.pendingText) continue;
      if (p.textLayer && this.textSwapBlocked(p.textLayer)) continue;
      if (p.textLayer) p.textLayer.replaceWith(p.pendingText);
      else p.overlay.before(p.pendingText);
      p.textLayer = p.pendingText;
      p.pendingText = null;
      this.hooks.onTextLayer?.(p);
    }
  }

  pageOf(node: Node | null): PageView | undefined {
    const el = (node instanceof Element ? node : node?.parentElement)?.closest(".page");
    return this.pages.find((p) => p.el === el);
  }

  private textCache: Promise<string[]> | null = null;

  /**
   * Plain text of every page, as the text layer will contain it (text items
   * concatenated, line ends as a space). Cached per document version.
   */
  pageTexts(): Promise<string[]> {
    const doc = this.doc;
    if (!doc) return Promise.resolve([]);
    this.textCache ??= (async () => {
      const texts: string[] = [];
      for (let i = 1; i <= doc.numPages; i++) {
        const content = await (await doc.getPage(i)).getTextContent();
        texts.push(
          content.items.map((item) => ("str" in item ? item.str + (item.hasEOL ? " " : "") : "")).join(""),
        );
      }
      return texts;
    })();
    return this.textCache;
  }

  /** The document outline (bookmarks) with destinations resolved to pages. */
  async outline(): Promise<OutlineItem[]> {
    const doc = this.doc;
    if (!doc) return [];
    const raw = await doc.getOutline();
    if (!raw) return [];
    type RawItem = (typeof raw)[number];
    const resolve = async (item: RawItem): Promise<OutlineItem> => {
      let page = -1;
      let frac = 0;
      try {
        const dest = typeof item.dest === "string" ? await doc.getDestination(item.dest) : item.dest;
        const ref = dest?.[0];
        if (ref && typeof ref === "object") page = await doc.getPageIndex(ref);
        else if (typeof ref === "number") page = ref;
        // [ref, {name: "XYZ"}, left, top, zoom] — top is in PDF units from the bottom
        const top = dest?.[3];
        const height = this.pages[page]?.h;
        if (typeof top === "number" && height) frac = Math.min(1, Math.max(0, 1 - top / height));
      } catch {
        page = -1;
      }
      return { title: item.title, page, frac, children: await Promise.all(item.items.map(resolve)) };
    };
    return Promise.all(raw.map(resolve));
  }

  /** Render a small preview of a page into a new canvas `cssWidth` px wide. */
  async thumbnail(index: number, cssWidth: number): Promise<HTMLCanvasElement> {
    if (!this.doc) throw new Error("no document loaded");
    const page = await this.doc.getPage(index + 1);
    const base = page.getViewport({ scale: 1 });
    const dpr = window.devicePixelRatio || 1;
    const vp = page.getViewport({ scale: (cssWidth / base.width) * dpr });
    const canvas = document.createElement("canvas");
    canvas.width = Math.floor(vp.width);
    canvas.height = Math.floor(vp.height);
    await page.render({ canvas, viewport: vp }).promise;
    return canvas;
  }

  /**
   * Render a page region as PNG (base64, no prefix): a sharp full-resolution
   * image and a small preview (longest side ≤ {@link PREVIEW_SIZE} px) that is
   * cheap to send to an agent.
   */
  async captureRegion(pageIndex: number, r: NRect): Promise<{ image: string; preview: string }> {
    if (!this.doc) throw new Error("no document loaded");
    const page = await this.doc.getPage(pageIndex + 1);
    const base = page.getViewport({ scale: 1 });
    let s = Math.min(6, Math.max(2, 1400 / Math.max(1, r[2] * base.width)));
    s = Math.min(s, 10000 / base.width, 10000 / base.height);
    const vp = page.getViewport({ scale: s });
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.ceil(r[2] * vp.width));
    canvas.height = Math.max(1, Math.ceil(r[3] * vp.height));
    await page.render({
      canvas,
      viewport: vp,
      transform: [1, 0, 0, 1, -r[0] * vp.width, -r[1] * vp.height],
    }).promise;
    const toBase64 = (c: HTMLCanvasElement) => c.toDataURL("image/png").split(",")[1] ?? "";
    const shrink = Math.min(1, PREVIEW_SIZE / Math.max(canvas.width, canvas.height));
    let preview = canvas;
    if (shrink < 1) {
      preview = document.createElement("canvas");
      preview.width = Math.max(1, Math.round(canvas.width * shrink));
      preview.height = Math.max(1, Math.round(canvas.height * shrink));
      const ctx = preview.getContext("2d");
      if (ctx) {
        ctx.imageSmoothingQuality = "high";
        ctx.drawImage(canvas, 0, 0, preview.width, preview.height);
      }
    }
    return { image: toBase64(canvas), preview: toBase64(preview) };
  }
}

export function selectionTouches(el: HTMLElement): boolean {
  const sel = window.getSelection();
  if (!sel || sel.isCollapsed || sel.rangeCount === 0) return false;
  return sel.getRangeAt(0).intersectsNode(el);
}
