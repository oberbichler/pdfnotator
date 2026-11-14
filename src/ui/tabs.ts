/** Document tabs in the title bar. Each tab shows its agent's state. */
import { closeDocument } from "../actions";
import { onLocaleChange, t } from "../i18n";
import { activate, type DocState, type State, state, subscribe } from "../store";
import { byId, h } from "./dom";
import { icon } from "./icons";

let root: HTMLElement;

const fileName = (doc: DocState) => (doc.source ?? doc.path ?? "").split(/[\\/]/).pop() || t("app.name");

/** Agent activity of a tab: listening, working or none. */
export function tabActivity(doc: DocState): "on" | "busy" | "" {
  if (doc.listeners > 0) return "on";
  return doc.agentSeen ? "busy" : "";
}

function tab(doc: DocState, active: boolean): HTMLElement {
  const el = h("div", `tab${active ? " active" : ""}`);
  el.setAttribute("role", "tab");
  el.setAttribute("aria-selected", String(active));
  el.tabIndex = active ? 0 : -1;
  el.dataset.doc = String(doc.id);
  const live = doc.source ? t("live.typst", { file: fileName(doc) }) : t("live.pdf");
  el.title = `${doc.source ?? doc.path ?? ""}\n${live}`;

  const activity = tabActivity(doc);
  const dot = h("span", `tab-dot ${activity}`);
  dot.title = activity === "on" ? t("agent.listening") : activity === "busy" ? t("agent.working") : "";
  const name = h("span", "tab-name", fileName(doc));
  el.append(dot, name);
  if (doc.compileError) el.append(icon("alert", 12, "tab-error"));
  if (doc.pending > 0 && doc.listeners === 0) {
    const badge = h("span", "tab-badge", String(doc.pending));
    badge.title = t("agent.pending", { n: doc.pending });
    el.append(badge);
  }
  const close = h("button", "tab-close");
  close.title = t("tab.close");
  close.setAttribute("aria-label", t("tab.close"));
  close.append(icon("x", 12));
  close.addEventListener("click", (e) => {
    e.stopPropagation();
    void closeDocument(doc.id);
  });
  el.append(close);

  el.addEventListener("click", () => activate(doc.id));
  el.addEventListener("auxclick", (e) => {
    if (e.button === 1) void closeDocument(doc.id); // middle click
  });
  return el;
}

function render(s: State): void {
  root.replaceChildren(...s.docs.map((d) => tab(d, d.id === s.activeId)));
  root.classList.toggle("empty", s.docs.length === 0);
  root.querySelector(".tab.active")?.scrollIntoView({ block: "nearest", inline: "nearest" });
}

/** Briefly highlight the active tab after its PDF was reloaded. */
export function pulseActiveTab(): void {
  const el = root.querySelector(".tab.active .tab-name");
  if (!el) return;
  el.classList.remove("reloaded");
  void (el as HTMLElement).offsetWidth; // restart the animation
  el.classList.add("reloaded");
}

/** Select the tab `offset` positions away from the active one (wraps around). */
export function cycleTab(offset: number): void {
  const n = state.docs.length;
  if (!n) return;
  const i = state.docs.findIndex((d) => d.id === state.activeId);
  activate(state.docs[(((i + offset) % n) + n) % n]?.id ?? null);
}

export function selectTab(index: number): void {
  const doc = index < 0 ? state.docs.at(-1) : state.docs[index];
  if (doc) activate(doc.id);
}

export function initTabs(openFile: () => void): void {
  root = byId("tabs");
  root.setAttribute("role", "tablist");
  byId("btn-new-tab").addEventListener("click", openFile);
  subscribe(render);
  onLocaleChange(() => render(state));
  render(state);
}
