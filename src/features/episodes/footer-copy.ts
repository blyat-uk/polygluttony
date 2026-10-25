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

/** "~1 min on Claude Sonnet 5.5 · uses glossary (232)"; null when nothing is ticked
 *  or there's no model to name. */
export function estimateLine(
  lines: number,
  translationLabel: string | null,
  glossaryTerms: number | null,
): string | null {
  if (lines === 0 || !translationLabel) return null;
  const minutes = Math.max(1, Math.ceil(lines / LINES_PER_MINUTE));
  let text = `${formatMinutes(minutes)} on ${modelName(translationLabel)}`;
  if (glossaryTerms != null) text += ` · uses glossary (${glossaryTerms.toLocaleString()})`;
  return text;
}

/** Ticked files the glossary hasn't scanned yet — the footer offers to add
 *  them. Empty when the folder has no (readable) glossary. */
export function unscannedFiles(selected: string[], glossary: GlossaryLoad | undefined): string[] {
  if (glossary?.status !== "ok") return [];
  const scanned = new Set(glossary.meta.scanned_files);
  return selected.filter((n) => !scanned.has(n));
}

/** "Add 1 episode to the glossary", "Add 3 episodes to the glossary". */
export const addToGlossaryLabel = (n: number) =>
  `Add ${n} ${n === 1 ? "episode" : "episodes"} to the glossary`;
