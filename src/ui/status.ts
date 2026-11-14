/** Agent status chip, connect menu and compile-error banner (for the active tab). */
import { type MessageKey, onLocaleChange, t } from "../i18n";
import { activeDoc, type State, state, subscribe } from "../store";
import { byId, h } from "./dom";
import { icon } from "./icons";
import { toast } from "./toast";

interface AgentState {
  cls: string;
  icon: string;
  label: MessageKey;
  hint: string;
}

/** Derive what the agent of the active tab is currently doing. */
export function agentState(s: State): AgentState {
  const doc = activeDoc(s);
  if (s.serverError) return { cls: "err", icon: "unplug", label: "agent.serverError", hint: s.serverError };
  if (doc && doc.listeners > 0) {
    return { cls: "on", icon: "listening", label: "agent.listening", hint: t("agent.listeningHint") };
  }
  if (doc?.agentSeen)
    return { cls: "busy", icon: "spinner", label: "agent.working", hint: t("agent.workingHint") };
  return { cls: "", icon: "plug", label: "agent.none", hint: t("agent.noneHint") };
}

const mcpUrl = (s: State) => `http://127.0.0.1:${s.port}/mcp`;

/** Path of the app binary; set at startup (used for the stdio MCP config). */
let executable: string | null = null;
export const setExecutable = (path: string | null) => {
  executable = path;
};

/**
 * Text copied by the connect menu entries. The stdio variant is preferred:
 * the agent then starts the viewer on demand. Without a known executable
 * path the HTTP endpoint is used.
 */
export const SNIPPETS = {
  url: mcpUrl,
  // English on purpose: this is the trigger phrase the agent's tool description knows
  prompt: (s: State) => {
    const doc = activeDoc(s);
    return `open ${doc?.source ?? doc?.path ?? "the document"} in pdfnotator`;
  },
  vscode: (s: State) =>
    JSON.stringify(
      {
        servers: {
          pdfnotator: executable
            ? { type: "stdio", command: executable, args: ["--mcp"] }
            : { type: "http", url: mcpUrl(s) },
        },
      },
      null,
      2,
    ),
  copilotCli: (s: State) =>
    executable
      ? `copilot mcp add pdfnotator -- "${executable}" --mcp`
      : `copilot mcp add --transport http pdfnotator ${mcpUrl(s)}`,
  claude: (s: State) =>
    executable
      ? `claude mcp add --scope user pdfnotator -- "${executable}" --mcp`
      : `claude mcp add --transport http --scope user pdfnotator ${mcpUrl(s)}`,
} as const;

function render(s: State): void {
  const st = agentState(s);
  const chip = byId("agent-chip");
  chip.className = `chip ${st.cls}`;
  chip.title = st.hint;
  chip.querySelector(".chip-icon")?.replaceChildren(icon(st.icon, 14));
  const label = chip.querySelector(".chip-label");
  if (label) label.textContent = t(st.label);
  chip.querySelector(".chip-badge")?.remove();
  const doc = activeDoc(s);
  if (doc && doc.pending > 0 && doc.listeners === 0) {
    const badge = h("span", "chip-badge", String(doc.pending));
    badge.title = t("agent.pending", { n: doc.pending });
    chip.append(badge);
  }

  const head = byId("connect-menu").querySelector(".menu-head");
  if (head) head.className = `menu-head ${st.cls}`;
  byId("menu-status-icon").replaceChildren(icon(st.icon, 16));
  byId("menu-status-title").textContent = t(st.label);
  byId("menu-status-sub").textContent = st.hint;
  byId("mcp-url").textContent = mcpUrl(s);

  const error = doc?.compileError ?? null;
  byId("compile-error").classList.toggle("hidden", !error);
  byId("compile-error-text").textContent = error ?? "";
}

function initConnectMenu(): void {
  const chip = byId("agent-chip");
  const menu = byId("connect-menu");
  const close = () => {
    menu.classList.add("hidden");
    chip.setAttribute("aria-expanded", "false");
  };
  chip.addEventListener("click", (e) => {
    e.stopPropagation();
    if (!menu.classList.contains("hidden")) return close();
    menu.classList.remove("hidden");
    chip.setAttribute("aria-expanded", "true");
    const r = chip.getBoundingClientRect();
    menu.style.top = `${r.bottom + 6}px`;
    menu.style.left = `${Math.max(8, Math.min(r.right - menu.offsetWidth, window.innerWidth - menu.offsetWidth - 8))}px`;
  });
  document.addEventListener("mousedown", (e) => {
    if (!menu.contains(e.target as Node) && !chip.contains(e.target as Node)) close();
  });
  document.addEventListener("keydown", (e) => e.key === "Escape" && close());

  for (const b of menu.querySelectorAll<HTMLButtonElement>("[data-copy]")) {
    b.addEventListener("click", async () => {
      const key = b.dataset.copy as keyof typeof SNIPPETS;
      try {
        await navigator.clipboard.writeText(SNIPPETS[key](state));
        toast(t(key === "prompt" ? "connect.promptCopied" : "connect.copied"), "ok");
      } catch (error) {
        toast(t("connect.copyFailed", { error: String(error) }), "error");
      }
      close();
    });
  }
}

export function initStatus(): void {
  initConnectMenu();
  subscribe(render);
  onLocaleChange(() => render(state));
  render(state);
}
