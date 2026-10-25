import { create } from "zustand";
import type { FolderPrefs } from "@/types/generated/FolderPrefs";
import type { ProjectView } from "@/types/generated/ProjectView";
import { ipc } from "@/lib/ipc";
import { useGlossaryRun } from "@/stores/glossary-store";
import { useTranslationRun } from "@/stores/translation-store";
import { startBuild } from "@/features/glossary/start-build";
import { startTranslationRun } from "./start-run";

/** "Build glossary, then translate": what to read for the glossary, and what to
 *  translate once it's ready. */
export interface CombinedPlan {
  folder: string;
  /** "first" builds a new glossary; "scan" adds to the one there. */
  kind: "first" | "scan";
  /** Ticked episodes the glossary step reads. */
  glossaryFiles: string[];
  /** Episodes translated once the glossary is ready. */
  files: string[];
  /** The folder's settings at the click; they're locked while it runs. */
  prefs: FolderPrefs;
}

/** How a glossary step ended. Only `built` goes on to translate. */
export type GlossaryStepEnd =
  | { kind: "built"; terms: number; added: number; partial: boolean }
  | { kind: "stopped"; terms: number }
  | { kind: "failed"; message: string; models?: boolean };

interface CombinedRunState {
  /** A run on its glossary step; null otherwise. */
  pending: CombinedPlan | null;
  /** Stop was pressed during the glossary step. */
  stopRequested: boolean;
  /** How the last glossary step ended, for the Episodes banner, until the next run. */
  last: { plan: CombinedPlan; end: GlossaryStepEnd } | null;
}

export const useCombinedRun = create<CombinedRunState>(() => ({
  pending: null,
  stopRequested: false,
  last: null,
}));

/** Build or update the glossary from `plan.glossaryFiles`, then translate `plan.files`. */
export function startCombinedRun(view: ProjectView, plan: CombinedPlan) {
  if (useCombinedRun.getState().pending || useGlossaryRun.getState().busy !== null) return;
  // This run's banner takes the place of the last run's.
  useTranslationRun.getState().clearOutcome();
  useCombinedRun.setState({ pending: plan, stopRequested: false, last: null });
  // Same as Build glossary's defaults: Tidy up on, Look up names online off.
  startBuild(view, { kind: plan.kind, files: plan.glossaryFiles, tidy: true, chained: true });
}

export function stopCombinedRun() {
  useCombinedRun.setState({ stopRequested: true });
  void ipc.cancelGlossaryBuild();
}

/** A run that doesn't build a glossary first: the last glossary step's banner goes. */
export function forgetGlossaryStep() {
  if (useCombinedRun.getState().last) useCombinedRun.setState({ last: null });
}

/**
 * Carry a combined run from its glossary step to the translation, wherever the
 * user is in the app. Returns the unsubscribe.
 */
export function watchCombinedRuns(): () => void {
  return useGlossaryRun.subscribe((g, prev) => {
    const plan = useCombinedRun.getState().pending;
    if (!plan || prev.busy !== "build" || g.busy !== null || g.folder !== plan.folder) return;
    let end = stepEnd(g.summary, g.error);
    // A Stop the build never heard (pressed before it got going) still stops the run.
    if (end.kind === "built" && useCombinedRun.getState().stopRequested) {
      end = { kind: "stopped", terms: end.terms };
    }
    useCombinedRun.setState({ pending: null, stopRequested: false, last: { plan, end } });
    if (end.kind === "built") startTranslationRun(plan.folder, plan.files, plan.prefs);
  });
}

function stepEnd(
  summary: ReturnType<typeof useGlossaryRun.getState>["summary"],
  error: string | null,
): GlossaryStepEnd {
  if (error || !summary) {
    return { kind: "failed", message: error ?? "The glossary build didn't finish." };
  }
  if (summary.cancelled) return { kind: "stopped", terms: summary.terms_final };
  // Stopped mid-build because the model refused the requests (a build stops
  // early for nothing else): what was found is saved.
  if (summary.aborted) {
    return {
      kind: "failed",
      message: "The model refused the request. Check it on the Models page.",
      models: true,
    };
  }
  // Nothing came back: the model is down or unreachable, and translating would fail too.
  if (summary.batches_total > 0 && summary.batches_processed === 0) {
    return {
      kind: "failed",
      message: "The model didn't answer. Check it on the Models page.",
      models: true,
    };
  }
  return {
    kind: "built",
    terms: summary.terms_final,
    added: summary.diff.total_added,
    // Some batches failed: the glossary goes ahead with what came back, and
    // those episodes aren't on record as read, so they're offered again.
    partial: summary.batches_processed < summary.batches_total,
  };
}
