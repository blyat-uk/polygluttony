import type { ReactNode } from "react";
import { useNavigate } from "@tanstack/react-router";
import { CheckCircle, Info, Warning, WarningCircle } from "@phosphor-icons/react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { plural } from "@/lib/utils";
import { ReactorBar } from "@/features/translate/reactor-bar";
import type { RunProgress } from "@/features/translate/telemetry";
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

export type BannerState =
  | { kind: "running"; progress: RunProgress; stopping: boolean }
  | { kind: "start_error"; message: string; noModel: boolean; retry: { files: string[]; resume: boolean } | null }
  | { kind: "stopped"; info: StopInfo; tally: RunTally }
  | { kind: "finished"; tally: RunTally }
  | { kind: "none" };

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
}: {
  state: BannerState;
  /** Something else is running; the banner's buttons wait. */
  busy: boolean;
  onResume: (files: string[]) => void;
  onRetry: (files: string[], resume: boolean) => void;
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

    case "running": {
      const { progress, stopping } = state;
      if (progress.total === 0) return null;
      return (
        <div className="space-y-1.5 px-0.5">
          <div className="flex justify-between text-[11px] text-muted-foreground tabular-nums">
            <span>
              {stopping ? "Stopping… " : ""}
              {progress.done.toLocaleString()} of {plural(progress.total, "line")} ·{" "}
              {progress.finished} of {plural(progress.files, "episode")} finished
            </span>
            <span>{Math.round((progress.done / progress.total) * 100)}%</span>
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
      const { tally } = state;
      const failed = tally.failed.length;
      const tone: Tone = failed > 0 ? "bad" : tally.look.length > 0 ? "look" : "good";
      return (
        <Banner
          tone={tone}
          actions={
            failed > 0 ? (
              <Button variant="outline" size="xs" disabled={busy} onClick={() => onRetry(tally.failed, false)}>
                Try again
              </Button>
            ) : null
          }
        >
          {finishedLine(tally)}
        </Banner>
      );
    }
  }
}
