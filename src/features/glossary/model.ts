import type { GlossaryDoc } from "@/types/generated/GlossaryDoc";
import type { GlossaryMeta } from "@/types/generated/GlossaryMeta";
import type { TermOrigin } from "@/types/generated/TermOrigin";
import type { TermRef } from "@/types/generated/TermRef";

// Category keys, in display order. Keep in sync with `CATEGORIES` in
// src-tauri/src/glossary/model.rs.
export const CATEGORIES = [
  "characters",
  "cultivation",
  "skills",
  "locations",
  "items",
  "organizations",
] as const;
export type Category = (typeof CATEGORIES)[number];

export const CATEGORY_LABEL: Record<Category, string> = {
  characters: "Characters",
  cultivation: "Cultivation",
  skills: "Skills",
  locations: "Locations",
  items: "Items",
  organizations: "Organizations",
};

/** What each category holds; shown in the category chip tooltips. */
export const CATEGORY_HINT: Record<Category, string> = {
  characters: "People: names, titles and forms of address.",
  cultivation: "Realms, stages and energy systems (Xianxia, Wuxia).",
  skills: "Named techniques, moves and abilities.",
  locations: "Places: sects, cities, realms and landmarks.",
  items: "Objects: weapons, pills, artifacts and treasures.",
  organizations: "Groups: clans, sects, factions and dynasties.",
};

export function isCategory(s: string): s is Category {
  return (CATEGORIES as readonly string[]).includes(s);
}

/** The glossary plus everything the app knows about it: one undo step. */
export interface GlossaryState {
  doc: GlossaryDoc;
  meta: GlossaryMeta;
}

/** One table row. */
export interface TermRow {
  key: string;
  category: Category;
  source: string;
  target: string;
  origin: TermOrigin;
  /** Added by a scan and not edited or reviewed since (the NEW tag). */
  isNew: boolean;
}

/** Stable row key: a source is unique across categories, but keying on both
 *  keeps a moved term from reusing its old row's state. */
export function termKey(category: string, source: string): string {
  return `${category}\u0000${source}`;
}

export function refOfKey(key: string): TermRef {
  const i = key.indexOf("\u0000");
  return { category: key.slice(0, i), source: key.slice(i + 1) };
}

export function emptyMeta(): GlossaryMeta {
  return {
    origins: {},
    unreviewed: {},
    last_change: null,
    scanned_files: [],
    show_name: null,
    untracked: null,
  };
}

/** Code-point order, close to the engine's BTreeMap order. */
export function compareSource(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

export function recount(terms: GlossaryDoc["terms"]): number {
  return Object.values(terms).reduce((n, t) => n + Object.keys(t ?? {}).length, 0);
}

/** The category that holds `source`, or null. */
export function categoryOf(doc: GlossaryDoc, source: string): Category | null {
  for (const c of CATEGORIES) {
    if (Object.prototype.hasOwnProperty.call(doc.terms[c] ?? {}, source)) return c;
  }
  return null;
}

export function originOf(meta: GlossaryMeta, category: string, source: string): TermOrigin {
  return meta.origins[category]?.[source] ?? "ai";
}

export function isUnreviewed(meta: GlossaryMeta, category: string, source: string): boolean {
  return meta.unreviewed[category]?.includes(source) ?? false;
}

/** Every term as a row, by category then source. */
export function buildRows(state: GlossaryState): TermRow[] {
  const { doc, meta } = state;
  const rows: TermRow[] = [];
  for (const category of CATEGORIES) {
    const terms = doc.terms[category] ?? {};
    const unreviewed = new Set(meta.unreviewed[category] ?? []);
    const origins = meta.origins[category] ?? {};
    for (const source of Object.keys(terms).sort(compareSource)) {
      rows.push({
        key: termKey(category, source),
        category,
        source,
        target: terms[source],
        origin: origins[source] ?? "ai",
        isNew: unreviewed.has(source),
      });
    }
  }
  return rows;
}

/** "xianxia" → "Xianxia" (section 8: World type values in Title Case). */
export function titleCase(s: string): string {
  return s ? s[0].toUpperCase() + s.slice(1) : s;
}

/** Content equality for two docs, ignoring key order (the engine writes sorted
 *  maps; the UI appends). Tells our own writes from external ones. */
export function sameDoc(a: GlossaryDoc, b: GlossaryDoc): boolean {
  if (a === b) return true;
  if (a.count !== b.count || a.world_type !== b.world_type) return false;
  const cats = new Set([...Object.keys(a.terms), ...Object.keys(b.terms)]);
  for (const c of cats) {
    const ta = a.terms[c] ?? {};
    const tb = b.terms[c] ?? {};
    const ka = Object.keys(ta);
    if (ka.length !== Object.keys(tb).length) return false;
    for (const k of ka) if (ta[k] !== tb[k]) return false;
  }
  return true;
}
