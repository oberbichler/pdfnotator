/**
 * Minimal, typed i18n: `t(key, params)` with `{placeholder}` interpolation and
 * plural support via `Intl.PluralRules`. Static DOM text is bound with
 * `data-i18n`, `data-i18n-title`, `data-i18n-placeholder` and
 * `data-i18n-aria-label` attributes and refreshed by {@link applyTranslations}.
 */
import bg from "./locales/bg";
import cs from "./locales/cs";
import da from "./locales/da";
import de from "./locales/de";
import el from "./locales/el";
import en, { type MessageKey, type Messages, type Plural } from "./locales/en";
import es from "./locales/es";
import et from "./locales/et";
import fi from "./locales/fi";
import fr from "./locales/fr";
import ga from "./locales/ga";
import hr from "./locales/hr";
import hu from "./locales/hu";
import it from "./locales/it";
import lt from "./locales/lt";
import lv from "./locales/lv";
import mt from "./locales/mt";
import nl from "./locales/nl";
import pl from "./locales/pl";
import pt from "./locales/pt";
import ro from "./locales/ro";
import sk from "./locales/sk";
import sl from "./locales/sl";
import sv from "./locales/sv";

export type { MessageKey, Messages, Plural };

/** All official EU languages, with their native names. */
export const LOCALES = {
  bg: { name: "Български", messages: bg },
  cs: { name: "Čeština", messages: cs },
  da: { name: "Dansk", messages: da },
  de: { name: "Deutsch", messages: de },
  el: { name: "Ελληνικά", messages: el },
  en: { name: "English", messages: en },
  es: { name: "Español", messages: es },
  et: { name: "Eesti", messages: et },
  fi: { name: "Suomi", messages: fi },
  fr: { name: "Français", messages: fr },
  ga: { name: "Gaeilge", messages: ga },
  hr: { name: "Hrvatski", messages: hr },
  hu: { name: "Magyar", messages: hu },
  it: { name: "Italiano", messages: it },
  lt: { name: "Lietuvių", messages: lt },
  lv: { name: "Latviešu", messages: lv },
  mt: { name: "Malti", messages: mt },
  nl: { name: "Nederlands", messages: nl },
  pl: { name: "Polski", messages: pl },
  pt: { name: "Português", messages: pt },
  ro: { name: "Română", messages: ro },
  sk: { name: "Slovenčina", messages: sk },
  sl: { name: "Slovenščina", messages: sl },
  sv: { name: "Svenska", messages: sv },
} as const satisfies Record<string, { name: string; messages: Messages }>;

export type Locale = keyof typeof LOCALES;
export type Params = Record<string, string | number>;

const FALLBACK: Locale = "en";

let current: Locale = FALLBACK;
let pluralRules = new Intl.PluralRules(FALLBACK);
const listeners = new Set<() => void>();

export function isLocale(code: string): code is Locale {
  return Object.hasOwn(LOCALES, code);
}

/** Pick the best supported locale from a list of BCP 47 tags (e.g. `navigator.languages`). */
export function detectLocale(preferred: readonly string[]): Locale {
  for (const tag of preferred) {
    const base = tag.toLowerCase().split(/[-_]/)[0];
    if (base && isLocale(base)) return base;
  }
  return FALLBACK;
}

export function getLocale(): Locale {
  return current;
}

export function setLocale(locale: Locale): void {
  current = locale;
  pluralRules = new Intl.PluralRules(locale);
  if (typeof document !== "undefined") document.documentElement.lang = locale;
  for (const fn of listeners) fn();
}

/** Run `fn` whenever the locale changes. Returns an unsubscribe function. */
export function onLocaleChange(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export function interpolate(template: string, params: Params = {}): string {
  return template.replace(/\{(\w+)\}/g, (match, name: string) =>
    Object.hasOwn(params, name) ? String(params[name]) : match,
  );
}

/** Choose the plural form for `n` (falls back to `other`). */
export function selectPlural(message: Plural, n: number, rules: Intl.PluralRules = pluralRules): string {
  return message[rules.select(n)] ?? message.other;
}

/** Translate `key`. Plural messages use `params.n` as count. */
export function t(key: MessageKey, params: Params = {}): string {
  const message: string | Plural = LOCALES[current].messages[key] ?? en[key];
  const template = typeof message === "string" ? message : selectPlural(message, Number(params.n ?? 0));
  return interpolate(template, params);
}

const ATTRIBUTES = [
  ["data-i18n-title", "title"],
  ["data-i18n-placeholder", "placeholder"],
  ["data-i18n-aria-label", "aria-label"],
] as const;

/** Translate all `data-i18n*` bound elements below `root`. */
export function applyTranslations(root: ParentNode = document): void {
  for (const el of root.querySelectorAll<HTMLElement>("[data-i18n]")) {
    el.textContent = t(el.dataset.i18n as MessageKey);
  }
  for (const [data, attr] of ATTRIBUTES) {
    for (const el of root.querySelectorAll<HTMLElement>(`[${data}]`)) {
      const key = el.getAttribute(data) as MessageKey;
      const shortcut = el.dataset.shortcut ? ` (${el.dataset.shortcut})` : "";
      el.setAttribute(attr, t(key) + shortcut);
    }
  }
}
