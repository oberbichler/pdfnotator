/**
 * Optional left sidebar with page thumbnails and the PDF outline. Thumbnails
 * render lazily and are swapped in place after a reload, like the pages.
 */
import { onLocaleChange, t } from "../i18n";
import { loadBool, load as loadPref, save, saveBool } from "../lib/storage";
import { activeDoc, type NavTab, type State, state, subscribe, update } from "../store";
import type { OutlineItem, Viewer } from "../viewer";
import { byId, h } from "./dom";
import { icon } from "./icons";

const THUMB_WIDTH = 132;

let viewer: Viewer;
let thumbs: HTMLElement;
let outlineEl: HTMLElement;
let observer: IntersectionObserver;
let outlineVersion = -1;
let flatOutline: { item: OutlineItem; el: HTMLElement }[] = [];

// ---------- visibility ----------

export function toggleNavigator(force?: boolean): void {
  update((s) => {
    s.navOpen = force ?? !s.navOpen;
    saveBool("nav", s.navOpen);
  });
}

function setTab(tab: NavTab): void {
  update((s) => {
    s.navTab = tab;
    save("navTab", tab);
  });
}

// ---------- thumbnails ----------

function buildThumbs(): void {
  observer.disconnect();
  thumbs.replaceChildren(
    ...viewer.pages.map((pv) => {
      const item = h("button", "thumb");
      item.dataset.page = String(pv.index + 1);
      item.setAttribute("aria-label", t("nav.page", { page: pv.index + 1 }));
      const frame = h("div", "thumb-frame");
      frame.style.aspectRatio = `${pv.w} / ${pv.h}`;
      item.append(frame, h("span", "thumb-label", String(pv.index + 1)));
      item.addEventListener("click", () => viewer.scrollToPage(pv.index + 1));
      observer.observe(item);
      return item;
    }),
  );
  markCurrent(viewer.currentPage);
  renderBadges(state);
}

async function renderThumb(item: HTMLElement): Promise<void> {
  const version = viewer.version;
  if (item.dataset.version === String(version)) return;
  item.dataset.version = String(version);
  try {
    const canvas = await viewer.thumbnail(Number(item.dataset.page) - 1, THUMB_WIDTH);
    if (viewer.version !== version) return;
    const frame = item.querySelector(".thumb-frame");
    // replace the old preview only when the new one is ready — no flicker
    frame?.querySelector("canvas")?.remove();
    frame?.prepend(canvas);
  } catch {
    delete item.dataset.version; // retry when it becomes visible again
  }
}

/** Number of open comments per page as small badges. */
function renderBadges(s: State): void {
  const counts = new Map<number, number>();
  for (const c of activeDoc(s)?.comments ?? []) {
    if (c.page && c.status !== "resolved") counts.set(c.page, (counts.get(c.page) ?? 0) + 1);
  }
  for (const item of thumbs.querySelectorAll<HTMLElement>(".thumb")) {
    item.querySelector(".thumb-badge")?.remove();
    const n = counts.get(Number(item.dataset.page));
    if (n) {
      const badge = h("span", "thumb-badge", String(n));
      badge.title = t("nav.comments", { n });
      item.append(badge);
    }
  }
}

function markCurrent(page: number): void {
  for (const item of thumbs.querySelectorAll<HTMLElement>(".thumb")) {
    const current = Number(item.dataset.page) === page;
    item.classList.toggle("current", current);
    if (current && state.navOpen && state.navTab === "pages") item.scrollIntoView({ block: "nearest" });
  }
  highlightSection(page);
}

// ---------- outline ----------

async function buildOutline(): Promise<void> {
  const version = viewer.version;
  if (outlineVersion === version) return;
  outlineVersion = version;
  const items = await viewer.outline();
  if (viewer.version !== version) return;
  flatOutline = [];
  if (!items.length) {
    outlineEl.replaceChildren(h("p", "nav-empty", t("nav.noOutline")));
    return;
  }
  outlineEl.replaceChildren(outlineList(items, 0));
  highlightSection(viewer.currentPage);
}

function outlineList(items: OutlineItem[], depth: number): HTMLElement {
  const ul = h("ul", "outline-list");
  ul.setAttribute("role", depth === 0 ? "tree" : "group");
  for (const item of items) {
    const li = h("li");
    li.setAttribute("role", "treeitem");
    const row = h("div", "outline-row");
    row.style.paddingLeft = `${8 + depth * 14}px`;
    if (item.children.length) {
      const toggle = h("button", "outline-toggle");
      toggle.append(icon("chevron", 12));
      toggle.addEventListener("click", (e) => {
        e.stopPropagation();
        li.classList.toggle("collapsed");
      });
      row.append(toggle);
      if (depth >= 1) li.classList.add("collapsed");
    } else {
      row.append(h("span", "outline-spacer"));
    }
    const title = h("span", "outline-title", item.title);
    row.append(title);
    if (item.page >= 0) {
      row.append(h("span", "outline-page", String(item.page + 1)));
      row.addEventListener("click", () => viewer.scrollToPage(item.page + 1, item.frac));
    } else {
      row.classList.add("unresolved");
    }
    li.append(row);
    if (item.children.length) li.append(outlineList(item.children, depth + 1));
    ul.append(li);
    flatOutline.push({ item, el: row });
  }
  return ul;
}

/** Highlight the last outline entry that starts on or before `page`. */
function highlightSection(page: number): void {
  let current: HTMLElement | null = null;
  let best = -1;
  for (const { item, el } of flatOutline) {
    el.classList.remove("current");
    const pos = item.page + item.frac;
    if (item.page >= 0 && item.page < page && pos >= best) {
      best = pos;
      current = el;
    }
  }
  current?.classList.add("current");
}

// ---------- wiring ----------

function render(s: State): void {
  document.body.classList.toggle("nav-open", s.navOpen);
  const button = byId("btn-nav");
  button.classList.toggle("active", s.navOpen);
  button.setAttribute("aria-pressed", String(s.navOpen));
  for (const b of document.querySelectorAll<HTMLButtonElement>("#nav-tabs button")) {
    b.classList.toggle("active", b.dataset.navTab === s.navTab);
  }
  thumbs.classList.toggle("hidden", s.navTab !== "pages");
  outlineEl.classList.toggle("hidden", s.navTab !== "outline");
  if (s.navOpen && s.navTab === "outline") void buildOutline();
  renderBadges(s);
}

export function initNavigator(v: Viewer): {
  onDocumentLoaded: (isReload: boolean) => void;
  onPageChange: (page: number) => void;
} {
  viewer = v;
  thumbs = byId("thumbs");
  outlineEl = byId("outline");
  observer = new IntersectionObserver(
    (entries) => {
      for (const e of entries) if (e.isIntersecting) void renderThumb(e.target as HTMLElement);
    },
    { root: thumbs, rootMargin: "200px 0px" },
  );

  state.navOpen = loadBool("nav", false);
  const tab = loadPref("navTab");
  if (tab === "pages" || tab === "outline") state.navTab = tab;

  byId("btn-nav").addEventListener("click", () => toggleNavigator());
  for (const b of document.querySelectorAll<HTMLButtonElement>("#nav-tabs button")) {
    b.addEventListener("click", () => setTab(b.dataset.navTab as NavTab));
  }
  subscribe(render);
  onLocaleChange(() => {
    outlineVersion = -1;
    render(state);
  });
  render(state);

  return {
    onDocumentLoaded(isReload) {
      if (!isReload || thumbs.children.length !== viewer.pages.length) buildThumbs();
      else {
        // same pages: re-render visible thumbnails in place
        for (const item of thumbs.querySelectorAll<HTMLElement>(".thumb")) {
          const box = item.getBoundingClientRect();
          const list = thumbs.getBoundingClientRect();
          if (box.bottom > list.top - 200 && box.top < list.bottom + 200) void renderThumb(item);
        }
      }
      outlineVersion = -1;
      if (state.navOpen && state.navTab === "outline") void buildOutline();
    },
    onPageChange: markCurrent,
  };
}
