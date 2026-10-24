import type { FileRow } from "@/stores/translation-store"
import type { FileStateKind } from "@/types/generated/FileStateKind"
import { rowMissing } from "@/features/episodes/episode-status"

const SUCCEEDED: FileStateKind[] = ["done", "warning"]
const FINISHED: FileStateKind[] = ["done", "warning", "failed"]

export interface RunProgress {
  /** Lines translated across the run's files. */
  done: number
  /** Lines across the run's files. */
  total: number
  /** Files that reached done, warning or failed. */
  finished: number
  files: number
}

/** Line-level progress of the run behind the bottom bar. A file counts its
 *  dialogue lines until the pipeline reports its own total, so the bar has a
 *  denominator from the start. Done follows each file's peak, not
 *  `translated`, so a full retranslation doesn't pull the bar back. A finished
 *  file counts the lines it translated: ones it left untranslated aren't done. */
export function runProgress(
  files: Record<string, FileRow>,
  dialogueCounts: Map<string, number>,
): RunProgress {
  let done = 0, total = 0, finished = 0
  const names = Object.keys(files)
  for (const name of names) {
    const f = files[name]
    const fileTotal = f.total > 0 ? f.total : (dialogueCounts.get(name) ?? 0)
    total += fileTotal
    done += SUCCEEDED.includes(f.state) ? Math.max(0, fileTotal - rowMissing(f)) : f.peakTranslated
    if (FINISHED.includes(f.state)) finished++
  }
  return { done, total, finished, files: names.length }
}
