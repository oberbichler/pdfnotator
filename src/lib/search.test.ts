import { describe, expect, it } from "vitest";
import { contextAround, findAll, fold, searchPages } from "./search";

const slice = (text: string, [s, e]: readonly [number, number]) => text.slice(s, e);

describe("search", () => {
  it("folds case, diacritics and whitespace", () => {
    const f = fold("  Résumé\n\n  Straße ");
    expect(f.text).toBe("resume straße ");
    expect(f.map).toHaveLength(f.text.length);
  });

  it("finds matches regardless of case and accents", () => {
    const text = "Das Café und das CAFE.";
    expect(findAll(text, "cafe").map((m) => slice(text, m))).toEqual(["Café", "CAFE"]);
  });

  it("matches across line breaks and repeated spaces", () => {
    const text = "Cloud-\nMigration und  Cloud Migration";
    expect(findAll(text, "cloud migration").map((m) => slice(text, m))).toEqual(["Cloud Migration"]);
    const wrapped = "Cloud\nMigration";
    expect(findAll(wrapped, "cloud migration").map((m) => slice(wrapped, m))).toEqual(["Cloud\nMigration"]);
  });

  it("returns nothing for empty queries and does not overlap", () => {
    expect(findAll("abc", "   ")).toEqual([]);
    expect(findAll("aaaa", "aa")).toEqual([
      [0, 2],
      [2, 4],
    ]);
  });

  it("maps matches containing decomposed characters back correctly", () => {
    const text = "été"; // "été" with combining accents
    expect(findAll(text, "ete")).toEqual([[0, 5]]);
  });

  it("searches pages in order and numbers matches per page", () => {
    const hits = searchPages(["alpha beta", "beta beta", "gamma"], "beta");
    expect(hits.map((h) => [h.page, h.index])).toEqual([
      [0, 0],
      [1, 0],
      [1, 1],
    ]);
  });

  it("cuts context at word boundaries around a quote", () => {
    const text = "alpha beta gamma delta epsilon zeta eta theta";
    const at = text.indexOf("delta");
    expect(contextAround(text, at, 5, 10)).toBe("…gamma delta epsilon…");
    expect(contextAround(text, at, 5, 500)).toBe(text);
    expect(contextAround("a  \n b", 0, 1)).toBe("a b");
  });
});
