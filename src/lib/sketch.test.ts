import { describe, expect, it } from "vitest";
import {
  describeShapes,
  extendShape,
  isAccidental,
  isClosed,
  type Point,
  type Shape,
  shapePath,
  sketchBounds,
  TEXT_PT,
  textOrigin,
  textShape,
  touchedWords,
} from "./sketch";

// A4 in points
const A4 = { w: 595.28, h: 841.89 };

const circle = (cx: number, cy: number, r: number, n = 24): Point[] =>
  Array.from({ length: n + 1 }, (_, i) => {
    const a = (i / n) * 2 * Math.PI;
    return [cx + (r * Math.cos(a)) / A4.w, cy + (r * Math.sin(a)) / A4.h];
  });

describe("extendShape", () => {
  it("appends pen points but skips jitter", () => {
    const s: Shape = { type: "pen", points: [[0.5, 0.5]] };
    extendShape(s, [0.5 + 0.1 / A4.w, 0.5], A4);
    extendShape(s, [0.6, 0.5], A4);
    expect(s.points).toHaveLength(2);
  });

  it("moves the end point of arrows and rectangles", () => {
    const s: Shape = { type: "arrow", points: [[0.1, 0.1]] };
    extendShape(s, [0.2, 0.2], A4);
    extendShape(s, [0.3, 0.4], A4);
    expect(s.points).toEqual([
      [0.1, 0.1],
      [0.3, 0.4],
    ]);
  });
});

describe("geometry", () => {
  it("treats a click without movement as accidental", () => {
    expect(isAccidental({ type: "rect", points: [[0.5, 0.5]] }, A4)).toBe(true);
    expect(
      isAccidental(
        {
          type: "rect",
          points: [
            [0.5, 0.5],
            [0.6, 0.5],
          ],
        },
        A4,
      ),
    ).toBe(false);
  });

  it("pads the sketch bounds and clamps them to the page", () => {
    const shapes: Shape[] = [
      {
        type: "rect",
        points: [
          [0, 0],
          [0.5, 0.5],
        ],
      },
    ];
    const [x, y, w, h] = sketchBounds(shapes, A4, 24);
    expect([x, y]).toEqual([0, 0]);
    expect(w).toBeCloseTo(0.5 + 24 / A4.w);
    expect(h).toBeCloseTo(0.5 + 24 / A4.h);
  });

  it("recognises loops", () => {
    expect(isClosed({ type: "pen", points: circle(0.5, 0.5, 30) }, A4)).toBe(true);
    const line: Point[] = Array.from({ length: 10 }, (_, i) => [0.1 + i * 0.05, 0.5]);
    expect(isClosed({ type: "pen", points: line }, A4)).toBe(false);
  });
});

describe("shapePath", () => {
  it("draws rectangles in page points", () => {
    const path = shapePath(
      {
        type: "rect",
        points: [
          [0, 0],
          [0.5, 0.5],
        ],
      },
      { w: 100, h: 200 },
    );
    expect(path).toBe("M0 0H50V100H0Z");
  });

  it("gives arrows a head at the end point", () => {
    const path = shapePath(
      {
        type: "arrow",
        points: [
          [0, 0.5],
          [1, 0.5],
        ],
      },
      { w: 100, h: 100 },
    );
    expect(path.startsWith("M0 50L100 50M")).toBe(true);
    expect(path.match(/L100 50/g)).toHaveLength(2);
  });

  it("smooths pen strokes with curves", () => {
    expect(shapePath({ type: "pen", points: circle(0.5, 0.5, 30) }, A4)).toContain("Q");
  });
});

describe("describeShapes", () => {
  it("names the text at both arrow ends and gives positions in mm", () => {
    const textIn = ([x]: readonly number[]) => ((x ?? 0) < 0.5 ? "Figure 3" : "Section 2.1");
    const [arrow] = describeShapes(
      [
        {
          type: "arrow",
          points: [
            [0.1, 0.1],
            [0.9, 0.5],
          ],
        },
      ],
      A4,
      textIn,
    );
    expect(arrow).toMatchObject({
      type: "arrow",
      fromText: "Figure 3",
      toText: "Section 2.1",
      fromMm: [21, 30],
      toMm: [189, 149],
    });
  });

  it("measures rectangles and marks loops", () => {
    const [rect, loop] = describeShapes(
      [
        {
          type: "rect",
          points: [
            [0.1, 0.1],
            [0.1 + 170.08 / A4.w, 0.1 + 113.39 / A4.h],
          ],
        },
        { type: "pen", points: circle(0.5, 0.5, 30) },
      ],
      A4,
      () => "  Q3\n revenue ",
    );
    expect(rect?.boxMm.slice(2)).toEqual([60, 40]);
    expect(rect?.closed).toBeUndefined();
    expect(loop).toMatchObject({ type: "pen", closed: true, text: "Q3 revenue" });
  });
});

describe("touchedWords", () => {
  it("takes every word a shape touches, whole", () => {
    const str = "dolore magnam aliquam quaerat";
    const hit = [...str].map((_, i) => i >= 9 && i <= 15); // "gnam al"
    expect(touchedWords(str, hit)).toBe("magnam aliquam");
    expect(touchedWords(str, [])).toBe("");
  });
});

describe("text notes", () => {
  it("sits centred on the click point, as wide as measured", () => {
    const note = textShape([0.2, 0.5], "bigger", 40, A4);
    const [x, y, w, h] = sketchBounds([note], A4, 0);
    expect(x).toBeCloseTo(0.2);
    expect(w * A4.w).toBeCloseTo(40);
    expect(y + h / 2).toBeCloseTo(0.5);
    expect(h * A4.h).toBeGreaterThan(TEXT_PT);
    const [, by] = textOrigin(note, A4);
    expect(by).toBeGreaterThan(y * A4.h);
    expect(by).toBeLessThan((y + h) * A4.h);
  });

  it("is passed on with its label; empty notes are dropped", () => {
    const note = textShape([0.2, 0.5], "bigger", 40, A4);
    expect(shapePath(note, A4)).toBe("");
    expect(describeShapes([note], A4, () => "ignored")[0]).toMatchObject({ type: "text", label: "bigger" });
    expect(isAccidental(note, A4)).toBe(false);
    expect(isAccidental({ ...note, label: " " }, A4)).toBe(true);
  });
});
