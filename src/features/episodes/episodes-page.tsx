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
import { useGlossaryUi } from "@/features/glossary/ui-store";
import { FolderHeader } from "./folder-header";
import { FolderSettings } from "./folder-settings";
import { GlossaryRow } from "./glossary-row";
import { EpisodeTable } from "./episode-table";
import { EpisodesFooter } from "./episodes-footer";
import { RunBanner, type BannerState, type TwoSteps } from "./run-banner";
import { glossaryStepStatus, resolveStatus, type EpisodeStatus } from "./episode-status";
import {
  PRESETS,
  existingSelection,
  presetSelection,
  selectionLine,
  type SelectPreset,
} from "./selection";
import {
  blockedReason,
  combinedLabel,
  estimateLine,
  glossaryStepFiles,
  glossaryTickLabel,
  replacingCount,
  translateLabel,
} from "./footer-copy";
import { tallyRun } from "./run-summary";
import { folderResultsKey, glossaryLoadKey, useFolderResults, useGlossaryLoad } from "./queries";
import {
  forgetGlossaryStep,
  startCombinedRun,
  stopCombinedRun,
  useCombinedRun,
  type CombinedPlan,
} from "./combined-run";
import { startTranslationRun } from "./start-run";

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

/** The glossary step's phases after it has read every episode. */
const PAST_READING = new Set(["normalizing", "personalizing", "saving"]);

const twoSteps = (plan: CombinedPlan): TwoSteps => ({
  first: plan.kind === "first",
  translating: plan.files.length,
});

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
  // Read whenever the folder can have one: a glossary.json that can't be read
  // counts no terms, and the page says so instead of "none yet".
  const { data: glossary } = useGlossaryLoad(folder, !!view?.supports_glossary);
  const glossaryBroken = !!view?.supports_glossary && glossary?.status === "unreadable";

  const translationLabel = useAppStore((s) => s.translationLabel);
  const hasUsableConnection = useAppStore((s) => s.hasUsableConnection);
  const glossaryBusy = useGlossaryRun((s) => s.busy !== null);
  const step = useGlossaryRun(
    useShallow((s) => ({
      phase: s.phase,
      done: s.done,
      total: s.total,
      filesRead: s.filesRead,
      found: s.glossTermCount,
      logs: s.logs,
    })),
  );
  const combined = useCombinedRun(
    useShallow((s) => ({ pending: s.pending, stopRequested: s.stopRequested, last: s.last })),
  );
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
  // A "Build glossary, then translate" run on its glossary step, in this folder.
  const pendingHere = combined.pending?.folder === folder ? combined.pending : null;
  const lastHere = combined.last?.plan.folder === folder ? combined.last : null;
  const busyHere = runningHere || pendingHere !== null;
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

  // Episodes the glossary step has been through: all of them once it's past reading.
  const pastReading = step.phase !== null && PAST_READING.has(step.phase);
  const readNames = useMemo(() => new Set(step.filesRead), [step.filesRead]);

  const statusOf = useMemo(() => {
    const map = new Map<string, EpisodeStatus>();
    // Episodes in a run on its glossary step: read, being read (the earliest
    // unread one; batches go in order), or waiting.
    const inStep = new Set([...(pendingHere?.files ?? []), ...(pendingHere?.glossaryFiles ?? [])]);
    const reading = new Set(pendingHere?.glossaryFiles ?? []);
    const firstUnread =
      step.phase === "extracting"
        ? pendingHere?.glossaryFiles.find((n) => !readNames.has(n))
        : undefined;
    for (const file of view?.files ?? []) {
      if (inStep.has(file.name)) {
        const state = !reading.has(file.name)
          ? "waiting"
          : pastReading || readNames.has(file.name)
            ? "read"
            : file.name === firstUnread
              ? "reading"
              : "waiting";
        map.set(file.name, glossaryStepStatus(state));
        continue;
      }
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
  }, [
    view,
    ours,
    runningHere,
    run.external,
    run.files,
    run.throttle,
    stored,
    now,
    pendingHere,
    step.phase,
    readNames,
    pastReading,
  ]);

  const selected = view ? existingSelection(view.prefs.selected_files, view.files) : [];
  const sameLanguage = !!view && view.prefs.source_lang === view.prefs.target_lang;

  // The glossary step: which ticked episodes it would read, and whether it builds
  // a glossary or adds to the one there.
  const glossaryFiles = view?.supports_glossary
    ? glossaryStepFiles(selected, view.files, glossary)
    : [];
  const stepKind = glossary?.status === "ok" ? "scan" : "first";
  const glossaryFirst = glossaryFiles.length > 0 && !!view?.prefs.glossary_first;

  // While this folder's run goes, the footer shows Stop instead of Translate.
  const gate = {
    otherRun: run.running && !runningHere,
    glossaryBusy,
    hasModel: hasUsableConnection,
    sameLanguage,
  };
  const blocked = blockedReason({ ...gate, ticked: selected.length });
  const canStart = !run.running && !pendingHere && blocked === null;
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

  /** Translate these episodes now, with no glossary step. */
  const startRun = (files: string[]) => {
    if (!view || files.length === 0) return;
    forgetGlossaryStep();
    startTranslationRun(folder, files, view.prefs);
  };

  /** The footer's button: the glossary step first when it's ticked. */
  const start = () => {
    if (!view || selected.length === 0) return;
    if (!glossaryFirst) {
      startRun(selected);
      return;
    }
    startCombinedRun(view, {
      folder,
      kind: stepKind,
      glossaryFiles,
      files: selected,
      prefs: view.prefs,
    });
  };

  const stop = () => {
    if (pendingHere) {
      stopCombinedRun();
      return;
    }
    useTranslationRun.getState().requestStop();
    void ipc.cancelTranslation();
  };

  const checkNewTerms = () => {
    const ui = useGlossaryUi.getState();
    ui.forFolder(folder);
    ui.setSearch("");
    ui.setCategory(null);
    ui.setShow("new");
    void navigate({ to: "/glossary" });
  };

  usePrimaryAction(start, canStart && !!view);

  if (!view) return null;

  // ── banner ────────────────────────────────────────────────────────────────
  let banner: BannerState = { kind: "none" };
  const chainedHere = lastHere?.end.kind === "built" ? lastHere : null;
  if (pendingHere) {
    banner = {
      kind: "glossary_step",
      steps: twoSteps(pendingHere),
      progress: {
        phase: step.phase,
        done: step.done,
        total: step.total,
        read: pastReading
          ? pendingHere.glossaryFiles.length
          : pendingHere.glossaryFiles.filter((n) => readNames.has(n)).length,
        files: pendingHere.glossaryFiles.length,
        found: step.found,
      },
      stopping: combined.stopRequested,
    };
  } else if (ours && run.running) {
    banner = {
      kind: "running",
      progress: runProgress(run.files, new Map(view.files.map((f) => [f.name, f.dialogue_count]))),
      stopping: run.stopRequested,
      steps: chainedHere && !run.external ? twoSteps(chainedHere.plan) : undefined,
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
    const end = chainedHere?.end;
    banner = run.stopInfo
      ? { kind: "stopped", info: run.stopInfo, tally }
      : {
          kind: "finished",
          tally,
          glossary:
            chainedHere && end?.kind === "built"
              ? { first: chainedHere.plan.kind === "first", terms: end.terms, added: end.added }
              : undefined,
        };
  } else if (lastHere?.end.kind === "stopped") {
    banner = {
      kind: "glossary_stopped",
      first: lastHere.plan.kind === "first",
      terms: lastHere.end.terms,
    };
  } else if (lastHere?.end.kind === "failed") {
    banner = {
      kind: "glossary_failed",
      first: lastHere.plan.kind === "first",
      message: lastHere.end.message,
      models: !!lastHere.end.models,
    };
  }

  // ── footer ────────────────────────────────────────────────────────────────
  const tickedSet = new Set(selected);
  const lines = view.files
    .filter((f) => tickedSet.has(f.name))
    .reduce((n, f) => n + f.dialogue_count, 0);
  const stepSet = new Set(glossaryFiles);
  const glossaryLines = glossaryFirst
    ? view.files.filter((f) => stepSet.has(f.name)).reduce((n, f) => n + f.dialogue_count, 0)
    : 0;
  const replacing = replacingCount(selected, view.files);
  const plainLabel = translateLabel(selected.length, replacing);
  const withGlossaryLabel = combinedLabel(stepKind, selected.length, replacing);
  const logs = pendingHere
    ? step.logs.map((l) => ({ ...l, file: "glossary" }))
    : ours
      ? run.logs
      : [];

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
          locked={busyHere}
          onChange={(next) => void persist(next)}
        />

        <GlossaryRow
          view={view}
          glossary={glossary}
          unread={glossary?.status === "ok" ? glossaryFiles : []}
          building={
            pendingHere
              ? {
                  kind: pendingHere.kind,
                  found: step.found,
                  files: pendingHere.glossaryFiles.length,
                }
              : null
          }
        />

        {/* The presets come first, so they stay put while the count changes length. */}
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[12.5px]">
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
          <span className="text-muted-foreground/50">·</span>
          <span className="font-medium text-foreground tabular-nums">
            {selectionLine(selected, view.files)}
          </span>
        </div>

        <RunBanner
          state={banner}
          busy={run.running || glossaryBusy}
          onResume={(files) => startRun(files)}
          onRetry={(files) => startRun(files)}
          onRetryGlossary={() => {
            if (lastHere) startCombinedRun(view, { ...lastHere.plan, prefs: view.prefs });
          }}
          onCheckNewTerms={checkNewTerms}
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
        running={busyHere}
        stopping={pendingHere ? combined.stopRequested : run.stopRequested}
        label={glossaryFirst ? withGlossaryLabel : plainLabel}
        blocked={blocked}
        estimate={estimateLine(
          lines,
          translationLabel,
          glossaryFirst && stepKind === "first" ? null : view.glossary_terms,
          glossaryLines,
        )}
        glossaryTick={
          glossaryFiles.length > 0
            ? {
                label: glossaryTickLabel(stepKind, glossaryFiles.length),
                checked: view.prefs.glossary_first,
                onChange: (checked) => void persist({ ...view.prefs, glossary_first: checked }),
                otherLabel: glossaryFirst ? plainLabel : withGlossaryLabel,
              }
            : null
        }
        note={glossaryBroken ? "The glossary can't be read, so this run won't use it." : null}
        runningNote={
          pendingHere ? "Stopping now keeps the terms found so far and doesn’t translate." : null
        }
        logs={logs}
        logsOpen={logsOpen}
        onToggleLogs={() => setLogsOpen((o) => !o)}
        onStart={start}
        onStop={stop}
      />
    </div>
  );
}
