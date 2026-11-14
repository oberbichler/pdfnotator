/**
 * Text search that ignores case, diacritics and whitespace differences
 * ("Resume" finds "Résumé", "foo  bar" finds "foo\nbar") while reporting
 * matches as offsets into the original text.
 */

/** A match as `[start, end)` offsets into the original text. */
export type Match = readonly [start: number, end: number];

export interface Folded {
  text: string;
  /** For every character of `text`, its index in the original string. */
  map: number[];
}

const MARKS = /\p{M}/gu;

/** Fold text for matching and keep a map back to the original offsets. */
export function fold(original: string): Folded {
  let text = "";
  const map: number[] = [];
  let lastWasSpace = true; // trims leading whitespace
  for (let i = 0; i < original.length; i++) {
    const ch = original[i] ?? "";
    if (/\s/.test(ch)) {
      if (!lastWasSpace) {
        text += " ";
        map.push(i);
        lastWasSpace = true;
      }
      continue;
    }
    lastWasSpace = false;
    for (const f of ch.normalize("NFD").replace(MARKS, "").toLocaleLowerCase()) {
      text += f;
      map.push(i);
    }
  }
  return { text, map };
}

/** Normalise a query like the searched text (and trim it). */
export const foldQuery = (query: string): string => fold(query).text.trimEnd();

/** All non-overlapping matches of `query` in `text`. */
export function findAll(text: string, query: string): Match[] {
  const q = foldQuery(query);
  if (!q) return [];
  const { text: hay, map } = fold(text);
  const out: Match[] = [];
  let from = 0;
  for (;;) {
    const at = hay.indexOf(q, from);
    if (at < 0) break;
    const start = map[at] ?? 0;
    let end = (map[at + q.length - 1] ?? start) + 1;
    // include combining marks that belong to the last matched letter
    while (end < text.length && /\p{M}/u.test(text[end] ?? "")) end++;
    out.push([start, end]);
    from = at + q.length;
  }
  return out;
}

/** A match located in a multi-page document. */
export interface PageMatch {
  page: number;
  /** Index of the match among the matches on its page. */
  index: number;
  match: Match;
}

/** Search every page; results are ordered by page, then position. */
export function searchPages(pages: readonly string[], query: string): PageMatch[] {
  return pages.flatMap((text, page) => findAll(text, query).map((match, index) => ({ page, index, match })));
}

/**
 * Text around `[offset, offset + length)`, about `radius` characters on each
 * side, cut at word boundaries and with whitespace collapsed. Marked with "…"
 * where it was cut.
 */
export function contextAround(text: string, offset: number, length: number, radius = 240): string {
  let start = Math.max(0, offset - radius);
  let end = Math.min(text.length, offset + length + radius);
  if (start > 0) start = text.indexOf(" ", start) + 1 || start;
  if (end < text.length)
    end = text.lastIndexOf(" ", end) > offset + length ? text.lastIndexOf(" ", end) : end;
  const body = text.slice(start, end).replace(/\s+/g, " ").trim();
  return `${start > 0 ? "…" : ""}${body}${end < text.length ? "…" : ""}`;
}
