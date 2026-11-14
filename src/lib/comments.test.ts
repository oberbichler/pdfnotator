import { afterEach, describe, expect, it } from "vitest";
import { setLocale } from "../i18n";
import { type Comment, composeInstruction, displayNumber, normalizeQuote, toPayload } from "./comments";

describe("comments", () => {
  afterEach(() => setLocale("en"));

  it("normalises selected text but keeps line breaks", () => {
    expect(normalizeQuote("  Hello \t  world \n   second  line \n\n")).toBe("Hello world\nsecond line");
  });

  it("composes instructions per mode", () => {
    expect(composeInstruction("comment", "text", "  make it bold ")).toBe("make it bold");
    expect(composeInstruction("delete", "text", "")).toBe("Delete this text.");
    expect(composeInstruction("delete", "area", "too busy")).toBe("Delete this area. too busy");
    expect(composeInstruction("replace", "text", "new", "old")).toBe("Replace with: «new»");
    expect(composeInstruction("replace", "text", "", "old")).toBe("Delete this text.");
    expect(composeInstruction("replace", "text", " old ", "old")).toBeNull();
  });

  it("uses the UI language for generated instructions", () => {
    setLocale("de");
    expect(composeInstruction("delete", "text", "")).toBe("Diesen Text löschen.");
    expect(composeInstruction("replace", "text", "neu", "alt")).toBe("Ersetzen durch: «neu»");
  });

  it("maps comments to the backend wire format", () => {
    const c: Comment = {
      id: "c4",
      kind: "area",
      page: 2,
      comment: "x",
      rect: [0, 0, 1, 1],
      image: "abc",
      status: "draft",
      createdAt: 1,
    };
    expect(toPayload(c)).toEqual({
      id: "c4",
      kind: "area",
      page: 2,
      quote: null,
      comment: "x",
      rect: [0, 0, 1, 1],
      image: "abc",
      preview: null,
      areaText: null,
      context: null,
    });
  });

  it("shows ids as numbers", () => {
    expect(displayNumber("c12")).toBe("12");
  });
});
