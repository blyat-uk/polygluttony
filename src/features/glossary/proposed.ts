// Tidy up / Look up names online results as a list of proposed changes the
// user ticks, and applying only the ticked ones to the glossary as it is now.

import type { GlossaryDiff } from "@/types/generated/GlossaryDiff";
import type { CategoryDiff } from "@/types/generated/CategoryDiff";
import type { GlossaryMeta } from "@/types/generated/GlossaryMeta";
import type { TermDiff } from "@/types/generated/TermDiff";
import { cloneMeta, dropTerm, dropTermMeta, markYours, putTerm } from "./edits";
import {
  CATEGORIES,
  CATEGORY_LABEL,
  categoryOf,
  compareSource,
  isCategory,
  originOf,
  recount,
  type Category,
  type GlossaryState,
} from "./model";

export type ChangeKind = "changed" | "removed" | "added" | "moved";

export interface ProposedChange {
  id: string;
  kind: ChangeKind;
  source: string;
  /** Where the term ends up (or, for `removed`, where it was). */
  category: Category;
  /** `moved` only: where it was. */
  from: Category | null;
  old: string | null;
  new: string | null;
  /** The user owns this term: the change starts unticked. */
  yours: boolean;
}

export type ApplyKind = "tidy_up" | "look_up";

/** Turn an engine diff into a flat change list. A source removed from one
 *  category and added to another becomes a single `moved` change, so it can't
 *  be half-applied into two categories at once. */
export function proposedChanges(diff: GlossaryDiff, meta: GlossaryMeta): ProposedChange[] {
  const removed = new Map<string, { category: Category; old: string | null }>();
  const added = new Map<string, { category: Category; new: string | null }>();
  const out: ProposedChange[] = [];

  for (const c of diff.categories) {
    if (!isCategory(c.key)) continue;
    for (const t of c.terms) {
      if (t.status === "removed") removed.set(t.source, { category: c.key, old: t.old });
      else if (t.status === "added") added.set(t.source, { category: c.key, new: t.new });
      else if (t.status === "modified") {
        out.push({
          id: `changed:${c.key}:${t.source}`,
          kind: "changed",
          source: t.source,
          category: c.key,
          from: null,
          old: t.old,
          new: t.new,
          yours: originOf(meta, c.key, t.source) === "yours",
        });
      }
    }
  }

  for (const [source, r] of removed) {
    const a = added.get(source);
    if (a && a.category !== r.category) {
      added.delete(source);
      out.push({
        id: `moved:${r.category}:${a.category}:${source}`,
        kind: "moved",
        source,
        category: a.category,
        from: r.category,
        old: r.old,
        new: a.new,
        yours: originOf(meta, r.category, source) === "yours",
      });
    } else {
      out.push({
        id: `removed:${r.category}:${source}`,
        kind: "removed",
        source,
        category: r.category,
        from: null,
        old: r.old,
        new: null,
        yours: originOf(meta, r.category, source) === "yours",
      });
    }
  }

  for (const [source, a] of added) {
    out.push({
      id: `added:${a.category}:${source}`,
      kind: "added",
      source,
      category: a.category,
      from: null,
      old: null,
      new: a.new,
      yours: false,
    });
  }

  const order = (c: Category) => CATEGORIES.indexOf(c);
  return out.sort((x, y) => order(x.category) - order(y.category) || compareSource(x.source, y.source));
}

/** Changes that start ticked: everything except terms the user owns. */
export function initialTicked(changes: ProposedChange[]): Set<string> {
  return new Set(changes.filter((c) => !c.yours).map((c) => c.id));
}

export function summarize(changes: ProposedChange[]) {
  let changed = 0;
  let removed = 0;
  let added = 0;
  for (const c of changes) {
    if (c.kind === "removed") removed++;
    else if (c.kind === "added") added++;
    else changed++;
  }
  return { changed, removed, added };
}

/** "11 changed · 2 removed · 1 added" (zeros left out). */
export function summaryLine(changes: ProposedChange[]): string {
  const s = summarize(changes);
  const parts = [
    s.changed ? `${s.changed} changed` : null,
    s.removed ? `${s.removed} removed` : null,
    s.added ? `${s.added} added` : null,
  ].filter(Boolean);
  return parts.length ? parts.join(" · ") : "No changes";
}

/** A diff holding just the applied changes, for `meta.last_change`. */
export function diffOfChanges(changes: ProposedChange[]): GlossaryDiff {
  const byCat = new Map<Category, TermDiff[]>();
  const push = (c: Category, t: TermDiff) => byCat.set(c, [...(byCat.get(c) ?? []), t]);
  for (const ch of changes) {
    switch (ch.kind) {
      case "changed":
        push(ch.category, { source: ch.source, old: ch.old, new: ch.new, status: "modified" });
        break;
      case "removed":
        push(ch.category, { source: ch.source, old: ch.old, new: null, status: "removed" });
        break;
      case "added":
        push(ch.category, { source: ch.source, old: null, new: ch.new, status: "added" });
        break;
      case "moved":
        if (ch.from) push(ch.from, { source: ch.source, old: ch.old, new: null, status: "removed" });
        push(ch.category, { source: ch.source, old: null, new: ch.new, status: "added" });
        break;
    }
  }
  const categories: CategoryDiff[] = [];
  let ta = 0;
  let tr = 0;
  let tm = 0;
  for (const key of CATEGORIES) {
    const terms = byCat.get(key);
    if (!terms) continue;
    const added = terms.filter((t) => t.status === "added").length;
    const removed = terms.filter((t) => t.status === "removed").length;
    const modified = terms.filter((t) => t.status === "modified").length;
    ta += added;
    tr += removed;
    tm += modified;
    categories.push({ key, name: CATEGORY_LABEL[key], terms, added, removed, modified, unchanged: 0 });
  }
  return {
    categories,
    total_added: ta,
    total_removed: tr,
    total_modified: tm,
    has_changes: ta + tr + tm > 0,
  };
}

/**
 * Whether a change still applies to the glossary as it is now. `at` is the
 * category that holds the change's source now (null: none does). A change to
 * a term that's gone (deleted, or renamed to another source) is skipped, so
 * applying it can't bring the term back; only `added` creates a term.
 */
export function stillApplies(ch: ProposedChange, at: Category | null): boolean {
  switch (ch.kind) {
    case "changed":
      return at !== null && ch.new !== null;
    case "removed":
      return at !== null;
    case "moved":
      return at !== null && (ch.new ?? ch.old) !== null;
    case "added":
      return (ch.new ?? ch.old) !== null;
  }
}

/**
 * Apply only the ticked changes to the glossary as it is now (the user may
 * have edited it while the run was going). Applied terms become yours and lose
 * their NEW tag; `meta.last_change` records what was applied.
 */
export function applyProposed(
  s: GlossaryState,
  changes: ProposedChange[],
  ticked: Set<string>,
  kind: ApplyKind,
  nowMs: number,
): { state: GlossaryState; applied: number } {
  const terms = { ...s.doc.terms };
  const meta = cloneMeta(s.meta);
  const doc = () => ({ ...s.doc, terms });
  const applied: ProposedChange[] = [];

  for (const ch of changes) {
    if (!ticked.has(ch.id)) continue;
    const at = categoryOf(doc(), ch.source);
    if (!stillApplies(ch, at)) continue;
    switch (ch.kind) {
      case "changed": {
        if (!at || ch.new === null) continue;
        // Update the term where it lives now; it may have been moved meanwhile.
        putTerm(terms, at, ch.source, ch.new);
        markYours(meta, at, ch.source);
        break;
      }
      case "removed":
        if (!at) continue;
        dropTerm(terms, at, ch.source);
        dropTermMeta(meta, at, ch.source);
        break;
      case "added":
      case "moved": {
        const value = ch.new ?? ch.old;
        if (value === null) continue;
        // A source is unique across categories: take it out of wherever it is.
        if (at && at !== ch.category) {
          dropTerm(terms, at, ch.source);
          dropTermMeta(meta, at, ch.source);
        }
        putTerm(terms, ch.category, ch.source, value);
        markYours(meta, ch.category, ch.source);
        break;
      }
    }
    applied.push(ch);
  }

  if (!applied.length) return { state: s, applied: 0 };
  meta.last_change = { kind, at_ms: nowMs, diff: diffOfChanges(applied) };
  return {
    state: { doc: { ...s.doc, terms, count: recount(terms) }, meta },
    applied: applied.length,
  };
}
