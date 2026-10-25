// "Replace in translations…": plain find and replace over translations, no AI.

import { cloneMeta, markYours, putTerm } from "./edits";
import { recount, refOfKey, type GlossaryState, type TermRow } from "./model";

export interface ReplaceOptions {
  find: string;
  replace: string;
  matchCase: boolean;
}

export interface ReplaceMatch {
  key: string;
  category: string;
  source: string;
  before: string;
  after: string;
  /** Occurrences replaced in this translation. */
  count: number;
  /** The result would be empty, so it can't be applied. */
  empty: boolean;
}

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** Replace every occurrence. A function replacer keeps `$&` etc. literal. */
export function replaceAllIn(
  text: string,
  { find, replace, matchCase }: ReplaceOptions,
): { text: string; count: number } {
  if (!find) return { text, count: 0 };
  let count = 0;
  const out = text.replace(new RegExp(escapeRegExp(find), matchCase ? "g" : "gi"), () => {
    count++;
    return replace;
  });
  return { text: out, count };
}

/** Every row whose translation would change, in row order. */
export function findReplaceMatches(rows: TermRow[], opts: ReplaceOptions): ReplaceMatch[] {
  if (!opts.find) return [];
  const out: ReplaceMatch[] = [];
  for (const row of rows) {
    const { text, count } = replaceAllIn(row.target, opts);
    const after = text.trim();
    if (count === 0 || after === row.target) continue;
    out.push({
      key: row.key,
      category: row.category,
      source: row.source,
      before: row.target,
      after,
      count,
      empty: after === "",
    });
  }
  return out;
}

/** Apply the replacement to the ticked rows, recomputed against the current
 *  translations. Changed terms become yours. Returns how many changed. */
export function applyReplace(
  s: GlossaryState,
  keys: Iterable<string>,
  opts: ReplaceOptions,
): { state: GlossaryState; changed: number } {
  const terms = { ...s.doc.terms };
  const meta = cloneMeta(s.meta);
  let changed = 0;
  for (const key of keys) {
    const { category, source } = refOfKey(key);
    const before = terms[category]?.[source];
    if (before === undefined) continue;
    const { text } = replaceAllIn(before, opts);
    const after = text.trim();
    if (!after || after === before) continue;
    putTerm(terms, category, source, after);
    markYours(meta, category, source);
    changed++;
  }
  if (!changed) return { state: s, changed };
  return { state: { doc: { ...s.doc, terms, count: recount(terms) }, meta }, changed };
}
