import type { ReactNode } from "react";
import { useNavigate } from "@tanstack/react-router";
import { CheckCircle, Info, Warning, WarningCircle } from "@phosphor-icons/react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { plural } from "@/lib/utils";
import { ReactorBar } from "@/features/translate/reactor-bar";
import type { RunProgress } from "@/features/translate/telemetry";
import type { GlossaryPhase } from "@/types/generated/GlossaryPhase";
import type { StopInfo } from "@/types/generated/StopInfo";
import {
  errorStopLine,
  finishedLine,
  fixedOnModels,
  sentence,
  stoppedLine,
  type RunTally,
} from "./run-summary";

type Tone = "neutral" | "good" | "look" | "bad";

const TONE_CLS: Record<Tone, string> = {
  neutral: "border-border bg-[color:var(--card)]",
  good: "border-[color:var(--color-success)]/35 bg-[color:var(--color-success)]/5",
  look: "border-[color:var(--color-alert)]/35 bg-[color:var(--color-alert)]/5",
  bad: "border-[color:var(--color-danger)]/35 bg-[color:var(--color-danger)]/5",
};

const TONE_ICON: Record<Tone, ReactNode> = {
  neutral: <Info weight="fill" className="size-4 shrink-0 text-muted-foreground" />,
  good: <CheckCircle weight="fill" className="size-4 shrink-0 text-[color:var(--color-success)]" />,
  look: <Warning weight="fill" className="size-4 shrink-0 text-[color:var(--color-alert)]" />,
  bad: <WarningCircle weight="fill" className="size-4 shrink-0 text-[color:var(--color-danger)]" />,
};

function Banner({ tone, children, actions }: { tone: Tone; children: ReactNode; actions?: ReactNode }) {
  return (
    <div
      role="status"
      className={cn("flex items-center gap-3 rounded-xl border px-4 py-2.5 text-[12.5px]", TONE_CLS[tone])}
    >
      {TONE_ICON[tone]}
      <span className="min-w-0 flex-1">{children}</span>
      {actions ? <span className="flex shrink-0 items-center gap-2">{actions}</span> : null}
    </div>
  );
}

/** The banner's main action: outlined in gold, so the footer keeps the one gold button. */
const LEAD = "border-[color:var(--color-gold)]/60 text-primary";

/** A run that builds or updates the glossary first: which, and how many episodes it translates. */
export interface TwoSteps {
  first: boolean;
  translating: number;
}

/** Where a glossary step is. */
export interface GlossaryStepProgress {
  phase: GlossaryPhase | null;
  /** Batches back, of `total`. */
  done: number;
  total: number;
  /** Episodes read, of `files`. */
  read: number;
  files: number;
  /** Terms found so far. */
  found: number;
}

/** What the glossary step added, said at the start of the finished banner. */
export interface GlossaryNote {
  first: boolean;
  terms: number;
  added: number;
}

export type BannerState =
  | { kind: "glossary_step"; steps: TwoSteps; progress: GlossaryStepProgress; stopping: boolean }
  | { kind: "running"; progress: RunProgress; stopping: boolean; steps?: TwoSteps }
  | { kind: "start_error"; message: string; noModel: boolean; retry: { files: string[]; resume: boolean } | null }
  | { kind: "stopped"; info: StopInfo; tally: RunTally }
  | { kind: "finished"; tally: RunTally; glossary?: GlossaryNote }
  | { kind: "glossary_stopped"; first: boolean; terms: number }
  | { kind: "glossary_failed"; first: boolean; message: string; models: boolean }
  | { kind: "none" };

const PHASE_LINE: Record<GlossaryPhase, string> = {
  loading: "Reading the episodes…",
  reference: "Reading your translated episodes…",
  extracting: "Finding names",
  normalizing: "Tidying up the glossary…",
  personalizing: "Looking up names online…",
  saving: "Saving the glossary…",
};

function glossaryStepLine(p: GlossaryStepProgress): string {
  if (p.phase !== "extracting") return PHASE_LINE[p.phase ?? "loading"];
  return `Finding names · ${p.read} of ${plural(p.files, "episode")} read · ${plural(p.found, "term")} found`;
}

/** How far the glossary step is: its batches, then full while it tidies up and saves. */
function glossaryStepFraction(p: GlossaryStepProgress): number {
  if (p.phase === "extracting") return p.total > 0 ? p.done / p.total : 0;
  return p.phase === "normalizing" || p.phase === "personalizing" || p.phase === "saving" ? 1 : 0;
}

/** ① Build the glossary ── ② Translate 3 episodes, with the current step lit. */
function StepTrail({ steps, at }: { steps: TwoSteps; at: 1 | 2 }) {
  const items = [
    steps.first ? "Build the glossary" : "Update the glossary",
    `Translate ${plural(steps.translating, "episode")}`,
  ];
  return (
    <ol className="flex items-center gap-2.5 text-[12px]">
      {items.map((label, i) => {
        const n = (i + 1) as 1 | 2;
        const state = n < at ? "done" : n === at ? "now" : "next";
        return (
          <li key={label} className="flex items-center gap-2.5">
            {i > 0 ? <span aria-hidden className="h-px w-8 bg-border" /> : null}
            <span
              aria-current={state === "now" ? "step" : undefined}
              className={cn(
                "flex items-center gap-1.5",
                state === "now" && "text-foreground",
                state === "done" && "text-[color:var(--color-success)]",
                state === "next" && "text-muted-foreground",
              )}
            >
              <span
                className={cn(
                  "grid size-[18px] place-items-center rounded-full border text-[10.5px] font-semibold",
                  state === "now" && "border-primary bg-primary text-primary-foreground",
                  state === "done" && "border-[color:var(--color-success)]",
                  state === "next" && "border-border",
                )}
              >
                {state === "done" ? "✓" : n}
              </span>
              {label}
            </span>
          </li>
        );
      })}
    </ol>
  );
}

/** "Glossary built with 12 terms. ", "Glossary updated with 3 new terms. " */
function glossarySentence(g: GlossaryNote): string {
  if (g.first) return `Glossary built with ${plural(g.terms, "term")}. `;
  return g.added > 0
    ? `Glossary updated with ${plural(g.added, "new term")}. `
    : "No new names for the glossary. ";
}

/**
 * One slot above the table for what the run is doing or how it ended: live
 * progress, a refused start, a stop (by you or by an error) with what to do
 * next, or the result.
 */
export function RunBanner({
  state,
  busy,
  onResume,
  onRetry,
  onRetryGlossary,
  onCheckNewTerms,
}: {
  state: BannerState;
  /** Something else is running; the banner's buttons wait. */
  busy: boolean;
  onResume: (files: string[]) => void;
  onRetry: (files: string[], resume: boolean) => void;
  /** Run the glossary step and the translation again. */
  onRetryGlossary: () => void;
  /** Open the glossary on its NEW terms. */
  onCheckNewTerms: () => void;
}) {
  const navigate = useNavigate();
  const openModels = (
    <Button variant="outline" size="xs" className={LEAD} onClick={() => void navigate({ to: "/connections" })}>
      Open Models
    </Button>
  );

  switch (state.kind) {
    case "none":
      return null;

    case "glossary_step": {
      const { steps, progress, stopping } = state;
      return (
        <div className="space-y-1.5 px-0.5">
          <StepTrail steps={steps} at={1} />
          <div className="flex justify-between gap-3 text-[11px] text-muted-foreground tabular-nums">
            <span>
              {stopping ? "Stopping… " : ""}
              {glossaryStepLine(progress)}
            </span>
            <span className="shrink-0">Step 1 of 2</span>
          </div>
          <ReactorBar done={glossaryStepFraction(progress)} total={1} />
        </div>
      );
    }

    case "glossary_stopped":
      return (
        <Banner tone="neutral">
          {state.first
            ? `Stopped while building the glossary. ${plural(state.terms, "term")} kept; nothing was translated.`
            : "Stopped while updating the glossary. Nothing was translated."}
        </Banner>
      );

    case "glossary_failed":
      return (
        <Banner
          tone="bad"
          actions={
            <>
              {state.models ? openModels : null}
              <Button variant="outline" size="xs" disabled={busy} onClick={onRetryGlossary}>
                Try again
              </Button>
            </>
          }
        >
          <span className="font-semibold">
            The glossary couldn’t be {state.first ? "built" : "updated"}:{" "}
          </span>
          {sentence(state.message)} Nothing was translated.
        </Banner>
      );

    case "running": {
      const { progress, stopping, steps } = state;
      if (progress.total === 0) return steps ? <StepTrail steps={steps} at={2} /> : null;
      const pct = `${Math.round((progress.done / progress.total) * 100)}%`;
      return (
        <div className="space-y-1.5 px-0.5">
          {steps ? <StepTrail steps={steps} at={2} /> : null}
          <div className="flex justify-between gap-3 text-[11px] text-muted-foreground tabular-nums">
            <span>
              {stopping ? "Stopping… " : ""}
              {progress.done.toLocaleString()} of {plural(progress.total, "line")} ·{" "}
              {progress.finished} of {plural(progress.files, "episode")} finished
            </span>
            <span className="shrink-0">{steps ? `Step 2 of 2 · ${pct}` : pct}</span>
          </div>
          {/* One segment: thousands of line dividers would bury the fill. */}
          <ReactorBar done={progress.done / progress.total} total={1} />
        </div>
      );
    }

    case "start_error": {
      const { retry } = state;
      return (
        <Banner
          tone="bad"
          actions={
            <>
              {state.noModel ? openModels : null}
              {retry && retry.files.length > 0 ? (
                <Button
                  variant="outline"
                  size="xs"
                  disabled={busy}
                  onClick={() => onRetry(retry.files, retry.resume)}
                >
                  Try again
                </Button>
              ) : null}
            </>
          }
        >
          <span className="font-medium">Couldn't start: </span>
          {sentence(state.message)}
        </Banner>
      );
    }

    case "stopped": {
      const { info, tally } = state;
      if (info.by_user) {
        const n = tally.unfinished.length;
        return (
          <Banner
            tone="neutral"
            actions={
              n > 0 ? (
                <Button
                  variant="outline"
                  size="xs"
                  className={LEAD}
                  disabled={busy}
                  onClick={() => onResume(tally.unfinished)}
                >
                  Resume {plural(n, "episode")}
                </Button>
              ) : null
            }
          >
            {stoppedLine(tally)}
          </Banner>
        );
      }
      const retry = [...tally.unfinished, ...tally.failed];
      return (
        <Banner
          tone="bad"
          actions={
            <>
              {fixedOnModels(info.kind) ? openModels : null}
              {retry.length > 0 ? (
                <Button variant="outline" size="xs" disabled={busy} onClick={() => onRetry(retry, true)}>
                  Try again
                </Button>
              ) : null}
            </>
          }
        >
          <span className="font-semibold">Stopped: </span>
          {errorStopLine(info, tally)}
        </Banner>
      );
    }

    case "finished": {
      const { tally, glossary } = state;
      const failed = tally.failed.length;
      const tone: Tone = failed > 0 ? "bad" : tally.look.length > 0 ? "look" : "good";
      const newTerms = glossary ? (glossary.first ? glossary.terms : glossary.added) : 0;
      return (
        <Banner
          tone={tone}
          actions={
            <>
              {newTerms > 0 ? (
                <Button variant="outline" size="xs" onClick={onCheckNewTerms}>
                  Check the new terms
                </Button>
              ) : null}
              {failed > 0 ? (
                <Button variant="outline" size="xs" disabled={busy} onClick={() => onRetry(tally.failed, false)}>
                  Try again
                </Button>
              ) : null}
            </>
          }
        >
          {glossary ? glossarySentence(glossary) : null}
          {finishedLine(tally)}
        </Banner>
      );
    }
  }
}
