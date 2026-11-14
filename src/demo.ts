/**
 * Demo mode for documentation, using the regular UI code paths:
 *
 * - `PDFNOTATOR_DEMO=1` stages a review session for a screenshot.
 * - `PDFNOTATOR_DEMO=animate` plays a scripted session with a visible pointer
 *   (see `scripts/record-demo.sh`, which also runs a scripted agent).
 *
 * Loaded on demand only; never active in normal use.
 */
import { commitDraft } from "./actions";
import { applyTranslations, setLocale } from "./i18n";
import { activeDoc, addAgentMessage, type DocState, nextCommentId, setStatus, update } from "./store";
import { commentArea, textInRect } from "./ui/area-tool";
import { setTool } from "./ui/dock";
import { autoGrow, byId, h } from "./ui/dom";
import { placeRect } from "./ui/marks";
import { selectionAction } from "./ui/selection";
import { beginShape, commentSketch, endShape, extendCurrent, setShape } from "./ui/sketch-tool";
import type { NRect, PageView, Viewer } from "./viewer";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Wait until `fn` returns something truthy (layout and text layers are async). */
async function until<T>(fn: () => T | undefined | null | false, timeout = 30_000): Promise<T> {
  const end = Date.now() + timeout;
  for (;;) {
    const v = fn();
    if (v) return v;
    if (Date.now() > end) throw new Error("demo: timed out");
    await sleep(50);
  }
}

/** Range over the first occurrence of `phrase` in a page's text layer. */
function findRange(viewer: Viewer, page: number, phrase: string): Range | null {
  const layer = viewer.pages[page]?.textLayer;
  if (!layer) return null;
  const walker = document.createTreeWalker(layer, NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const at = n.textContent?.indexOf(phrase) ?? -1;
    if (at < 0) continue;
    const range = document.createRange();
    range.setStart(n, at);
    range.setEnd(n, at + phrase.length);
    return range;
  }
  return null;
}

function select(range: Range): void {
  const sel = window.getSelection();
  sel?.removeAllRanges();
  sel?.addRange(range);
}

/** Type into the composer and optionally submit it (Enter). */
async function type(text: string, delay = 0, submit = true): Promise<void> {
  const input = byId<HTMLTextAreaElement>("cmp-input");
  if (delay === 0) input.value = text;
  for (let i = 1; delay && i <= text.length; i++) {
    input.value = text.slice(0, i);
    autoGrow(input);
    await sleep(delay);
  }
  if (submit) input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
}

// ---------- static screenshot ----------

async function stage(viewer: Viewer, doc: DocState): Promise<void> {
  // demo settings are not saved: the user's preferences stay untouched
  update((s) => {
    s.live = false;
    s.navOpen = true;
  });
  const quote = await until(() => findRange(viewer, 0, "roughly 30 %"));
  select(quote);
  selectionAction("comment");
  await type("Add the source for this number");

  const page = viewer.pages[0];
  if (page) {
    const rect: NRect = [0.08, 0.53, 0.84, 0.17];
    await commitDraft(
      {
        id: nextCommentId(),
        kind: "area",
        page: 1,
        rect,
        areaText: textInRect(page, rect),
        imagePromise: viewer.captureRegion(0, rect),
      },
      "Turn these cost figures into a small table",
    );
  }
  const ids = doc.comments.map((c) => c.id);
  setStatus(doc.id, ids, "delivered");
  addAgentMessage(doc.id, "Added the cost model as source (footnote 1).", ids.slice(0, 1));

  select(await until(() => findRange(viewer, 0, "maintenance window")));
  selectionAction("comment");
  await type("Mention the current release frequency here", 0, false);
}

// ---------- animation ----------

/** A visible mouse pointer (the real one is not part of window captures). */
class Pointer {
  private el = h("div", "demo-pointer");
  x = window.innerWidth * 0.6;
  y = window.innerHeight * 0.8;

  constructor() {
    this.el.innerHTML =
      '<svg width="22" height="22" viewBox="0 0 24 24"><path d="M4 2l14 11-6.5 1 4 7.5-3 1.5-4-7.5L4 20z" ' +
      'fill="#fff" stroke="#111" stroke-width="1.6" stroke-linejoin="round"/></svg>';
    document.body.append(this.el);
    this.place(0);
  }

  private place(ms: number): void {
    this.el.style.transition = `transform ${ms}ms cubic-bezier(.3,.7,.3,1)`;
    this.el.style.transform = `translate(${this.x}px, ${this.y}px)`;
  }

  async move(x: number, y: number, ms = 600): Promise<void> {
    this.x = x;
    this.y = y;
    this.place(ms);
    await sleep(ms);
  }

  async click(): Promise<void> {
    this.el.classList.add("down");
    await sleep(140);
    this.el.classList.remove("down");
  }

  press(down: boolean): void {
    this.el.classList.toggle("down", down);
  }
}

/** Drag-select `phrase` word by word, then release (the composer opens). */
async function dragSelect(viewer: Viewer, pointer: Pointer, phrase: string): Promise<void> {
  const full = await until(() => findRange(viewer, 0, phrase));
  const node = full.startContainer;
  const start = full.startOffset;
  const first = full.getClientRects()[0] ?? full.getBoundingClientRect();
  await pointer.move(first.left - 1, first.top + first.height * 0.55, 900);
  pointer.press(true);
  const steps = 12;
  for (let i = 1; i <= steps; i++) {
    const r = document.createRange();
    r.setStart(node, start);
    r.setEnd(node, start + Math.round((phrase.length * i) / steps));
    select(r);
    const end = r.getBoundingClientRect();
    await pointer.move(end.right, end.top + end.height * 0.55, 45);
  }
  pointer.press(false);
  select(full);
  selectionAction("comment");
}

/** Drag a rectangle with the area tool over the paragraphs from `fromText` to `toText`. */
async function dragArea(viewer: Viewer, pointer: Pointer, fromText: string, toText: string): Promise<void> {
  const pv = viewer.pages[0];
  if (!pv) return;
  const a = await until(() => findRange(viewer, 0, fromText));
  const b = await until(() => findRange(viewer, 0, toText));
  const page = pv.el.getBoundingClientRect();
  const top = a.getBoundingClientRect().top - 8;
  const bottom = b.getBoundingClientRect().bottom + 8;
  const left = page.left + page.width * 0.075;
  const right = page.left + page.width * 0.925;

  const tool = document.querySelector<HTMLElement>('#tools [data-tool="area"]');
  if (tool) {
    const r = tool.getBoundingClientRect();
    await pointer.move(r.left + r.width / 2, r.top + r.height / 2, 800);
    await pointer.click();
  }
  setTool("area");
  await pointer.move(left, top, 900);
  pointer.press(true);
  const band = h("div", "band");
  pv.overlay.append(band);
  const toRect = (x: number, y: number): NRect => [
    (left - page.left) / page.width,
    (top - page.top) / page.height,
    (x - left) / page.width,
    (y - top) / page.height,
  ];
  const steps = 18;
  for (let i = 1; i <= steps; i++) {
    const x = left + ((right - left) * i) / steps;
    const y = top + ((bottom - top) * i) / steps;
    placeRect(band, toRect(x, y));
    await pointer.move(x, y, 35);
  }
  pointer.press(false);
  commentArea(viewer, pv, toRect(right, bottom), band);
}

/** Move the visible pointer onto a dock button and click it. */
async function clickButton(pointer: Pointer, selector: string): Promise<void> {
  const el = document.querySelector<HTMLElement>(selector);
  if (!el) return;
  const r = el.getBoundingClientRect();
  await pointer.move(r.left + r.width / 2, r.top + r.height / 2, 700);
  await pointer.click();
}

/** Draw the points (client coordinates) as one shape, moving the pointer along. */
async function drawStroke(
  pv: PageView,
  pointer: Pointer,
  points: [number, number][],
  ms: number,
): Promise<void> {
  const page = pv.el.getBoundingClientRect();
  const norm = ([x, y]: [number, number]): [number, number] => [
    (x - page.left) / page.width,
    (y - page.top) / page.height,
  ];
  const [first, ...rest] = points;
  if (!first) return;
  await pointer.move(first[0], first[1], 800);
  pointer.press(true);
  beginShape(pv, norm(first));
  for (const p of rest) {
    extendCurrent(norm(p));
    await pointer.move(p[0], p[1], ms);
  }
  endShape();
  pointer.press(false);
}

/** Circle the "Risks" section freehand and draw an arrow up to the costs heading. */
async function drawSketch(viewer: Viewer, pointer: Pointer): Promise<void> {
  const pv = viewer.pages[0];
  if (!pv) return;
  const heading = await until(() => findRange(viewer, 0, "Risks"));
  heading.startContainer.parentElement?.scrollIntoView({ block: "center", behavior: "smooth" });
  await sleep(900);
  const head = await until(() => findRange(viewer, 0, "Risks"));
  const end = await until(() => findRange(viewer, 0, "before the switch."));
  const costs = await until(() => findRange(viewer, 0, "Costs and benefits"));
  const page = pv.el.getBoundingClientRect();
  const top = head.getBoundingClientRect().top - 12;
  const bottom = end.getBoundingClientRect().bottom + 12;
  const left = page.left + page.width * 0.07;
  const right = page.left + page.width * 0.93;

  await clickButton(pointer, '#tools [data-tool="draw"]');
  setTool("draw");

  // a slightly wobbly loop, a bit more than once around, like by hand
  const cx = (left + right) / 2;
  const cy = (top + bottom) / 2;
  const rx = (right - left) / 2;
  const ry = (bottom - top) / 2 + 6;
  const loop: [number, number][] = [];
  for (let i = 0; i <= 48; i++) {
    const a = Math.PI * 0.85 + (i / 48) * 2.1 * Math.PI;
    const wobble = 1 + 0.025 * Math.sin(i * 0.9);
    loop.push([cx + rx * wobble * Math.cos(a), cy + ry * wobble * Math.sin(a)]);
  }
  await drawStroke(pv, pointer, loop, 22);
  await sleep(250);

  await clickButton(pointer, '#shapes [data-shape="arrow"]');
  setShape("arrow");
  const target = costs.getBoundingClientRect();
  const from: [number, number] = [left + 4, top - 4];
  const to: [number, number] = [target.left - 14, target.top + target.height / 2];
  const arrow: [number, number][] = [];
  for (let i = 0; i <= 14; i++) {
    arrow.push([from[0] + ((to[0] - from[0]) * i) / 14, from[1] + ((to[1] - from[1]) * i) / 14]);
  }
  await drawStroke(pv, pointer, arrow, 30);
  await sleep(400);
  commentSketch();
}

async function animate(viewer: Viewer, doc: DocState): Promise<void> {
  update((s) => {
    s.live = true;
    s.navOpen = false;
  });
  const pointer = new Pointer();
  const resolved = (n: number) => () => doc.comments.filter((c) => c.status === "resolved").length >= n;

  // the scripted agent connects and starts listening
  await until(() => doc.listeners > 0);
  await sleep(1200);

  await dragSelect(viewer, pointer, "roughly 30 %");
  await sleep(350);
  await type("Add the source for this number", 55);
  await pointer.move(window.innerWidth - 230, window.innerHeight - 190, 900);
  await until(resolved(1), 60_000);
  await sleep(1800);

  await dragArea(viewer, pointer, "Costs and benefits", "larger benefit");
  await sleep(500);
  await type("Turn these cost figures into a small table", 50);
  setTool("text");
  await pointer.move(window.innerWidth - 230, window.innerHeight - 190, 900);
  await until(resolved(2), 60_000);
  await sleep(1800);

  await drawSketch(viewer, pointer);
  await sleep(400);
  await type("Move this section before the costs", 50);
  setShape("pen");
  setTool("text");
  await pointer.move(window.innerWidth - 230, window.innerHeight - 190, 900);
  await until(resolved(3), 60_000);
  await sleep(2500);
}

export async function runDemo(viewer: Viewer, animated: boolean): Promise<void> {
  setLocale("en");
  applyTranslations();
  const doc = await until(() => activeDoc());
  await until(() => viewer.pages[0]?.textLayer);
  await sleep(400);
  await (animated ? animate(viewer, doc) : stage(viewer, doc));
}
