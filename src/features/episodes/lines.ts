import type { ProjectView } from "@/types/generated/ProjectView";
import type { SourceFile } from "@/types/generated/SourceFile";

/**
 * The lines a run sends for this episode: its lines to translate in every
 * style that isn't kept as it is. A sign repeated frame by frame counts once;
 * song effects, drawings and lines with no words never count.
 */
export function linesToTranslate(file: SourceFile, kept: readonly string[]): number {
  let n = 0;
  for (const [style, lines] of Object.entries(file.style_lines)) {
    if (!kept.includes(style)) n += lines ?? 0;
  }
  return n;
}

/** Lines to translate across the folder, with its kept styles. */
export function folderLinesToTranslate(view: ProjectView): number {
  return view.files.reduce((n, f) => n + linesToTranslate(f, view.prefs.kept_styles), 0);
}
