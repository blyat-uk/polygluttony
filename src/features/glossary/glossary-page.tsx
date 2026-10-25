import { useEffect, useMemo } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { BookOpen } from "@phosphor-icons/react";
import { ipc } from "@/lib/ipc";
import { useAppStore } from "@/stores/app-store";
import { useGlossaryRun } from "@/stores/glossary-store";
import { useOpenFolder, useProject, projectKey } from "@/features/project/use-project";
import type { ProjectView } from "@/types/generated/ProjectView";
import { EmptyState } from "@/components/empty-state";
import { Button } from "@/components/ui/button";
import { CreateView } from "./create-view";
import { BuildProgress } from "./build-progress";
import { ImportProgress } from "./import-progress";
import { EditorView } from "./editor-view";
import { ReferenceReview } from "./reference-review";
import { UnreadableView } from "./unreadable-view";
import { syncLoaded, useGlossaryDoc } from "./doc-store";
import { glossaryKey, recentlySavedLocally } from "./keys";
import { useGlossaryUi } from "./ui-store";
import type { GlossaryState } from "./model";

// Kept here for anything that imported them from the page.
export { glossaryKey, markLocalSave } from "./keys";

export function GlossaryPage() {
  const workdir = useAppStore((s) => s.workdir);
  if (!workdir) return <NoFolder />;
  return <FolderGlossary folder={workdir} />;
}

function NoFolder() {
  const open = useOpenFolder();
  const pick = async () => {
    const path = await openDialog({ directory: true, multiple: false });
    if (typeof path === "string") open.mutate(path);
  };
  return (
    <EmptyState
      icon={<BookOpen weight="duotone" className="size-9 text-primary" />}
      title="Glossary"
      description="Open a folder of subtitles first. Its glossary lives next to the episodes."
      action={
        <Button className="mt-2" onClick={() => void pick()} disabled={open.isPending}>
          Open folder
        </Button>
      }
    />
  );
}

function FolderGlossary({ folder }: { folder: string }) {
  const qc = useQueryClient();
  const busy = useGlossaryRun((s) => s.busy);
  const fileTick = useGlossaryRun((s) => s.fileTick);
  const reviewOpen = useGlossaryRun((s) => s.reviewOpen);
  const { data: view } = useProject(folder);
  // Always reloaded when the page mounts: an episode window, a text editor or
  // Restore may have changed the file while the page was away, and an edit
  // made on a stale copy would save over that change.
  const { data: load, isPending } = useQuery({
    queryKey: glossaryKey(folder),
    queryFn: () => ipc.loadGlossary(folder),
    staleTime: 0,
    refetchOnMount: "always",
  });
  const state = useMemo<GlossaryState | null>(
    () => (load?.status === "ok" ? { doc: load.doc, meta: load.meta } : null),
    [load],
  );
  // An emptied glossary stays in the workspace while there's something to undo.
  const hasHistory = useGlossaryDoc(
    (s) => s.folder === folder && (s.undo.length > 0 || s.redo.length > 0),
  );

  // Files copied into the folder since it was read show up here (Add new
  // episodes) without a trip to Episodes → Rescan.
  useEffect(() => {
    void qc.invalidateQueries({ queryKey: projectKey(folder) });
  }, [qc, folder]);

  // Workspace state (filters, open panel) belongs to one folder.
  useEffect(() => {
    useGlossaryUi.getState().forFolder(folder);
  }, [folder]);

  // Run state is global; a run (or its results) belongs to the folder it was
  // started in. Reset only when the state belongs to a DIFFERENT folder:
  // never on plain remounts, and never mid-run.
  useEffect(() => {
    const s = useGlossaryRun.getState();
    if (s.folder !== null && s.folder !== folder && !s.busy) s.reset();
  }, [folder]);

  // O15: watch glossary.json for external edits while this page is mounted.
  useEffect(() => {
    void ipc.watchGlossary(folder);
    return () => {
      void ipc.unwatchGlossary();
    };
  }, [folder]);

  // Build completion / external edits → refetch. Our own saves rename the file
  // too; skip those (I3), the cache already holds what we wrote.
  useEffect(() => {
    if (fileTick === 0 || recentlySavedLocally()) return;
    void qc.invalidateQueries({ queryKey: glossaryKey(folder) });
  }, [fileTick, folder, qc]);

  // Tell our own writes from changes on disk (they start a new undo history).
  useEffect(() => {
    if (state) syncLoaded(folder, state);
  }, [state, folder]);

  // Whenever the glossary (re)loads, sync the term count into the rail badge
  // and the cached ProjectView (staleTime: Infinity), which would otherwise
  // feed a stale count back to the shell.
  const count = load === undefined ? undefined : load.status === "ok" ? load.doc.count : null;
  useEffect(() => {
    if (count === undefined) return;
    useAppStore.getState().setGlossaryTerms(count);
    qc.setQueryData<ProjectView>(projectKey(folder), (v) => (v ? { ...v, glossary_terms: count } : v));
  }, [count, folder, qc]);

  if (view && !view.supports_glossary) {
    return (
      <EmptyState
        title="Glossary"
        description="A glossary can only be built for Chinese subtitles for now."
      />
    );
  }
  if (busy === "build") return <BuildProgress />;
  if (busy === "import") return <ImportProgress />;
  if (!view || isPending || !load) return null;
  if (reviewOpen) return <ReferenceReview view={view} />;
  if (load.status === "unreadable") {
    return <UnreadableView folder={folder} message={load.message} line={load.line} />;
  }
  if (!state || (state.doc.count === 0 && !hasHistory)) return <CreateView view={view} />;
  return <EditorView view={view} state={state} />;
}
