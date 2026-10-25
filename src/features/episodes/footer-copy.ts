import type { GlossaryLoad } from "@/types/generated/GlossaryLoad";
import type { SourceFile } from "@/types/generated/SourceFile";

/** Why Translate can't start right now; null when it can. */
export type Blocked =
  | "other_run"
  | "glossary_busy"
  | "no_model"
  | "same_language"
  | "nothing_ticked"
  | null;

/** The first reason that holds, in the order a user would fix them. */
export function blockedReason(s: {
  /** A translation run in another folder. */
  otherRun: boolean;
  glossaryBusy: boolean;
  hasModel: boolean;
  sameLanguage: boolean;
  ticked: number;
}): Blocked {
  if (s.otherRun) return "other_run";
  if (s.glossaryBusy) return "glossary_busy";
  if (!s.hasModel) return "no_model";
  if (s.sameLanguage) return "same_language";
  if (s.ticked === 0) return "nothing_ticked";
  return null;
}

/** Lines-per-minute heuristic for the run estimate (same as the old Translate page). */
export const LINES_PER_MINUTE = 600;

/** "Translate 1 episode", "Translate 3 episodes · 2 replace existing translations".
 *  `replacing` = ticked files that already have a translation on disk. */
export function translateLabel(count: number, replacing: number): string {
  if (count === 0) return "Translate";
  const base = `Translate ${count} ${count === 1 ? "episode" : "episodes"}`;
  if (replacing === 0) return base;
  return replacing === 1
    ? `${base} · 1 replaces an existing translation`
    : `${base} · ${replacing} replace existing translations`;
}

/** How many of the ticked files already have a translation on disk. */
export function replacingCount(selected: string[], files: SourceFile[]): number {
  const ticked = new Set(selected);
  return files.filter((f) => ticked.has(f.name) && f.translation !== "none").length;
}

/** "~1 min", "~1 h 20 min". */
export function formatMinutes(minutes: number): string {
  if (minutes < 60) return `~${minutes} min`;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return m === 0 ? `~${h} h` : `~${h} h ${m} min`;
}

/** The model's own name from the Translate label: "Claude Sonnet 5.5 +2" → "Claude Sonnet 5.5". */
export const modelName = (label: string) => label.replace(/\s\+\d+$/, "");

/** Rough speed of a glossary build, which reads in big batches. */
export const GLOSSARY_LINES_PER_MINUTE = 1800;

/** "~1 min on Claude Sonnet 5.5 · uses glossary (232)"; null when nothing is ticked
 *  or there's no model to name. `glossaryLines`: lines a glossary step reads first. */
export function estimateLine(
  lines: number,
  translationLabel: string | null,
  glossaryTerms: number | null,
  glossaryLines = 0,
): string | null {
  if (lines === 0 || !translationLabel) return null;
  const minutes = Math.max(
    1,
    Math.ceil(lines / LINES_PER_MINUTE + glossaryLines / GLOSSARY_LINES_PER_MINUTE),
  );
  let text = `${formatMinutes(minutes)} on ${modelName(translationLabel)}`;
  if (glossaryTerms != null) text += ` · uses glossary (${glossaryTerms.toLocaleString()})`;
  return text;
}

/** The glossary has terms from episodes it has no record of: it was made
 *  before PG kept track, or imported. */
export function untracked(glossary: Extract<GlossaryLoad, { status: "ok" }>): boolean {
  return glossary.meta.untracked ?? glossary.meta.scanned_files.length === 0;
}

/**
 * Whether the glossary has read an episode. For an untracked glossary an
 * episode that has a translation counts as read too, so the ones offered are
 * the new ones.
 */
export function glossaryHasRead(
  glossary: GlossaryLoad | undefined,
  files: SourceFile[],
): (name: string) => boolean {
  if (glossary?.status !== "ok") return () => false;
  const read = new Set(glossary.meta.scanned_files);
  if (untracked(glossary)) {
    for (const f of files) if (f.translation !== "none") read.add(f.name);
  }
  return (name) => read.has(name);
}

/** The ticked episodes a glossary step would read: every one when there's no
 *  glossary yet, else the ones it hasn't read. Empty while the glossary is
 *  loading or can't be read. */
export function glossaryStepFiles(
  selected: string[],
  files: SourceFile[],
  glossary: GlossaryLoad | undefined,
): string[] {
  if (glossary?.status === "missing") return selected;
  if (glossary?.status !== "ok") return [];
  const read = glossaryHasRead(glossary, files);
  return selected.filter((n) => !read(n));
}

/** "Build glossary, then translate 3 episodes", "Update glossary, then translate
 *  1 episode · 1 replaces an existing translation". */
export function combinedLabel(kind: "first" | "scan", count: number, replacing: number): string {
  const verb = kind === "first" ? "Build" : "Update";
  return `${verb} glossary, then t${translateLabel(count, replacing).slice(1)}`;
}

/** The tick next to the button: "Build a glossary first", "Add 1 episode to the glossary first". */
export function glossaryTickLabel(kind: "first" | "scan", n: number): string {
  return kind === "first"
    ? "Build a glossary first"
    : `Add ${n} ${n === 1 ? "episode" : "episodes"} to the glossary first`;
}
