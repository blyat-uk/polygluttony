import type { EpisodeLine } from "@/types/generated/EpisodeLine";
import type { EpisodeSnapshot } from "@/types/generated/EpisodeSnapshot";
import type { GlossaryDoc } from "@/types/generated/GlossaryDoc";
import type { GlossaryMeta } from "@/types/generated/GlossaryMeta";
import type { VerifyIssue } from "@/types/generated/VerifyIssue";

// Pure helpers for review in the episode window: no React, no IPC.

// ── flagged lines ────────────────────────────────────────────────────────────

/** What a flagged line needs: its open issues, and whether it has no
 *  translation at all. */
export type LineFlag = { issues: VerifyIssue[]; untranslated: boolean };

/** Lines that need a look, by id: named by an issue, or left untranslated
 *  (the backend lists only the first few of those as issues). File-level
 *  issues (`line_id` 0) are left out. */
export function flagsByLine(
  lines: readonly EpisodeLine[],
  issues: readonly VerifyIssue[],
): Map<number, LineFlag> {
  const out = new Map<number, LineFlag>();
  const known = new Set(lines.map((l) => l.id));
  for (const it of issues) {
    if (it.line_id <= 0 || !known.has(it.line_id)) continue;
    const flag = out.get(it.line_id) ?? { issues: [], untranslated: false };
    flag.issues.push(it);
    out.set(it.line_id, flag);
  }
  for (const line of lines) {
    if (line.status !== "untranslated") continue;
    const flag = out.get(line.id) ?? { issues: [], untranslated: false };
    flag.untranslated = true;
    out.set(line.id, flag);
  }
  return out;
}

/** Flagged ids, ascending. */
export function flaggedIds(flags: ReadonlyMap<number, LineFlag>): number[] {
  return [...flags.keys()].sort((a, b) => a - b);
}

/** 1-based position of `selected` among the flagged ids, or null when the
 *  selected line isn't flagged. */
export function flaggedPosition(flagged: readonly number[], selected: number | null): number | null {
  if (selected === null) return null;
  const i = flagged.indexOf(selected);
  return i < 0 ? null : i + 1;
}

/** The flagged line after (or before) `selected`, wrapping around. With no
 *  selection, the first (or last). */
export function stepFlagged(
  flagged: readonly number[],
  selected: number | null,
  dir: 1 | -1,
): number | null {
  if (flagged.length === 0) return null;
  if (selected === null) return dir === 1 ? flagged[0] : flagged[flagged.length - 1];
  if (dir === 1) return flagged.find((id) => id > selected) ?? flagged[0];
  for (let i = flagged.length - 1; i >= 0; i--) if (flagged[i] < selected) return flagged[i];
  return flagged[flagged.length - 1];
}

// ── issues ───────────────────────────────────────────────────────────────────

export type IssueTone = "name" | "meaning" | "untranslated";

/** The tag a user sees. The engine's old "drift" issues read as meaning:
 *  users never see that word. */
export function issueTag(issueType: string): { label: string; tone: IssueTone } {
  switch (issueType) {
    case "name":
      return { label: "NAME", tone: "name" };
    case "untranslated":
      return { label: "UNTRANSLATED", tone: "untranslated" };
    default:
      return { label: "MEANING", tone: "meaning" };
  }
}

/** "Looks fine" applies to these: an untranslated line has nothing to look at. */
export function dismissable(flag: LineFlag): VerifyIssue[] {
  return flag.issues.filter((i) => i.issue_type !== "untranslated");
}

/** The glossary term a line's name issue is about, if any. */
export function issueTerm(flag: LineFlag): string | null {
  return flag.issues.find((i) => i.issue_type === "name" && i.term)?.term ?? null;
}

// ── what the window may do ───────────────────────────────────────────────────

/** Why review actions are off right now, or null when they're on. */
export function reviewBlocked(
  snapshot: EpisodeSnapshot,
  kind: "live" | "stored",
  hasFolder: boolean,
): string | null {
  if (!hasFolder) return "Open this episode from the Episodes list to review it.";
  if (kind === "live" && snapshot.run_active) {
    const finished =
      snapshot.state === "done" || snapshot.state === "warning" || snapshot.state === "failed";
    return finished
      ? "The run is still going. Review this episode when it ends."
      : "This episode is being translated. Review it when it's done.";
  }
  if (snapshot.detail && kind === "stored") return `${snapshot.detail} Translate it again to review it.`;
  if (snapshot.state === "stopped") return "This episode isn't finished. Resume it from the Episodes list.";
  if (snapshot.state === "failed") return "This episode didn't translate. Try it again from the Episodes list.";
  if (snapshot.state !== "done" && snapshot.state !== "warning") return "There's nothing to review yet.";
  return null;
}

/** Errors from the backend, in the window's words where they're known. */
export function plainError(err: unknown): string {
  const text = err instanceof Error ? err.message : String(err);
  if (text === "another operation is already running") {
    return "Another run is going. Try again when it ends.";
  }
  return text;
}

// ── glossary ─────────────────────────────────────────────────────────────────

/** Category keys, as the glossary stores them, with their labels. */
export const GLOSSARY_CATEGORIES = [
  { key: "characters", label: "Characters" },
  { key: "cultivation", label: "Cultivation" },
  { key: "skills", label: "Skills" },
  { key: "locations", label: "Locations" },
  { key: "items", label: "Items" },
  { key: "organizations", label: "Organizations" },
] as const;

export type GlossaryCategory = (typeof GLOSSARY_CATEGORIES)[number]["key"];

/** Where a source term already sits in the glossary, and its translation. */
export function findTerm(
  doc: GlossaryDoc,
  source: string,
): { category: GlossaryCategory; target: string } | null {
  for (const { key } of GLOSSARY_CATEGORIES) {
    const target = doc.terms[key]?.[source];
    if (target !== undefined) return { category: key, target };
  }
  return null;
}

/** The glossary with `source → target` in `category` (moved there if it sat
 *  elsewhere), marked as the user's own and no longer new. */
export function withTerm(
  doc: GlossaryDoc,
  meta: GlossaryMeta,
  category: GlossaryCategory,
  source: string,
  target: string,
): { doc: GlossaryDoc; meta: GlossaryMeta } {
  const terms: GlossaryDoc["terms"] = {};
  const origins: GlossaryMeta["origins"] = {};
  const unreviewed: GlossaryMeta["unreviewed"] = {};
  for (const [cat, map] of Object.entries(doc.terms)) {
    const next = { ...map };
    delete next[source];
    terms[cat] = next;
  }
  for (const [cat, map] of Object.entries(meta.origins)) {
    const next = { ...map };
    delete next[source];
    origins[cat] = next;
  }
  for (const [cat, list] of Object.entries(meta.unreviewed)) {
    unreviewed[cat] = list.filter((s) => s !== source);
  }
  terms[category] = { ...(terms[category] ?? {}), [source]: target };
  origins[category] = { ...(origins[category] ?? {}), [source]: "yours" };
  const count = Object.values(terms).reduce((n, map) => n + Object.keys(map).length, 0);
  return {
    doc: { ...doc, terms, count },
    meta: { ...meta, origins, unreviewed },
  };
}

/** "14 lines in 3 episodes", "1 line in this episode". */
export function termReach(lines: number, episodes: number, onlyThis: boolean): string {
  const l = `${lines.toLocaleString()} ${lines === 1 ? "line" : "lines"}`;
  if (onlyThis) return `${l} in this episode`;
  return `${l} in ${episodes} ${episodes === 1 ? "episode" : "episodes"}`;
}
