// Pure row filtering for the workspace: category chip, Show filter, search.

import type { GlossaryMeta } from "@/types/generated/GlossaryMeta";
import { CATEGORIES, termKey, type Category, type TermRow } from "./model";

export type ShowFilter = "all" | "new" | "changed" | "yours";

export const SHOW_FILTERS: ShowFilter[] = ["all", "new", "changed", "yours"];

export const SHOW_LABEL: Record<ShowFilter, string> = {
  all: "All terms",
  // NEW marks add up across scans until reviewed, so not just the last scan's.
  new: "New, not reviewed yet",
  changed: "Changed by Tidy up or Look up",
  yours: "Edited by you",
};

/** Terms the last Tidy up / Look up added or changed (removed ones are gone). */
export function changedKeys(meta: GlossaryMeta): Set<string> {
  const out = new Set<string>();
  const lc = meta.last_change;
  if (!lc || (lc.kind !== "tidy_up" && lc.kind !== "look_up")) return out;
  for (const c of lc.diff.categories) {
    for (const t of c.terms) {
      if (t.status === "added" || t.status === "modified") out.add(termKey(c.key, t.source));
    }
  }
  return out;
}

export function matchesShow(row: TermRow, show: ShowFilter, changed: Set<string>): boolean {
  switch (show) {
    case "all":
      return true;
    case "new":
      return row.isNew;
    case "changed":
      return changed.has(row.key);
    case "yours":
      return row.origin === "yours";
  }
}

/** Case-insensitive substring match on either side. `q` must be trimmed + lowercased. */
export function matchesSearch(row: TermRow, q: string): boolean {
  if (!q) return true;
  return row.source.toLowerCase().includes(q) || row.target.toLowerCase().includes(q);
}

export interface RowFilter {
  search: string;
  show: ShowFilter;
  category: Category | null;
}

export interface FilterResult {
  visible: TermRow[];
  /** Rows per category chip after Show + search (the chips work as facets). */
  categoryCounts: Record<Category | "all", number>;
  /** Rows per Show option, ignoring search and category. */
  showCounts: Record<ShowFilter, number>;
}

export function filterRows(rows: TermRow[], f: RowFilter, meta: GlossaryMeta): FilterResult {
  const q = f.search.trim().toLowerCase();
  const changed = changedKeys(meta);
  const categoryCounts = { all: 0 } as Record<Category | "all", number>;
  for (const c of CATEGORIES) categoryCounts[c] = 0;
  const showCounts: Record<ShowFilter, number> = { all: rows.length, new: 0, changed: 0, yours: 0 };
  const visible: TermRow[] = [];
  for (const row of rows) {
    if (row.isNew) showCounts.new++;
    if (changed.has(row.key)) showCounts.changed++;
    if (row.origin === "yours") showCounts.yours++;
    if (!matchesShow(row, f.show, changed) || !matchesSearch(row, q)) continue;
    categoryCounts.all++;
    categoryCounts[row.category]++;
    if (f.category === null || row.category === f.category) visible.push(row);
  }
  return { visible, categoryCounts, showCounts };
}
