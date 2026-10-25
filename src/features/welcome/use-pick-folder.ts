import { useCallback } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import type { ProjectView } from "@/types/generated/ProjectView";
import { useOpenFolder } from "@/features/project/use-project";
import { folderTitle } from "@/lib/folder-title";

/** A folder with no subtitles stays where it is; say so. */
function sayEmpty(path: string) {
  toast.warning("No subtitle files here", {
    description: `${folderTitle(path)} has no .ass files. Pick the folder that holds them.`,
  });
}

/**
 * Open a folder from anywhere: `pick` shows the folder dialog (Open folder, Ctrl+O),
 * `openPath` opens a known path (a recent folder, a drop). On success `useOpenFolder` moves
 * to the folder's episodes; a folder without subtitles stays put and `onEmpty` runs (a toast
 * unless the caller shows it inline).
 */
export function usePickFolder({ onEmpty = sayEmpty }: { onEmpty?: (path: string) => void } = {}) {
  const open = useOpenFolder();
  const { mutate } = open;
  const openPath = useCallback(
    (path: string) =>
      mutate(path, {
        onSuccess: (view: ProjectView) => {
          if (view.files.length === 0) onEmpty(path);
        },
      }),
    [mutate, onEmpty],
  );
  const pick = useCallback(async () => {
    const path = await openDialog({ directory: true, multiple: false });
    if (typeof path === "string") openPath(path);
  }, [openPath]);
  return { pick, openPath, pending: open.isPending, data: open.data };
}
