// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";

const commitDraft = vi.fn(async () => {});
vi.mock("../actions", () => ({ commitDraft: (...args: unknown[]) => commitDraft(...(args as [])) }));

const { closeComposer, initComposer, isComposerOpen, openComposer } = await import("./composer");

const MARKUP = `
  <div id="composer" class="hidden">
    <div id="cmp-modes">
      <button data-mode="comment"></button><button data-mode="replace"></button><button data-mode="delete"></button>
    </div>
    <span id="cmp-kind"></span><button id="cmp-close"></button>
    <div id="cmp-ctx"></div>
    <textarea id="cmp-input"></textarea>
    <span id="cmp-action"></span><button id="cmp-send"></button>
  </div>`;

const anchor = new DOMRect(10, 10, 100, 20);
const input = () => document.getElementById("cmp-input") as HTMLTextAreaElement;
const press = (key: string, shiftKey = false) =>
  input().dispatchEvent(new KeyboardEvent("keydown", { key, shiftKey, bubbles: true }));
const draft = (id: string) => ({ id, kind: "text" as const, page: 1, quote: "old text" });

describe("composer", () => {
  beforeEach(() => {
    document.body.innerHTML = MARKUP;
    commitDraft.mockClear();
    initComposer({ afterClose: () => {} });
  });

  it("starts empty after a previous comment was sent", () => {
    openComposer(anchor, draft("c1"), { label: "p. 1", quote: "old text" });
    input().value = "first comment";
    press("Enter");
    expect(commitDraft).toHaveBeenCalledWith(expect.objectContaining({ id: "c1" }), "first comment");
    expect(isComposerOpen()).toBe(false);

    openComposer(anchor, draft("c2"), { label: "p. 1", quote: "old text" });
    expect(input().value).toBe("");
  });

  it("starts empty after a cancelled comment", () => {
    openComposer(anchor, draft("c1"), { label: "p. 1" });
    input().value = "typed but cancelled";
    closeComposer();
    openComposer(anchor, draft("c2"), { label: "p. 1" });
    expect(input().value).toBe("");
  });

  it("prefills replace mode with the quote and cycles modes with Tab", () => {
    openComposer(anchor, draft("c1"), { label: "p. 1", quote: "old text", mode: "replace" });
    expect(input().value).toBe("old text");
    press("Tab"); // → delete
    expect(input().value).toBe("");
    expect(document.getElementById("composer")?.classList.contains("mode-delete")).toBe(true);
    press("Tab", true); // ← replace
    expect(input().value).toBe("old text");
  });

  it("sends nothing when replace text is unchanged", () => {
    openComposer(anchor, draft("c1"), { label: "p. 1", quote: "old text", mode: "replace" });
    press("Enter");
    expect(commitDraft).not.toHaveBeenCalled();
    expect(isComposerOpen()).toBe(true);
  });

  it("hides replace for area comments", () => {
    openComposer(anchor, { id: "c1", kind: "area", page: 1, rect: [0, 0, 1, 1] }, { label: "x" });
    expect(document.querySelector('[data-mode="replace"]')?.classList.contains("hidden")).toBe(true);
  });
});
