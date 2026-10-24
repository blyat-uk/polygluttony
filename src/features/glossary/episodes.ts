// Which episodes a scan starts with ticked.

import type { SourceFile } from "@/types/generated/SourceFile";

export interface AddSelection {
  selected: string[];
  /** An older glossary that doesn't record which episodes built it. */
  unknownHistory: boolean;
}

/**
 * "Add new episodes": a request from the Episodes page wins; otherwise tick the
 * files the glossary hasn't scanned yet. A glossary from before PG kept track
 * (`scanned_files` empty) falls back to the episodes that aren't translated.
 */
export function defaultAddSelection(
  files: SourceFile[],
  scanned: string[],
  preset?: string[] | null,
): AddSelection {
  const unknownHistory = scanned.length === 0;
  if (preset) {
    const wanted = new Set(preset);
    return { selected: files.filter((f) => wanted.has(f.name)).map((f) => f.name), unknownHistory };
  }
  if (unknownHistory) {
    return {
      selected: files.filter((f) => f.translation === "none").map((f) => f.name),
      unknownHistory,
    };
  }
  const done = new Set(scanned);
  return { selected: files.filter((f) => !done.has(f.name)).map((f) => f.name), unknownHistory };
}

/** "1 episode", "3 episodes". */
export function episodes(n: number): string {
  return `${n.toLocaleString()} ${n === 1 ? "episode" : "episodes"}`;
}
