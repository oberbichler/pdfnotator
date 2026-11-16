// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { applyTranslations, setLocale } from "../i18n";
import { type State, state } from "../store";
import { docStatus } from "../test-utils";
import { agentState, SNIPPETS, setExecutable } from "./status";

const withDoc = (
  patch: Parameters<typeof docStatus>[1] = {},
  agentSeen = false,
  serverError: string | null = null,
): State => ({
  ...state,
  serverError,
  activeId: 1,
  docs: [
    { ...docStatus(1, patch), comments: [], agentMessages: [], selectedId: null, agentSeen, view: null },
  ],
});

describe("agent status", () => {
  afterEach(() => setLocale("en"));

  it("derives the chip state", () => {
    expect(agentState(withDoc()).label).toBe("No agent");
    expect(agentState(withDoc({ listeners: 1 })).cls).toBe("on");
    expect(agentState(withDoc({ listeners: 1 })).label).toBe("Agent listening");
    expect(agentState(withDoc({}, true)).label).toBe("Agent working");
    expect(agentState(withDoc({ listeners: 1 }, false, "port busy")).hint).toBe("port busy");
    expect(agentState({ ...state, docs: [], activeId: null }).label).toBe("No agent");
  });

  it("names the listening agents", () => {
    const s = withDoc({ listeners: 2, agents: ["GitHub Copilot CLI", "Claude Code"] });
    expect(agentState(s).label).toBe("GitHub Copilot CLI, Claude Code listening");
  });

  it("builds the start prompt from the source file", () => {
    const s = withDoc({ source: "/p/doc.typ", path: "/p/doc.pdf" });
    expect(SNIPPETS.prompt(s)).toBe("open /p/doc.typ in pdfnotator");
  });

  it("prefers the stdio bridge when the executable is known", () => {
    const s = withDoc();
    setExecutable(null);
    expect(JSON.parse(SNIPPETS.vscode(s)).servers.pdfnotator).toEqual({
      type: "http",
      url: "http://127.0.0.1:4747/mcp",
    });
    setExecutable("/Applications/PDFnotator.app/Contents/MacOS/pdfnotator");
    expect(JSON.parse(SNIPPETS.vscode(s)).servers.pdfnotator).toEqual({
      type: "stdio",
      command: "/Applications/PDFnotator.app/Contents/MacOS/pdfnotator",
      args: ["--mcp"],
    });
    expect(SNIPPETS.copilotCli(s)).toBe(
      'copilot mcp add pdfnotator -- "/Applications/PDFnotator.app/Contents/MacOS/pdfnotator" --mcp',
    );
    expect(SNIPPETS.claude(s)).toBe(
      'claude mcp add --scope user pdfnotator -- "/Applications/PDFnotator.app/Contents/MacOS/pdfnotator" --mcp',
    );
  });
});

describe("applyTranslations", () => {
  afterEach(() => setLocale("en"));

  it("translates text, attributes and appends shortcuts", () => {
    document.body.innerHTML = `
      <h1 data-i18n="sidebar.title"></h1>
      <button data-i18n-title="zoom.in" data-shortcut="⌘+"></button>
      <textarea data-i18n-placeholder="general.placeholder"></textarea>`;
    setLocale("de");
    applyTranslations();
    expect(document.querySelector("h1")?.textContent).toBe("Feedback");
    expect(document.querySelector("button")?.title).toBe("Vergrößern (⌘+)");
    expect(document.querySelector("textarea")?.placeholder).toBe("Nachricht an den Agenten …");
    expect(document.documentElement.lang).toBe("de");
  });
});
