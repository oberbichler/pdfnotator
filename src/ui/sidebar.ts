/** Sidebar controls: message box, instant-send toggle, drafts bar, filters. */
import { sendDrafts } from "../actions";
import { onLocaleChange, t } from "../i18n";
import { loadBool, saveBool } from "../lib/storage";
import { activeDoc, addComment, drafts, nextCommentId, type State, state, subscribe, update } from "../store";
import { autoGrow, byId } from "./dom";
import { icon } from "./icons";

let input: HTMLTextAreaElement;

export function toggleSidebar(force?: boolean): void {
  const hidden = document.body.classList.toggle("sidebar-hidden", force === undefined ? undefined : !force);
  saveBool("sidebar", !hidden);
}

export function focusMessageBox(): void {
  toggleSidebar(true);
  input.focus();
}

/** Send the typed message (if any) together with all drafts. */
function sendMessage(): void {
  const text = input.value.trim();
  if (text) {
    addComment({
      id: nextCommentId(),
      kind: "general",
      page: 0,
      comment: text,
      status: "draft",
      createdAt: Date.now(),
    });
    input.value = "";
    autoGrow(input);
  }
  void sendDrafts(activeDoc());
}

function render(s: State): void {
  const live = byId("live-toggle");
  live.classList.toggle("on", s.live);
  live.setAttribute("aria-pressed", String(s.live));
  live.title = t(s.live ? "live.on" : "live.off");

  const n = drafts(activeDoc(s)).length;
  byId("draftbar").classList.toggle("hidden", n === 0);
  byId("draft-count").textContent = t("drafts.count", { n });
  byId<HTMLButtonElement>("btn-send").disabled = !input.value.trim() && n === 0;

  const filter = byId("btn-toggle-resolved");
  filter.replaceChildren(icon(s.hideResolved ? "eyeOff" : "eye", 15));
  filter.classList.toggle("active", s.hideResolved);
  const label = t(s.hideResolved ? "resolved.show" : "resolved.hide");
  filter.title = label;
  filter.setAttribute("aria-label", label);
}

export function initSidebar(): void {
  input = byId<HTMLTextAreaElement>("general-input");
  state.live = loadBool("live", true);
  if (!loadBool("sidebar", true)) document.body.classList.add("sidebar-hidden");

  byId("btn-sidebar").addEventListener("click", () => toggleSidebar());
  byId("live-toggle").addEventListener("click", () =>
    update((s) => {
      s.live = !s.live;
      saveBool("live", s.live);
    }),
  );
  byId("btn-toggle-resolved").addEventListener("click", () =>
    update((s) => (s.hideResolved = !s.hideResolved)),
  );
  byId("btn-send").addEventListener("click", sendMessage);
  byId("btn-send-all").addEventListener("click", () => void sendDrafts(activeDoc()));
  input.addEventListener("input", () => {
    autoGrow(input);
    render(state);
  });
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      sendMessage();
    }
  });

  subscribe(render);
  onLocaleChange(() => render(state));
  render(state);
}
