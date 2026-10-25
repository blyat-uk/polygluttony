import type { FolderPrefs } from "@/types/generated/FolderPrefs";
import { ipc } from "@/lib/ipc";
import { useTranslationRun } from "@/stores/translation-store";

/**
 * Translate these episodes with the folder's settings. Every start resumes:
 * the engine carries on only episodes whose stored result is a stop with the
 * same source (and settings); the rest start fresh. So a stopped episode's
 * kept batches are never thrown away.
 */
export function startTranslationRun(folder: string, files: string[], prefs: FolderPrefs) {
  if (files.length === 0) return;
  const resume = true;
  useTranslationRun.getState().start(files, folder, resume);
  // Running a folder keeps its settings (and it's no longer a first visit).
  void ipc.saveFolderPrefs(folder, prefs);
  ipc
    .startTranslation({
      folder,
      files,
      tone: prefs.tone,
      sourceLang: prefs.source_lang,
      targetLang: prefs.target_lang,
      resume,
    })
    .catch((err: unknown) => {
      useTranslationRun.getState().failStart(err instanceof Error ? err.message : String(err));
    });
}
