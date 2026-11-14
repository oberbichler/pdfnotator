// @vitest-environment jsdom
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../actions", () => ({ commitDraft: vi.fn(async () => {}) }));

const { initComposer, isComposerOpen, closeComposer } = await import("./composer");
const { initSelection } = await import("./selection");
const { resetStore } = await import("../test-utils");

// jsdom has no layout: give ranges empty geometry
beforeAll(() => {
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () => new DOMRect(10, 10, 50, 12);
});

const MARKUP = `
  <div id="composer" class="hidden">
    <div id="cmp-modes">
      <button data-mode="comment"></button><button data-mode="replace"></button><button data-mode="delete"></button>
    </div>
    <span id="cmp-kind"></span><button id="cmp-close"></button>
    <div id="cmp-ctx"></div>
    <textarea id="cmp-input"></textarea>
    <span id="cmp-action"></span><button id="cmp-send"></button>
  </div>
  <div class="page"><div class="textLayer"><span>Alpha beta gamma delta</span></div><div class="overlay"></div></div>`;

function setup() {
  document.body.innerHTML = MARKUP;
  const el = document.querySelector<HTMLDivElement>(".page");
  const textLayer = document.querySelector<HTMLDivElement>(".textLayer");
  const overlay = document.querySelector<HTMLDivElement>(".overlay");
  if (!el || !textLayer || !overlay) throw new Error("markup");
  const pv = { index: 0, el, textLayer, overlay };
  const viewer = { pageOf: (n: Node | null) => (n && el.contains(n) ? pv : undefined), flushTextSwap() {} };
  initComposer({ afterClose: () => {} });
  initSelection(viewer as never);
  return textLayer;
}

/** Simulate dragging over `word` with the mouse. */
async function select(layer: HTMLElement, word: string) {
  const text = layer.querySelector("span")?.firstChild as Text;
  const start = text.data.indexOf(word);
  layer.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
  const range = document.createRange();
  range.setStart(text, start);
  range.setEnd(text, start + word.length);
  const sel = window.getSelection();
  sel?.removeAllRanges();
  sel?.addRange(range);
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
  await new Promise((r) => setTimeout(r, 0));
}

describe("text selection", () => {
  beforeEach(() => {
    resetStore(1);
    closeComposer();
  });

  it("opens the composer with the input focused right after selecting", async () => {
    const layer = setup();
    await select(layer, "gamma");
    expect(isComposerOpen()).toBe(true);
    expect(document.activeElement?.id).toBe("cmp-input");
    expect(document.querySelector("#cmp-ctx blockquote")?.textContent).toBe("gamma");
  });

  it("does not replace a comment that is still being typed", async () => {
    const layer = setup();
    await select(layer, "gamma");
    const input = document.getElementById("cmp-input") as HTMLTextAreaElement;
    input.value = "half-written comment";
    await select(layer, "delta");
    expect(input.value).toBe("half-written comment");
    expect(document.querySelector("#cmp-ctx blockquote")?.textContent).toBe("gamma");
  });
});
