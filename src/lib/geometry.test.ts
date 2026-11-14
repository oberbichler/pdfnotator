import { describe, expect, it } from "vitest";
import { clamp, isTiny, mergeLineRects, type NRect, normalizeBox, rectFromPoints } from "./geometry";

describe("geometry", () => {
  it("clamps", () => {
    expect(clamp(5, 0, 1)).toBe(1);
    expect(clamp(-1, 0, 1)).toBe(0);
    expect(clamp(0.5, 0, 1)).toBe(0.5);
  });

  it("builds a rect from points in any drag direction", () => {
    expect(rectFromPoints([0.5, 0.6], [0.1, 0.2])).toEqual([0.1, 0.2, 0.4, expect.closeTo(0.4)]);
    expect(rectFromPoints([0.1, 0.2], [0.1, 0.2])).toEqual([0.1, 0.2, 0, 0]);
  });

  it("normalises a box relative to its page", () => {
    const page = { left: 100, top: 50, width: 200, height: 400 };
    expect(normalizeBox({ left: 150, top: 150, width: 50, height: 40 }, page)).toEqual([
      0.25, 0.25, 0.25, 0.1,
    ]);
  });

  it("detects tiny rects in pixels", () => {
    expect(isTiny([0, 0, 0.01, 0.5], { width: 500, height: 500 })).toBe(true);
    expect(isTiny([0, 0, 0.02, 0.02], { width: 500, height: 500 })).toBe(false);
  });

  it("merges glyph runs into line boxes", () => {
    const runs: NRect[] = [
      [0.3, 0.1, 0.1, 0.02], // line 1, second run
      [0.1, 0.1, 0.2, 0.02], // line 1, first run
      [0.1, 0.2, 0.3, 0.02], // line 2
      [0.7, 0.2, 0.1, 0.02], // line 2 but far away → separate
    ];
    const merged = mergeLineRects(runs);
    expect(merged).toHaveLength(3);
    expect(merged[0][0]).toBeCloseTo(0.1);
    expect(merged[0][2]).toBeCloseTo(0.3);
    expect(merged[1]).toEqual([0.1, 0.2, 0.3, 0.02]);
    expect(merged[2]).toEqual([0.7, 0.2, 0.1, 0.02]);
  });

  it("does not mutate its input", () => {
    const runs: NRect[] = [[0.1, 0.1, 0.1, 0.02]];
    const merged = mergeLineRects(runs);
    merged[0][0] = 9;
    expect(runs[0][0]).toBe(0.1);
  });
});
