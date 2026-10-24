import { useMutation, useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";
import type { ProjectView } from "@/types/generated/ProjectView";
import { ipc } from "@/lib/ipc";
import { useAppStore } from "@/stores/app-store";
import { keepSessionSelection } from "@/features/episodes/selection";

export function projectKey(path: string) {
  return ["project", path] as const;
}

/** Apply a ProjectView to global shell state (status bar + rail gating). */
export function syncProjectStore(view: ProjectView) {
  const untranslated = view.files.filter((f) => f.translation === "none").length;
  useAppStore.getState().setProject({
    workdir: view.folder,
    sourceLang: view.prefs.source_lang,
    targetLang: view.prefs.target_lang,
    worldType: view.prefs.world_override ?? view.detected_world,
    tone: view.prefs.tone,
    fileCount: view.files.length,
    dialogueLineCount: view.total_dialogue_lines,
    hasUntranslated: untranslated > 0,
    glossaryTerms: view.glossary_terms,
  });
}

/**
 * Open a folder: discover/analyze, seed the project cache, sync the shell, and
 * navigate to Episodes. Every open ticks the episodes with no translation yet.
 * A folder with zero `.ass` files stays where it is (the caller shows a message
 * from `mutation.data`).
 */
export function useOpenFolder() {
  const qc = useQueryClient();
  const navigate = useNavigate();
  return useMutation({
    mutationFn: ipc.openFolder,
    onSuccess: (view) => {
      qc.invalidateQueries({ queryKey: ["recents"] });
      if (view.files.length === 0) return; // empty: stay, show inline message
      qc.setQueryData(projectKey(view.folder), view);
      syncProjectStore(view);
      void navigate({ to: "/episodes" });
    },
    onError: (e) => toast.error(String(e)),
  });
}

/** Re-read a folder that's open this session (Rescan, window focus, a run
 *  ending): fresh disk state, but the user's ticks stay. */
async function fetchProject(qc: QueryClient, path: string): Promise<ProjectView> {
  const next = await ipc.openFolder(path);
  // Read after the await: ticks made while the folder was being read count.
  const prev = qc.getQueryData<ProjectView>(projectKey(path));
  const view = prev ? keepSessionSelection(prev, next) : next;
  if (useAppStore.getState().workdir === path) syncProjectStore(view);
  return view;
}

/** The current folder's ProjectView (seeded by useOpenFolder; refetched on
 *  Rescan, when the window regains focus and after a run). */
export function useProject(path: string) {
  const qc = useQueryClient();
  return useQuery({
    queryKey: projectKey(path),
    queryFn: () => fetchProject(qc, path),
    enabled: !!path,
    staleTime: Infinity,
    // Never dropped while the app runs: the cached view holds this session's ticks.
    gcTime: Infinity,
  });
}
