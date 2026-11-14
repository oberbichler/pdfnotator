/**
 * Find in document (⌘F): searches the text of all pages, highlights matches on
 * rendered pages and jumps between them. Survives reloads and tab switches.
 */
import { onLocaleChange, t } from "../i18n";
import { mergeLineRects, normalizeBox } from "../lib/geometry";
import { findAll, type PageMatch, searchPages } from "../lib/search";
import type { PageView, Viewer } from "../viewer";
import { byId, h } from "./dom";
import { placeRect } from "./marks";

let viewer: Viewer;
let bar: HTMLElement;
let input: HTMLInputElement;
let hits: PageMatch[] = [];
let current = -1;
let query = "";
let runId = 0;
/** Scroll to the current hit once its page has a text layer. */
let pendingScroll = false;

export const isSearchOpen = (): boolean => !bar.classList.contains("hidden");

export function openSearch(): void {
  const selected = window.getSelection()?.toString().trim();
  bar.classList.remove("hidden");
  if (selected && selected.length < 200 && !selected.includes("\n")) input.value = selected;
  input.focus();
  input.select();
  if (input.value.trim() !== query) void run(input.value, true);
}

export function closeSearch(): void {
  bar.classList.add("hidden");
  query = "";
  hits = [];
  current = -1;
  clearHighlights();
}

/** Go to the next (+1) or previous (-1) match. */
export function step(direction: 1 | -1): void {
  if (!isSearchOpen()) {
    openSearch();
    return;
  }
  if (!hits.length) return;
  current = (current + direction + hits.length) % hits.length;
  updateCount();
  highlightAll();
  reveal();
}

async function run(raw: string, fromCurrentPage: boolean): Promise<void> {
  const id = ++runId;
  query = raw.trim();
  if (!query) {
    hits = [];
    current = -1;
    updateCount();
    clearHighlights();
    return;
  }
  const texts = await viewer.pageTexts();
  if (id !== runId) return;
  hits = searchPages(texts, query);
  if (!hits.length) current = -1;
  else if (fromCurrentPage || current < 0) {
    const page = viewer.currentPage - 1;
    current = Math.max(
      0,
      hits.findIndex((hit) => hit.page >= page),
    );
  } else {
    current = Math.min(current, hits.length - 1);
  }
  updateCount();
  highlightAll();
  if (fromCurrentPage) reveal();
}

function updateCount(): void {
  const count = byId("search-count");
  count.textContent = !query
    ? ""
    : hits.length
      ? t("search.count", { current: current + 1, total: hits.length })
      : t("search.none");
  bar.classList.toggle("no-match", !!query && hits.length === 0);
}

// ---------- highlighting ----------

/** Text of a page's text layer with a map from offsets to text nodes. */
function layerText(layer: HTMLElement): { text: string; nodes: { node: Text; start: number }[] } {
  let text = "";
  const nodes: { node: Text; start: number }[] = [];
  const walker = document.createTreeWalker(layer, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if (n.nodeType === Node.TEXT_NODE) {
      nodes.push({ node: n as Text, start: text.length });
      text += n.textContent ?? "";
    } else if ((n as Element).tagName === "BR") {
      text += " ";
    }
  }
  return { text, nodes };
}

function locate(nodes: { node: Text; start: number }[], offset: number): [Text, number] | null {
  for (let i = nodes.length - 1; i >= 0; i--) {
    const entry = nodes[i];
    if (entry && entry.start <= offset) {
      return [entry.node, Math.min(offset - entry.start, entry.node.length)];
    }
  }
  return null;
}

function clearHighlights(): void {
  for (const el of document.querySelectorAll(".search-hit")) el.remove();
}

function highlightPage(pv: PageView): void {
  for (const el of pv.overlay.querySelectorAll(".search-hit")) el.remove();
  if (!query || !pv.textLayer) return;
  const onPage = hits.filter((hit) => hit.page === pv.index);
  if (!onPage.length) return;
  const { text, nodes } = layerText(pv.textLayer);
  const local = findAll(text, query);
  const page = pv.el.getBoundingClientRect();
  const range = document.createRange();
  for (const hit of onPage) {
    const match = local[hit.index];
    const start = match && locate(nodes, match[0]);
    const end = match && locate(nodes, match[1]);
    if (!start || !end) continue;
    range.setStart(...start);
    range.setEnd(...end);
    const boxes = [...range.getClientRects()].filter((r) => r.width > 0.5 && r.height > 0.5);
    const isCurrent = hits[current] === hit;
    for (const r of mergeLineRects(boxes.map((b) => normalizeBox(b, page)))) {
      const el = h("div", `search-hit${isCurrent ? " current" : ""}`);
      placeRect(el, r);
      pv.overlay.append(el);
    }
  }
}

function highlightAll(): void {
  clearHighlights();
  for (const pv of viewer.pages) highlightPage(pv);
}

/** Scroll the current match into view (exactly, once its text layer exists). */
function reveal(): void {
  const hit = hits[current];
  if (!hit) return;
  const pv = viewer.pages[hit.page];
  if (!pv) return;
  const el = pv.overlay.querySelector<HTMLElement>(".search-hit.current");
  if (el) {
    pendingScroll = false;
    const y = pv.el.offsetTop + el.offsetTop - viewer.scrollEl.clientHeight / 3;
    viewer.scrollEl.scrollTo({ top: y });
  } else {
    pendingScroll = true;
    viewer.scrollToPage(hit.page + 1);
  }
}

export function initSearch(v: Viewer): {
  onTextLayer: (pv: PageView) => void;
  onDocumentLoaded: () => void;
} {
  viewer = v;
  bar = byId("search");
  input = byId<HTMLInputElement>("search-input");

  let timer = 0;
  input.addEventListener("input", () => {
    clearTimeout(timer);
    timer = window.setTimeout(() => void run(input.value, true), 120);
  });
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      step(e.shiftKey ? -1 : 1);
    } else if (e.key === "Escape") {
      e.preventDefault();
      closeSearch();
    }
  });
  byId("search-next").addEventListener("click", () => step(1));
  byId("search-prev").addEventListener("click", () => step(-1));
  byId("search-close").addEventListener("click", closeSearch);
  onLocaleChange(updateCount);

  return {
    onTextLayer(pv) {
      if (!query) return;
      highlightPage(pv);
      if (pendingScroll && hits[current]?.page === pv.index) reveal();
    },
    onDocumentLoaded() {
      // text changed (reload) or another tab: search again, keep the position
      if (query) void run(query, false);
    },
  };
}
