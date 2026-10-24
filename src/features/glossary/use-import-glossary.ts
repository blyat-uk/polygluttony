import { useQueryClient } from "@tanstack/react-query";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import { ipc } from "@/lib/ipc";
import { useAppStore } from "@/stores/app-store";
import { glossaryKey } from "./keys";
import { projectKey } from "@/features/project/use-project";

/** The single undo slot (glossary.prev.json) as the confirm dialogs describe it.
 *  Every op that REPLACES the backup must invalidate this key. */
export function glossaryBackupKey(folder: string) {
  return ["glossary-backup", folder] as const;
}

/** Pick a glossary.json and install it into `folder`.
 *  Shared by CreateView ("Start from an existing glossary…") and EditorView
 *  ("Import glossary…"). Any existing glossary is backed up to
 *  glossary.prev.json by the backend, so this doubles as re-generate undo. */
export function useImportGlossary(folder: string) {
  const qc = useQueryClient();
  return async () => {
    const picked = await openDialog({
      multiple: false,
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (!picked || Array.isArray(picked)) return;
    try {
      const count = await ipc.importGlossary(folder, picked);
      // The user may have switched folders while the dialog was open — the
      // import still belongs to `folder`, but don't touch the active view's
      // rail badge with another folder's count.
      await qc.invalidateQueries({ queryKey: glossaryKey(folder) });
      await qc.invalidateQueries({ queryKey: projectKey(folder) });
      // The import just overwrote glossary.prev.json with the replaced
      // glossary — the "replacing the existing backup" disclosure is now stale.
      await qc.invalidateQueries({ queryKey: glossaryBackupKey(folder) });
      if (useAppStore.getState().workdir === folder) {
        useAppStore.getState().setGlossaryTerms(count);
      }
      toast.success(`Imported ${count} term${count !== 1 ? "s" : ""}`);
    } catch (e: unknown) {
      toast.error(String(e));
    }
  };
}
