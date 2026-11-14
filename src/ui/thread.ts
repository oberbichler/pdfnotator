/** Sidebar thread: comment cards and agent replies in chronological order. */
import { resendComment } from "../actions";
import { type MessageKey, onLocaleChange, t } from "../i18n";
import { type Comment, type CommentKind, type CommentStatus, displayNumber } from "../lib/comments";
import {
  activeDoc,
  type DocState,
  removeComment,
  type State,
  selectComment,
  state,
  subscribe,
  update,
} from "../store";
import type { Viewer } from "../viewer";
import { byId, formatTime, h } from "./dom";
import { icon } from "./icons";
import { flashMarks, focusMarks } from "./marks";

const STATUS: Record<CommentStatus, { icon: string; label: MessageKey; title: MessageKey }> = {
  draft: { icon: "draft", label: "status.draft", title: "status.draftTitle" },
  sent: { icon: "check", label: "status.sent", title: "status.sentTitle" },
  delivered: { icon: "checks", label: "status.delivered", title: "status.deliveredTitle" },
  resolved: { icon: "done", label: "status.resolved", title: "status.resolvedTitle" },
};
const KIND_ICON: Record<CommentKind, string> = { text: "quote", area: "area", general: "general" };

let list: HTMLElement;
let empty: HTMLElement;
let viewer: Viewer;
let renderedCount = 0;
let renderedDoc: number | undefined;

function card(c: Comment, doc: DocState): HTMLElement {
  const el = h("article", `card ${c.status}`);
  el.classList.toggle("selected", c.id === doc.selectedId);
  el.dataset.id = c.id;
  el.tabIndex = 0;

  const head = h("div", "card-head");
  const kind = h("span", "kind");
  kind.append(
    icon(KIND_ICON[c.kind], 12),
    document.createTextNode(c.kind === "general" ? t("card.general") : t("card.page", { page: c.page })),
  );
  const status = h("span", "st");
  status.title = t(STATUS[c.status].title);
  status.append(icon(STATUS[c.status].icon, 13), h("span", "st-label", t(STATUS[c.status].label)));
  const actions = h("span", "actions");
  if (c.status === "sent" || c.status === "delivered") {
    const resend = h("button", "icon-btn sm");
    resend.title = t("card.resend");
    resend.setAttribute("aria-label", t("card.resend"));
    resend.append(icon("resend", 13));
    resend.addEventListener("click", (e) => {
      e.stopPropagation();
      void resendComment(doc, c.id);
    });
    actions.append(resend);
  }
  if (c.status === "draft") {
    const del = h("button", "icon-btn sm");
    del.title = t("card.delete");
    del.setAttribute("aria-label", t("card.delete"));
    del.append(icon("trash", 13));
    del.addEventListener("click", (e) => {
      e.stopPropagation();
      removeComment(c.id);
    });
    actions.append(del);
  }
  head.append(h("span", "pin", displayNumber(c.id)), kind, h("span", "spacer"), actions, status);
  el.append(head);

  if (c.kind === "area") {
    if (c.image) {
      const img = h("img");
      img.alt = c.areaText ?? "";
      img.src = `data:image/png;base64,${c.image}`;
      el.append(img);
    } else {
      el.append(h("div", "shimmer"));
    }
  } else if (c.quote) {
    el.append(h("blockquote", undefined, c.quote));
  }

  const text = h("div", c.comment ? "text" : "text empty", c.comment || t("card.noComment"));
  if (c.status === "draft") {
    text.contentEditable = "true";
    text.addEventListener("click", (e) => e.stopPropagation());
    text.addEventListener("focus", () => {
      if (!c.comment) text.textContent = "";
    });
    text.addEventListener("blur", () => update(() => (c.comment = text.textContent?.trim() ?? "")));
  }
  el.append(text);

  const toggle = () => {
    const id = doc.selectedId === c.id ? null : c.id;
    selectComment(id);
    if (id && c.page) {
      const r = c.rect ?? c.rects?.[0];
      viewer.scrollToPage(c.page, r ? r[1] : 0);
      flashMarks(c.id);
    }
  };
  el.addEventListener("click", toggle);
  el.addEventListener("keydown", (e) => {
    if (e.target === el && (e.key === "Enter" || e.key === " ")) {
      e.preventDefault();
      toggle();
    }
  });
  el.addEventListener("mouseenter", () => focusMarks(c.id, true));
  el.addEventListener("mouseleave", () => focusMarks(c.id, false));
  return el;
}

function agentReply(message: string, resolved: string[], createdAt: number): HTMLElement {
  const row = h("div", "agent-msg");
  const avatar = h("div", "avatar");
  avatar.append(icon("sparkles", 13));
  const body = h("div", "body");
  const who = h("div", "who", t("agent.name"));
  who.append(h("time", undefined, formatTime(createdAt)));
  body.append(who);
  if (message) body.append(h("div", "msg", message));
  if (resolved.length) {
    const refs = h("div", "refs");
    for (const id of resolved) {
      const ref = h("span", "ref");
      ref.append(icon("check", 11), document.createTextNode(displayNumber(id)));
      refs.append(ref);
    }
    body.append(refs);
  }
  row.append(avatar, body);
  return row;
}

export function renderThread(s: State): void {
  // don't rebuild while a draft card is being edited
  if (list.contains(document.activeElement) && document.activeElement?.matches("[contenteditable=true]"))
    return;
  const atBottom = list.scrollHeight - list.scrollTop - list.clientHeight < 40;
  for (const n of list.querySelectorAll(".card, .agent-msg")) n.remove();

  const doc = activeDoc(s);
  const entries: { at: number; node: HTMLElement }[] = doc
    ? [
        ...doc.comments
          .filter((c) => !(s.hideResolved && c.status === "resolved"))
          .map((c) => ({ at: c.createdAt, node: card(c, doc) })),
        ...doc.agentMessages.map((m) => ({
          at: m.createdAt,
          node: agentReply(m.message, m.resolved, m.createdAt),
        })),
      ].sort((a, b) => a.at - b.at)
    : [];

  list.append(...entries.map((e) => e.node));
  empty.classList.toggle("hidden", entries.length > 0);
  // switching tabs or new entries → show the latest
  if (atBottom || entries.length > renderedCount || doc?.id !== renderedDoc)
    list.scrollTop = list.scrollHeight;
  renderedCount = entries.length;
  renderedDoc = doc?.id;

  const open = doc?.comments.filter((c) => c.status !== "resolved").length ?? 0;
  byId("count").textContent = open ? String(open) : "";
}

export function initThread(v: Viewer): void {
  viewer = v;
  list = byId("thread");
  empty = byId("thread-empty");
  subscribe(renderThread);
  onLocaleChange(() => renderThread(state));
}
