/** Transient notifications at the top of the stage. */
import { byId, h } from "./dom";
import { icon } from "./icons";

export type ToastKind = "info" | "ok" | "error" | "agent";

const ICON: Record<ToastKind, string> = { info: "comment", ok: "check", error: "x", agent: "sparkles" };
const DURATION: Record<ToastKind, number> = { info: 2200, ok: 2200, error: 5000, agent: 6000 };

export function toast(message: string, kind: ToastKind = "info"): void {
  const el = h("div", `toast glass ${kind}`);
  el.setAttribute("role", kind === "error" ? "alert" : "status");
  el.append(icon(ICON[kind], 14), h("span", undefined, message));
  byId("toasts").append(el);
  setTimeout(() => el.classList.add("out"), DURATION[kind]);
  setTimeout(() => el.remove(), DURATION[kind] + 400);
}
