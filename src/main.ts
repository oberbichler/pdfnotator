/**
 * Entry point: wires the PDF viewer, the UI modules and the backend events.
 *
 * Data flow: backend events / user input → actions → `store` → subscribed
 * renderers. The viewer shows the active tab; switching tabs saves and
 * restores zoom and scroll position per document.
 */
import "pdfjs-dist/web/pdf_viewer.css";
import "./styles.css";

import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { openDocument } from "./actions";
import { backend, isDesktop, on } from "./backend";
import { t } from "./i18n";
import {
  activate,
  activeDoc,
  addAgentMessage,
  applyServerStatus,
  docById,
  markAgentSeen,
  removeDoc,
  setStatus,
  state,
  subscribe,
} from "./store";
import { attachAreaTool } from "./ui/area-tool";
import { closeComposer, initComposer } from "./ui/composer";
import { initDock, showScale } from "./ui/dock";
import { byId, isMac, shortcut } from "./ui/dom";
import { hydrateIcons } from "./ui/icons";
import { initKeyboard } from "./ui/keyboard";
import { initLanguagePicker, initLocale } from "./ui/language";
import { initMarks, renderMarks } from "./ui/marks";
import { initNavigator } from "./ui/navigator";
import { initResize } from "./ui/resize";
import { initSearch } from "./ui/search";
import { initSelection, isSelecting } from "./ui/selection";
import { initSidebar } from "./ui/sidebar";
import { attachSketchTool, initSketchTool } from "./ui/sketch-tool";
import { initStatus, setExecutable } from "./ui/status";
import { initTablet } from "./ui/tablet";
import { initTabs, pulseActiveTab } from "./ui/tabs";
import { initThread } from "./ui/thread";
import { toast } from "./ui/toast";
import { selectionTouches, Viewer } from "./viewer";

// ---------- static markup ----------

document.body.classList.toggle("macos", isMac);
document.body.classList.toggle("remote", !isDesktop);
for (const el of document.querySelectorAll<HTMLElement>("[data-shortcut]")) {
  el.dataset.shortcut = shortcut(el.dataset.shortcut ?? "");
}
for (const el of document.querySelectorAll<HTMLElement>("kbd[data-kbd]")) {
  el.textContent = shortcut(el.dataset.kbd ?? "");
}
hydrateIcons();
initLocale();

// ---------- viewer ----------

/** Document currently shown in the viewer (may lag behind `state.activeId` while loading). */
let shownDoc: number | null = null;
let reportTimer = 0;

const viewer = new Viewer(byId("viewer-scroll"), byId("pages"), {
  onPageChange(page, count) {
    dock.onPageChange(page, count);
    navigatorPanel.onPageChange(page);
    const doc = shownDoc;
    if (doc === null) return;
    clearTimeout(reportTimer);
    reportTimer = window.setTimeout(() => backend.setView(doc, page, count).catch(() => {}), 250);
  },
  onPageCreated: (pv) => {
    attachAreaTool(viewer, pv);
    attachSketchTool(pv);
  },
  onScaleChange: showScale,
  onTextLayer: (pv) => search.onTextLayer(pv),
  onDocumentLoaded(isReload) {
    navigatorPanel.onDocumentLoaded(isReload);
    search.onDocumentLoaded();
  },
});
// keep the text layer stable while the user selects text, so a reload never kills a selection
viewer.textSwapBlocked = (layer) => isSelecting() || selectionTouches(layer);

const dock = initDock(viewer);
const navigatorPanel = initNavigator(viewer);
const search = initSearch(viewer);
initComposer({ afterClose: () => viewer.flushTextSwap() });
initSelection(viewer);
initMarks(viewer);
initSketchTool(viewer);
initThread(viewer);
initSidebar();
initResize();
initStatus();
initTablet();
initTabs(() => void pickFile());
initLanguagePicker();
initKeyboard(viewer, () => void pickFile());

// ---------- documents ----------

const MAX_LOAD_ATTEMPTS = 10;
let loadGeneration = 0;

/** Load a document's PDF into the viewer. Retries quietly: the compiler may still be writing it. */
async function loadDocument(id: number, isReload: boolean): Promise<void> {
  const generation = ++loadGeneration;
  for (let attempt = 1; attempt <= MAX_LOAD_ATTEMPTS; attempt++) {
    try {
      const data = await backend.readPdf(id);
      if (generation !== loadGeneration || shownDoc !== id) return;
      await viewer.load(data, isReload, isReload ? null : docById(id)?.view);
      if (generation !== loadGeneration) return;
      document.body.classList.add("has-doc");
      renderMarks(viewer, state);
      if (isReload) pulseActiveTab();
      return;
    } catch (error) {
      if (generation !== loadGeneration) return;
      if (attempt === MAX_LOAD_ATTEMPTS) {
        toast(t("error.loadPdf", { error: String(error) }), "error");
        return;
      }
      await new Promise((r) => setTimeout(r, 120 * attempt));
    }
  }
}

/** Show the active tab in the viewer (saving the view of the previous one). */
function showActive(): void {
  const id = state.activeId;
  if (id === shownDoc) return;
  const previous = shownDoc === null ? undefined : docById(shownDoc);
  if (previous) previous.view = viewer.viewState();
  shownDoc = id;
  closeComposer();
  if (id === null) {
    loadGeneration++;
    viewer.clear();
    document.body.classList.remove("has-doc");
    return;
  }
  void loadDocument(id, false);
}
subscribe(showActive);

async function pickFile(): Promise<void> {
  if (!isDesktop) return; // a tablet shows the computer's tabs; files are opened there
  const path = await openDialog({
    multiple: false,
    filters: [{ name: t("dialog.filter"), extensions: ["typ", "pdf"] }],
  });
  if (typeof path === "string") void openDocument(path);
}
byId("btn-open").addEventListener("click", () => void pickFile());

/** Re-read the backend status (e.g. before focusing a tab we don't know yet). */
const refreshStatus = () => backend.getStatus().then(applyServerStatus);

// ---------- backend events ----------

void on("status", applyServerStatus);
void on("focus", ({ doc }) => {
  if (docById(doc)) activate(doc);
  else void refreshStatus().then(() => activate(doc));
});
void on("pdf-opened", ({ doc }) => {
  if (doc === shownDoc) void loadDocument(doc, true); // re-opened: refresh in place
});
void on("pdf-changed", ({ doc }) => {
  if (doc === shownDoc) void loadDocument(doc, true);
});
void on("doc-closed", ({ doc }) => removeDoc(doc));
void on("feedback-delivered", ({ doc, ids }) => {
  markAgentSeen(doc);
  setStatus(doc, ids, "delivered");
});
void on("agent-message", ({ doc, message, resolved }) => {
  addAgentMessage(doc, message, resolved);
  const hidden = document.body.classList.contains("sidebar-hidden") || document.hidden;
  if (message && (hidden || doc !== activeDoc()?.id)) {
    const name = (docById(doc)?.path ?? "").split(/[\\/]/).pop();
    toast(doc === activeDoc()?.id || !name ? message : `${name}: ${message}`, "agent");
  }
});

if (isDesktop) {
  void getCurrentWebview().onDragDropEvent(({ payload }) => {
    document.body.classList.toggle("drag-over", payload.type === "enter" || payload.type === "over");
    if (payload.type !== "drop") return;
    const files = payload.paths.filter((p) => /\.(pdf|typ)$/i.test(p));
    if (!files.length) toast(t("error.unsupportedDrop"), "error");
    for (const file of files) void openDocument(file);
  });
}

// ---------- startup ----------

void backend.appInfo().then((info) => {
  setExecutable(info.executable);
  const mode = info.demo;
  if (mode) void import("./demo").then((demo) => demo.runDemo(viewer, mode === "animate"));
});
void refreshStatus();
