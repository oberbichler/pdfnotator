/**
 * Validates every locale file against the English source: same keys, same
 * message shapes, complete plural categories, placeholders and product names.
 * Locale files are discovered with `import.meta.glob`, so this test also works
 * while a translation is being added.
 */
import { describe, expect, it } from "vitest";
import en, { type Messages, type Plural } from "./locales/en";

const files = import.meta.glob<{ default: Messages }>("./locales/*.ts", { eager: true });
const locales = Object.entries(files).map(
  ([path, mod]) => [path.replace(/^.*\/(\w+)\.ts$/, "$1"), mod.default] as const,
);

const placeholders = (s: string) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
const SYMBOLS = ["⌘↵", "⌘", "↵", "⇧", "⇥", "Esc", "…"] as const;
const PRODUCT_NAMES = ["pdfnotator", "PDFnotator", "Typst", "Copilot", "Claude Code", "VS Code", "mcp.json"];

it("covers all 24 official EU languages", () => {
  expect(locales.map(([code]) => code).sort()).toEqual(
    "bg cs da de el en es et fi fr ga hr hu it lt lv mt nl pl pt ro sk sl sv".split(" "),
  );
});

describe.each(locales)("locale %s", (code, mod) => {
  const messages = mod as unknown as Record<string, string | Plural>;
  const reference = en as Record<string, string | Plural>;

  it("has exactly the English keys", () => {
    expect(Object.keys(messages).sort()).toEqual(Object.keys(reference).sort());
  });

  it("keeps the message shape, placeholders and product names", () => {
    const categories = new Intl.PluralRules(code).resolvedOptions().pluralCategories;
    for (const [key, ref] of Object.entries(reference)) {
      const msg = messages[key];
      const refText = typeof ref === "string" ? ref : ref.other;
      if (typeof ref === "string") {
        expect(typeof msg, key).toBe("string");
      } else {
        expect(typeof msg, key).toBe("object");
        for (const cat of categories) expect((msg as Plural)[cat], `${key}.${cat}`).toBeTypeOf("string");
      }
      const forms = typeof msg === "string" ? [msg] : Object.values(msg);
      for (const form of forms) {
        expect(form.trim(), key).not.toBe("");
        expect(placeholders(form), key).toEqual(placeholders(refText));
        for (const name of PRODUCT_NAMES) {
          if (refText.includes(name)) expect(form, `${key} keeps "${name}"`).toContain(name);
        }
      }
      if (typeof msg === "string") {
        for (const sym of SYMBOLS.slice(0, 5)) {
          if (refText.includes(sym)) expect(msg, `${key} keeps ${sym}`).toContain(sym);
        }
      }
    }
  });

  it("keeps the app name", () => {
    expect(messages["app.name"]).toBe("PDFnotator");
  });
});
