import { afterEach, describe, expect, it } from "vitest";
import { detectLocale, interpolate, selectPlural, setLocale, t } from ".";

describe("runtime", () => {
  afterEach(() => setLocale("en"));

  it("detects the best locale", () => {
    expect(detectLocale(["de-AT", "en-US"])).toBe("de");
    expect(detectLocale(["pt_BR"])).toBe("pt");
    expect(detectLocale(["ja", "ko"])).toBe("en");
    expect(detectLocale([])).toBe("en");
  });

  it("interpolates and keeps unknown placeholders", () => {
    expect(interpolate("{a} and {b}", { a: 1 })).toBe("1 and {b}");
  });

  it("selects plural forms", () => {
    const rules = new Intl.PluralRules("pl");
    const msg = { one: "1 plik", few: "{n} pliki", many: "{n} plików", other: "{n} pliku" };
    expect(selectPlural(msg, 1, rules)).toBe("1 plik");
    expect(selectPlural(msg, 3, rules)).toBe("{n} pliki");
    expect(selectPlural(msg, 5, rules)).toBe("{n} plików");
    expect(selectPlural({ other: "x" }, 1, rules)).toBe("x");
  });

  it("translates with plurals and params", () => {
    expect(t("drafts.count", { n: 1 })).toBe("1 draft");
    expect(t("drafts.count", { n: 4 })).toBe("4 drafts");
    setLocale("de");
    expect(t("drafts.count", { n: 4 })).toBe("4 Entwürfe");
    expect(t("card.page", { page: 3 })).toBe("S. 3");
  });
});
