// Pure edits on a glossary + its metadata. Every function returns a new state
// and never mutates its input. Every user edit, add or move marks the term
// `yours` and clears its NEW tag.

import type { GlossaryDoc } from "@/types/generated/GlossaryDoc";
import type { GlossaryMeta } from "@/types/generated/GlossaryMeta";
import type { TermOrigin } from "@/types/generated/TermOrigin";
import type { TermRef } from "@/types/generated/TermRef";
import {
  CATEGORY_LABEL,
  categoryOf,
  isCategory,
  recount,
  type Category,
  type GlossaryState,
} from "./model";

export type EditResult = { ok: true; state: GlossaryState } | { ok: false; error: string };

type Terms = GlossaryDoc["terms"];

function cloneTerms(terms: Terms): Terms {
  const out: Terms = {};
  for (const [c, t] of Object.entries(terms)) out[c] = { ...t };
  return out;
}

export function cloneMeta(meta: GlossaryMeta): GlossaryMeta {
  const origins: GlossaryMeta["origins"] = {};
  for (const [c, o] of Object.entries(meta.origins)) origins[c] = { ...o };
  const unreviewed: GlossaryMeta["unreviewed"] = {};
  for (const [c, u] of Object.entries(meta.unreviewed)) unreviewed[c] = [...u];
  return { ...meta, origins, unreviewed };
}

function withTerms(doc: GlossaryDoc, terms: Terms): GlossaryDoc {
  return { ...doc, terms, count: recount(terms) };
}

// ── in-place helpers on already-cloned terms / meta ─────────────────────────

export function putTerm(terms: Terms, category: string, source: string, target: string) {
  terms[category] = { ...(terms[category] ?? {}), [source]: target };
}

export function dropTerm(terms: Terms, category: string, source: string) {
  if (!terms[category]) return;
  const rest = { ...terms[category] };
  delete rest[source];
  terms[category] = rest;
}

export function setOrigin(meta: GlossaryMeta, category: string, source: string, origin: TermOrigin) {
  meta.origins[category] = { ...(meta.origins[category] ?? {}), [source]: origin };
}

export function clearNew(meta: GlossaryMeta, category: string, source: string) {
  const list = meta.unreviewed[category];
  if (list?.includes(source)) meta.unreviewed[category] = list.filter((s) => s !== source);
}

/** Forget everything the meta knows about a term (delete, or before a move/rename). */
export function dropTermMeta(meta: GlossaryMeta, category: string, source: string) {
  if (meta.origins[category] && source in meta.origins[category]) {
    const rest = { ...meta.origins[category] };
    delete rest[source];
    meta.origins[category] = rest;
  }
  clearNew(meta, category, source);
}

/** The user owns this term now. */
export function markYours(meta: GlossaryMeta, category: string, source: string) {
  setOrigin(meta, category, source, "yours");
  clearNew(meta, category, source);
}

// ── edits ────────────────────────────────────────────────────────────────────

export function addTerm(
  s: GlossaryState,
  category: Category,
  rawSource: string,
  rawTarget: string,
): EditResult {
  const source = rawSource.trim();
  const target = rawTarget.trim();
  if (!source || !target) return { ok: false, error: "Type the term and its translation." };
  // The engine's merge semantics: a source is unique across all categories.
  const holder = categoryOf(s.doc, source);
  if (holder) return { ok: false, error: `${source} is already in ${CATEGORY_LABEL[holder]}.` };
  const terms = cloneTerms(s.doc.terms);
  putTerm(terms, category, source, target);
  const meta = cloneMeta(s.meta);
  markYours(meta, category, source);
  return { ok: true, state: { doc: withTerms(s.doc, terms), meta } };
}

/** Change a term's translation. An unchanged value returns the same state. */
export function editTarget(s: GlossaryState, ref: TermRef, rawTarget: string): EditResult {
  const target = rawTarget.trim();
  const current = s.doc.terms[ref.category]?.[ref.source];
  if (current === undefined) return { ok: false, error: `${ref.source} is no longer in the glossary.` };
  if (!target) return { ok: false, error: "A translation can't be empty. Delete the term instead." };
  if (target === current) return { ok: true, state: s };
  const terms = cloneTerms(s.doc.terms);
  putTerm(terms, ref.category, ref.source, target);
  const meta = cloneMeta(s.meta);
  markYours(meta, ref.category, ref.source);
  return { ok: true, state: { doc: withTerms(s.doc, terms), meta } };
}

/** Rename a term's source (its key). Refuses a source another term already has. */
export function renameSource(s: GlossaryState, ref: TermRef, rawSource: string): EditResult {
  const source = rawSource.trim();
  const target = s.doc.terms[ref.category]?.[ref.source];
  if (target === undefined) return { ok: false, error: `${ref.source} is no longer in the glossary.` };
  if (!source) return { ok: false, error: "A term can't be empty. Delete it instead." };
  if (source === ref.source) return { ok: true, state: s };
  const holder = categoryOf(s.doc, source);
  if (holder) return { ok: false, error: `${source} is already in ${CATEGORY_LABEL[holder]}.` };
  const terms = cloneTerms(s.doc.terms);
  dropTerm(terms, ref.category, ref.source);
  putTerm(terms, ref.category, source, target);
  const meta = cloneMeta(s.meta);
  dropTermMeta(meta, ref.category, ref.source);
  markYours(meta, ref.category, source);
  return { ok: true, state: { doc: withTerms(s.doc, terms), meta } };
}

/** Move terms to another category. Terms already there are left alone. */
export function moveTerms(s: GlossaryState, refs: TermRef[], to: Category): GlossaryState {
  const terms = cloneTerms(s.doc.terms);
  const meta = cloneMeta(s.meta);
  let moved = 0;
  for (const ref of refs) {
    if (ref.category === to) continue;
    const target = terms[ref.category]?.[ref.source];
    if (target === undefined) continue;
    dropTerm(terms, ref.category, ref.source);
    dropTermMeta(meta, ref.category, ref.source);
    putTerm(terms, to, ref.source, target);
    markYours(meta, to, ref.source);
    moved++;
  }
  return moved ? { doc: withTerms(s.doc, terms), meta } : s;
}

export function deleteTerms(s: GlossaryState, refs: TermRef[]): GlossaryState {
  const terms = cloneTerms(s.doc.terms);
  const meta = cloneMeta(s.meta);
  for (const ref of refs) {
    dropTerm(terms, ref.category, ref.source);
    dropTermMeta(meta, ref.category, ref.source);
  }
  return { doc: withTerms(s.doc, terms), meta };
}

/** Clear the NEW tag. Leaves the origin alone: reviewing isn't editing. */
export function markReviewed(s: GlossaryState, refs: TermRef[]): GlossaryState {
  const meta = cloneMeta(s.meta);
  for (const ref of refs) clearNew(meta, ref.category, ref.source);
  return { doc: s.doc, meta };
}

export function setShowName(s: GlossaryState, name: string): GlossaryState {
  const show_name = name.trim() || null;
  if (show_name === s.meta.show_name) return s;
  return { doc: s.doc, meta: { ...s.meta, show_name } };
}

/** Keep only refs that still exist (e.g. a selection after an undo). */
export function existingRefs(s: GlossaryState, refs: TermRef[]): TermRef[] {
  return refs.filter(
    (r) => isCategory(r.category) && s.doc.terms[r.category]?.[r.source] !== undefined,
  );
}
