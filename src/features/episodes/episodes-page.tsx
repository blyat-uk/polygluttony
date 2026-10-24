import { useCallback, useEffect, useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useShallow } from "zustand/react/shallow";
import { toast } from "sonner";
import type { FolderPrefs } from "@/types/generated/FolderPrefs";
import type { ProjectView } from "@/types/generated/ProjectView";
import { ipc } from "@/lib/ipc";
import { useAppStore } from "@/stores/app-store";
import { useGlossaryRun } from "@/stores/glossary-store";
import { useTranslationRun } from "@/stores/translation-store";
import { usePrimaryAction } from "@/hooks/use-primary-action";
import { projectKey, syncProjectStore, useProject } from "@/features/project/use-project";
import { runProgress } from "@/features/translate/telemetry";
import { FolderHeader } from "./folder-header";
import { FolderSettings } from "./folder-settings";
import { EpisodeTable } from "./episode-table";
import { EpisodesFooter } from "./episodes-footer";
import { RunBanner, type BannerState } from "./run-banner";
import { resolveStatus, type EpisodeStatus } from "./episode-status";
import {
  PRESETS,
  existingSelection,
  presetSelection,
  selectionLine,
  type SelectPreset,
} from "./selection";
import {
  addToGlossaryLabel,
  blockedReason,
  estimateLine,
  replacingCount,
  translateLabel,
  unscannedFiles,
} from "./footer-copy";
import { tallyRun } from "./run-summary";
import { folderResultsKey, glossaryLoadKey, useFolderResults, useGlossaryLoad } from "./queries";

export function EpisodesPage() {
  const workdir = useAppStore((s) => s.workdir);
  if (!workdir) return null;
  // Keyed by folder: the settings' open/closed state and the fold-outs start over.
  return <Episodes key={workdir} folder={workdir} />;
}

/** Ticks once a second while `active` (the slow-down countdown). */
function useNow(active: boolean) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [active]);
  return now;
}

function Episodes({ folder }: { folder: string }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const { data: view, isFetching } = useProject(folder);
  const { data: languages } = useQuery({
    queryKey: ["languages"],
    queryFn: ipc.listLanguages,
    staleTime: Infinity,
  });
  const { data: stored } = useFolderResults(folder);
  const hasGlossary = !!view?.supports_glossary && view.glossary_terms != null;
  const { data: glossary } = useGlossaryLoad(folder, hasGlossary);

  const translationLabel = useAppStore((s) => s.translationLabel);
  const hasUsableConnection = useAppStore((s) => s.hasUsableConnection);
  const glossaryBusy = useGlossaryRun((s) => s.busy !== null);
  const run = useTranslationRun(
    useShallow((s) => ({
      running: s.running,
      folder: s.folder,
      files: s.files,
      logs: s.logs,
      results: s.results,
      stopInfo: s.stopInfo,
      stopRequested: s.stopRequested,
      throttle: s.throttle,
      startError: s.startError,
      lastStart: s.lastStart,
      external: s.external,
    })),
  );
  const [logsOpen, setLogsOpen] = useState(false);

  const ours = run.folder === folder;
  const runningHere = ours && run.running;
  const throttleActive = runningHere && run.throttle !== null && run.throttle.until > Date.now();
  const now = useNow(throttleActive);

  const refresh = useCallback(() => {
    void qc.invalidateQueries({ queryKey: projectKey(folder) });
    void qc.invalidateQueries({ queryKey: folderResultsKey(folder) });
    void qc.invalidateQueries({ queryKey: glossaryLoadKey(folder) });
  }, [qc, folder]);

  // The disk may have changed while the user was in another app (a new
  // episode dropped in, a file fixed by hand): re-read on focus.
  useEffect(() => {
    const unlisten = getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (focused) refresh();
    });
    return () => {
      void unlisten.then((f) => f());
    };
  }, [refresh]);

  const statusOf = useMemo(() => {
    const map = new Map<string, EpisodeStatus>();
    for (const file of view?.files ?? []) {
      map.set(
        file.name,
        resolveStatus({
          file,
          live: ours ? run.files[file.name] : undefined,
          running: runningHere,
          external: run.external,
          record: stored?.episodes[file.name],
          throttle: run.throttle,
          now,
        }),
      );
    }
    return (name: string) => map.get(name)!;
  }, [view, ours, runningHere, run.external, run.files, run.throttle, stored, now]);

  const selected = view ? existingSelection(view.prefs.selected_files, view.files) : [];
  const sameLanguage = !!view && view.prefs.source_lang === view.prefs.target_lang;
  // While this folder's run goes, the footer shows Stop instead of Translate.
  const gate = {
    otherRun: run.running && !runningHere,
    glossaryBusy,
    hasModel: hasUsableConnection,
    sameLanguage,
  };
  const blocked = blockedReason({ ...gate, ticked: selected.length });
  const canStart = !run.running && blocked === null;
  const canTranslateOne = !run.running && blockedReason({ ...gate, ticked: 1 }) === null;

  // The ticks live in the cached view for the session; every open seeds them afresh.
  const setSelection = (names: string[]) =>
    qc.setQueryData<ProjectView>(projectKey(folder), (v) =>
      v ? { ...v, prefs: { ...v.prefs, selected_files: names } } : v,
    );

  const persist = async (next: FolderPrefs) => {
    if (!view) return;
    const prev = view.prefs;
    const nextView = { ...view, prefs: next };
    qc.setQueryData(projectKey(folder), nextView);
    syncProjectStore(nextView);
    try {
      await ipc.saveFolderPrefs(folder, next);
    } catch (err) {
      toast.error("Couldn't save the folder settings", { description: String(err) });
      return;
    }
    const langsChanged =
      next.source_lang !== prev.source_lang || next.target_lang !== prev.target_lang;
    if (langsChanged && next.source_lang !== next.target_lang) {
      // The pair is also the default for new folders.
      void ipc.setDefaultLanguages(next.source_lang, next.target_lang);
    }
    // Which files are sources and which outputs exist depend on the pair.
    if (langsChanged) refresh();
  };

  // Every start resumes: the engine carries on only episodes whose stored
  // result is a stop with the same source (and settings); the rest start
  // fresh. So a stopped episode's kept batches are never thrown away.
  const startRun = (files: string[]) => {
    if (!view || files.length === 0) return;
    const { prefs } = view;
    const resume = true;
    const store = useTranslationRun.getState();
    store.start(files, folder, resume);
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
  };

  const stop = () => {
    useTranslationRun.getState().requestStop();
    void ipc.cancelTranslation();
  };

  usePrimaryAction(() => startRun(selected), canStart && !!view);

  if (!view) return null;

  // ── banner ────────────────────────────────────────────────────────────────
  let banner: BannerState = { kind: "none" };
  if (ours && run.running) {
    banner = {
      kind: "running",
      progress: runProgress(run.files, new Map(view.files.map((f) => [f.name, f.dialogue_count]))),
      stopping: run.stopRequested,
    };
  } else if (ours && run.startError) {
    banner = {
      kind: "start_error",
      message: run.startError,
      noModel: !hasUsableConnection,
      retry: run.lastStart,
    };
  } else if (ours && run.results && !run.external) {
    const tally = tallyRun(run.files, run.results);
    banner = run.stopInfo ? { kind: "stopped", info: run.stopInfo, tally } : { kind: "finished", tally };
  }

  // ── footer ────────────────────────────────────────────────────────────────
  const tickedSet = new Set(selected);
  const lines = view.files
    .filter((f) => tickedSet.has(f.name))
    .reduce((n, f) => n + f.dialogue_count, 0);
  const unscanned = hasGlossary ? unscannedFiles(selected, glossary) : [];
  const addToGlossary =
    unscanned.length > 0 && !glossaryBusy
      ? {
          label: addToGlossaryLabel(unscanned.length),
          onClick: () => {
            useGlossaryRun.getState().requestAddEpisodes(folder, unscanned);
            void navigate({ to: "/glossary" });
          },
        }
      : null;

  const watchable = runningHere ? new Set(Object.keys(run.files)) : new Set<string>();
  const pick = (preset: SelectPreset) =>
    setSelection(presetSelection(preset, view.files, (n) => statusOf(n).kind));

  return (
    // Inline-size containment: long paths and names truncate instead of
    // widening the app's grid column.
    <div className="flex h-full min-w-0 flex-col [contain:inline-size]">
      <FolderHeader folder={folder} rescanning={isFetching} onRescan={refresh} />

      <div className="min-h-0 flex-1 space-y-3.5 overflow-auto p-5">
        <FolderSettings
          view={view}
          languages={languages}
          locked={runningHere}
          onChange={(next) => void persist(next)}
        />

        <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[12.5px]">
          <span className="font-medium text-foreground tabular-nums">
            {selectionLine(selected, view.files)}
          </span>
          <span className="text-muted-foreground/50">·</span>
          <span className="text-muted-foreground">Select:</span>
          {PRESETS.map((p) => (
            <button
              key={p.key}
              type="button"
              onClick={() => pick(p.key)}
              className="rounded-md px-1 text-primary underline-offset-2 outline-none hover:underline focus-visible:ring-[3px] focus-visible:ring-ring/50"
            >
              {p.label}
            </button>
          ))}
        </div>

        <RunBanner
          state={banner}
          busy={run.running || glossaryBusy}
          onResume={(files) => startRun(files)}
          onRetry={(files) => startRun(files)}
        />

        <EpisodeTable
          folder={folder}
          files={view.files}
          selected={selected}
          onSelect={setSelection}
          statusOf={statusOf}
          watchable={watchable}
          canTranslate={canTranslateOne}
          onTranslateOne={(name) => startRun([name])}
        />
      </div>

      <EpisodesFooter
        running={runningHere}
        stopping={run.stopRequested}
        label={translateLabel(selected.length, replacingCount(selected, view.files))}
        blocked={blocked}
        estimate={estimateLine(lines, translationLabel, view.glossary_terms)}
        addToGlossary={addToGlossary}
        logs={ours ? run.logs : []}
        logsOpen={logsOpen}
        onToggleLogs={() => setLogsOpen((o) => !o)}
        onStart={() => startRun(selected)}
        onStop={stop}
      />
    </div>
  );
}
