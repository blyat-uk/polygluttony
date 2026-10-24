import type { FileResult } from "@/types/generated/FileResult";
import type { ProjectView } from "@/types/generated/ProjectView";
import type { SourceFile } from "@/types/generated/SourceFile";
import type { StatusKind } from "./episode-status";

export type SelectPreset = "all" | "new" | "needs_look" | "none";

export const PRESETS: { key: SelectPreset; label: string }[] = [
  { key: "all", label: "All" },
  { key: "new", label: "New" },
  { key: "needs_look", label: "Needs a look" },
  { key: "none", label: "None" },
];

/** The files a quick-select button ticks. New = no translation on disk (what
 *  every open ticks); Needs a look = the status column says so. */
export function presetSelection(
  preset: SelectPreset,
  files: SourceFile[],
  statusOf: (name: string) => StatusKind,
): string[] {
  switch (preset) {
    case "all":
      return files.map((f) => f.name);
    case "new":
      return files.filter((f) => f.translation === "none").map((f) => f.name);
    case "needs_look":
      return files.filter((f) => statusOf(f.name) === "needs_look").map((f) => f.name);
    case "none":
      return [];
  }
}

/** Ticked names that still exist, in list order — what a run is sent. */
export function existingSelection(selected: string[], files: SourceFile[]): string[] {
  const ticked = new Set(selected);
  return files.filter((f) => ticked.has(f.name)).map((f) => f.name);
}

/** Tick or untick one file, keeping list order. */
export function toggleOne(selected: string[], name: string, files: SourceFile[]): string[] {
  const next = new Set(selected);
  if (next.has(name)) next.delete(name);
  else next.add(name);
  return existingSelection([...next], files);
}

/**
 * A refetch of a folder that's already open this session (Rescan, focus, a
 * run ending) keeps the user's ticks instead of the open's fresh seed: ticked
 * files that still exist stay ticked, and an untranslated file that wasn't in
 * the list before (a new episode dropped in) gets ticked too.
 */
export function keepSessionSelection(prev: ProjectView, next: ProjectView): ProjectView {
  const before = new Set(prev.files.map((f) => f.name));
  const arrived = next.files
    .filter((f) => !before.has(f.name) && f.translation === "none")
    .map((f) => f.name);
  const selected_files = existingSelection([...prev.prefs.selected_files, ...arrived], next.files);
  return { ...next, prefs: { ...next.prefs, selected_files } };
}

/** After a run, untick the files it translated: they're done, and leaving them
 *  ticked would set up the next run to overwrite them. Failed and unfinished
 *  files stay ticked for another go. */
export function untickTranslated(selected: string[], results: FileResult[]): string[] {
  const translated = new Set(results.filter((r) => r.success).map((r) => r.file));
  return selected.filter((n) => !translated.has(n));
}

/** "1 new episode selected", "3 episodes selected", "No episodes selected". */
export function selectionLine(selected: string[], files: SourceFile[]): string {
  const n = selected.length;
  if (n === 0) return "No episodes selected";
  const ticked = new Set(selected);
  const allNew = files.every((f) => !ticked.has(f.name) || f.translation === "none");
  const noun = n === 1 ? "episode" : "episodes";
  return `${n} ${allNew ? "new " : ""}${noun} selected`;
}
