/**
 * Text selection on the PDF text layer. Releasing the mouse after selecting
 * text opens the composer right away with the cursor in the input field —
 * comment, replace or delete without an extra click.
 */
import { commitDraft } from "../actions";
import { t } from "../i18n";
import { type CommentDraft, type ComposerMode, normalizeQuote } from "../lib/comments";
import { mergeLineRects, type NRect, normalizeBox } from "../lib/geometry";
import { contextAround } from "../lib/search";
import { nextCommentId, state } from "../store";
import type { PageView, Viewer } from "../viewer";
import { hasUnsavedText, isComposerOpen, openComposer } from "./composer";
import { byId, h } from "./dom";
import { placeRect } from "./marks";

let viewer: Viewer;
/** Mouse button is down inside a text layer (selection in progress). */
let selecting = false;

export const isSelecting = (): boolean => selecting;

/** The current selection if it lies on a PDF page. */
export function pdfSelection(): Selection | null {
  const sel = window.getSelection();
  if (!sel || sel.isCollapsed || sel.rangeCount === 0 || !sel.toString().trim()) return null;
  return viewer.pageOf(sel.anchorNode) ? sel : null;
}

/** Text of the page around the selection, so the agent can locate ambiguous quotes. */
function selectionContext(pv: PageView, range: Range, quote: string): string | undefined {
  const layer = pv.textLayer;
  if (!layer?.contains(range.startContainer)) return undefined;
  const before = document.createRange();
  before.setStart(layer, 0);
  before.setEnd(range.startContainer, range.startOffset);
  return contextAround(layer.textContent ?? "", before.toString().length, quote.length);
}

/** Keep the selected text visibly highlighted while the composer has focus. */
function showPending(pv: PageView, rects: NRect[]): () => void {
  const marks = rects.map((r) => {
    const el = h("div", "pending-mark");
    placeRect(el, r);
    pv.overlay.append(el);
    return el;
  });
  return () => {
    for (const m of marks) m.remove();
  };
}

/** Act on the current text selection (default: open the composer for a comment). */
export function selectionAction(action: ComposerMode = "comment"): void {
  const sel = pdfSelection();
  if (!sel) return;
  const range = sel.getRangeAt(0);
  const pv = viewer.pageOf(range.startContainer);
  if (!pv) return;
  const page = pv.el.getBoundingClientRect();
  const lineBoxes = [...range.getClientRects()]
    .filter((r) => r.width > 1 && r.height > 1 && r.height < page.height * 0.1)
    .filter((r) => r.bottom > page.top && r.top < page.bottom)
    .map((r) => normalizeBox(r, page));
  const quote = normalizeQuote(sel.toString());
  const rects = mergeLineRects(lineBoxes);
  const draft: CommentDraft = {
    id: nextCommentId(),
    kind: "text",
    page: pv.index + 1,
    quote,
    rects,
    context: selectionContext(pv, range, sel.toString()),
  };

  if (action === "delete") {
    sel.removeAllRanges();
    void commitDraft(draft, t("comment.deleteText"));
    return;
  }
  const boxes = range.getClientRects();
  const anchor = boxes[boxes.length - 1] ?? range.getBoundingClientRect();
  openComposer(anchor, draft, {
    label: t("card.page", { page: draft.page }),
    quote,
    mode: action,
    onClose: showPending(pv, rects),
  });
}

export function initSelection(v: Viewer): void {
  viewer = v;

  document.addEventListener("mousedown", (e) => {
    const target = e.target as HTMLElement;
    if (byId("composer").contains(target)) return;
    const layer = target.closest?.(".textLayer");
    if (layer && (state.tool === "text" || state.tool === "select")) {
      selecting = true;
      layer.classList.add("selecting"); // pdf.js CSS: keeps selection smooth
    }
  });

  document.addEventListener("mouseup", () => {
    if (!selecting) return;
    selecting = false;
    for (const el of document.querySelectorAll(".textLayer.selecting")) el.classList.remove("selecting");
    setTimeout(() => {
      viewer.flushTextSwap();
      // never throw away a comment that is still being typed
      if (isComposerOpen() && hasUnsavedText()) return;
      if (state.tool === "text") selectionAction("comment"); // the select tool only marks
    }, 0);
  });

  document.addEventListener("selectionchange", () => {
    if (!selecting && !pdfSelection()) viewer.flushTextSwap();
  });
}
