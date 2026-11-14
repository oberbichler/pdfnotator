/** Language picker in the title bar. The choice is remembered per user. */
import {
  applyTranslations,
  detectLocale,
  getLocale,
  isLocale,
  LOCALES,
  type Locale,
  onLocaleChange,
  setLocale,
  t,
} from "../i18n";
import { load, save } from "../lib/storage";
import { byId, h } from "./dom";
import { icon } from "./icons";

const STORAGE_KEY = "locale";

function systemLocale(): Locale {
  return detectLocale(navigator.languages?.length ? navigator.languages : [navigator.language]);
}

/** Apply the stored or system language. */
export function initLocale(): void {
  const stored = load(STORAGE_KEY);
  setLocale(stored && isLocale(stored) ? stored : systemLocale());
  applyTranslations();
  onLocaleChange(() => applyTranslations());
}

function choose(locale: Locale | null): void {
  save(STORAGE_KEY, locale);
  setLocale(locale ?? systemLocale());
}

function renderMenu(menu: HTMLElement): void {
  const stored = load(STORAGE_KEY);
  menu.replaceChildren(h("div", "menu-title", t("language.title")));
  const item = (label: string, code: Locale | null, active: boolean) => {
    const b = h("button", `menu-item lang${active ? " active" : ""}`);
    b.setAttribute("role", "menuitemradio");
    b.setAttribute("aria-checked", String(active));
    if (code) b.lang = code;
    b.append(h("span", "lang-name", label));
    if (active) b.append(icon("check", 14));
    b.addEventListener("click", () => {
      choose(code);
      menu.classList.add("hidden");
    });
    return b;
  };
  menu.append(item(t("language.system"), null, !stored));
  const list = h("div", "lang-list");
  const current = getLocale();
  const sorted = (Object.keys(LOCALES) as Locale[]).sort((a, b) =>
    LOCALES[a].name.localeCompare(LOCALES[b].name, current),
  );
  for (const code of sorted) list.append(item(LOCALES[code].name, code, stored === code));
  menu.append(list);
}

export function initLanguagePicker(): void {
  const button = byId("btn-language");
  const menu = byId("language-menu");
  button.addEventListener("click", (e) => {
    e.stopPropagation();
    if (!menu.classList.contains("hidden")) return menu.classList.add("hidden");
    renderMenu(menu);
    menu.classList.remove("hidden");
    const r = button.getBoundingClientRect();
    menu.style.top = `${r.bottom + 6}px`;
    menu.style.left = `${Math.max(8, Math.min(r.right - menu.offsetWidth, window.innerWidth - menu.offsetWidth - 8))}px`;
  });
  document.addEventListener("mousedown", (e) => {
    if (!menu.contains(e.target as Node) && !button.contains(e.target as Node)) menu.classList.add("hidden");
  });
  document.addEventListener("keydown", (e) => e.key === "Escape" && menu.classList.add("hidden"));
}
