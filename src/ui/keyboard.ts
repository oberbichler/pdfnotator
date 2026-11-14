/** Global keyboard shortcuts. */
import { closeDocument, sendDrafts } from "../actions";
import { activeDoc, selectComment, state } from "../store";
import type { Viewer } from "../viewer";
import { closeComposer, isComposerOpen } from "./composer";
import { setTool, zoomIn, zoomOut } from "./dock";
import { isEditing } from "./dom";
import { toggleNavigator } from "./navigator";
import { closeSearch, isSearchOpen, openSearch, step } from "./search";
import { pdfSelection, selectionAction } from "./selection";
import { focusMessageBox, toggleSidebar } from "./sidebar";
import { cycleTab, selectTab } from "./tabs";

/** Shortcuts with ⌘ (macOS) / Ctrl, matched on `KeyboardEvent.code` where layouts differ. */
function modifierShortcut(e: KeyboardEvent, viewer: Viewer, openFile: () => void): (() => void) | undefined {
  if (e.altKey) return e.code === "KeyS" ? () => toggleNavigator() : undefined;
  if (/^Digit[1-9]$/.test(e.code)) {
    const n = Number(e.code.slice(5));
    return () => selectTab(n === 9 ? -1 : n - 1); // ⌘9 = last tab, like browsers
  }
  switch (e.key.toLowerCase()) {
    case "o":
      return openFile;
    case "f":
      return openSearch;
    case "g":
      return () => step(e.shiftKey ? -1 : 1);
    case "w":
      return () => {
        const doc = activeDoc();
        if (doc) void closeDocument(doc.id);
      };
    case "=":
    case "+":
      return () => zoomIn(viewer);
    case "-":
      return () => zoomOut(viewer);
    case "0":
      return () => viewer.fit();
    case "\\":
      return () => toggleSidebar();
    case "enter":
      return () => void sendDrafts();
    default:
      return undefined;
  }
}

export function initKeyboard(viewer: Viewer, openFile: () => void): void {
  document.addEventListener("keydown", (e) => {
    // Ctrl+Tab / Ctrl+Shift+Tab switch tabs on every platform
    if (e.ctrlKey && e.key === "Tab") {
      e.preventDefault();
      cycleTab(e.shiftKey ? -1 : 1);
      return;
    }
    if (e.metaKey || e.ctrlKey) {
      const handler = modifierShortcut(e, viewer, openFile);
      if (handler) {
        e.preventDefault();
        handler();
      }
      return;
    }
    if (isEditing(e.target) || e.altKey) return;

    const key = e.key.toLowerCase();
    const selection = pdfSelection();
    if (key === "escape") {
      if (isComposerOpen()) closeComposer();
      else if (isSearchOpen()) closeSearch();
      else if (selection) selection.removeAllRanges();
      else if (activeDoc()?.selectedId) selectComment(null);
      else if (state.tool === "area") setTool("text");
      return;
    }
    if (selection) {
      const action = {
        c: "comment",
        e: "replace",
        d: "delete",
        backspace: "delete",
        delete: "delete",
      } as const;
      const act = action[key as keyof typeof action];
      if (act) {
        e.preventDefault();
        selectionAction(act);
        return;
      }
    }
    switch (key) {
      case "r":
        setTool(state.tool === "area" ? "text" : "area");
        break;
      case "t":
      case "v":
        setTool("text");
        break;
      case "/":
      case "m":
        e.preventDefault();
        focusMessageBox();
        break;
    }
  });
}
