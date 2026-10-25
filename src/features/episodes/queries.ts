import { useQuery } from "@tanstack/react-query";
import { ipc } from "@/lib/ipc";

export const folderResultsKey = (folder: string) => ["folder-results", folder] as const;

/** Each episode's stored last result. Refetched when a run finishes. */
export function useFolderResults(folder: string) {
  return useQuery({
    queryKey: folderResultsKey(folder),
    queryFn: () => ipc.getFolderResults(folder),
    staleTime: Infinity,
  });
}

/** The glossary with its metadata, for "Add N episodes to the glossary"
 *  (which ticked files it hasn't scanned). Its own key: the Glossary page keeps
 *  its own query. Refetched whenever Episodes mounts. */
export const glossaryLoadKey = (folder: string) => ["episodes", "glossary-load", folder] as const;

export function useGlossaryLoad(folder: string, enabled: boolean) {
  return useQuery({
    queryKey: glossaryLoadKey(folder),
    queryFn: () => ipc.loadGlossary(folder),
    enabled,
    staleTime: 0,
  });
}
