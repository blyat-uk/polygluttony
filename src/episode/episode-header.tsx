import { useEffect, useState, type ReactNode } from "react";
import { CaretLeftIcon, CaretRightIcon, CheckSquareIcon, SquareIcon } from "@phosphor-icons/react";
import { FileStateChip } from "@/features/translate/file-state-chip";
import { cn, plural } from "@/lib/utils";
import type { EpisodeSnapshot } from "@/types/generated/EpisodeSnapshot";
import { describeActivity, formatElapsed, secondsLeft, type ActivityTone } from "./format";

const TONE_CLS: Record<ActivityTone, string> = {
  live: "text-foreground/85",
  idle: "text-muted-foreground",
  done: "text-muted-foreground",
  warn: "text-[color:var(--color-amber)]",
  failed: "text-[color:var(--color-danger)]",
};

/** Seconds since `since` (epoch ms). Owns its own tick, so only this text
 *  re-renders each second — not the header, never the table. */
function Elapsed({ since }: { since: number }) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    setNow(Date.now());
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [since]);
  return <span className="tabular-nums">{formatElapsed(now - since)}</span>;
}

/** Seconds left until `until` (epoch ms), ticking down. */
function Countdown({ until }: { until: number }) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    setNow(Date.now());
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [until]);
  return <span className="tabular-nums">{secondsLeft(until, now)}s</span>;
}

/** "3 left ‹ 2 / 3 ›": step through the lines that need a look. */
export type Navigator = {
  flagged: number;
  /** 1-based position of the selected line among them, if it's one. */
  position: number | null;
  onPrev: () => void;
  onNext: () => void;
};

function NavButton({ label, onClick, children }: { label: string; onClick: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      aria-label={label}
      onClick={onClick}
      className="inline-flex size-[22px] items-center justify-center rounded-full border border-[color:var(--input)] bg-[color:var(--color-raised)] text-foreground/85 outline-none hover:border-[color:var(--color-gold)] hover:text-[color:var(--color-gold-hi)] focus-visible:ring-[3px] focus-visible:ring-ring/50"
    >
      {children}
    </button>
  );
}

export function EpisodeHeader({
  file,
  snapshot,
  liveIds,
  flaggedLines,
  refreshError,
  nav,
  onlyFlagged,
  onToggleOnlyFlagged,
}: {
  file: string;
  /** Null while loading and for episodes outside the run. */
  snapshot: EpisodeSnapshot | null;
  liveIds: number[];
  flaggedLines: number;
  refreshError: string | null;
  /** Shown once the episode has lines that need a look. */
  nav: Navigator | null;
  onlyFlagged: boolean;
  onToggleOnlyFlagged: () => void;
}) {
  const activity = snapshot ? describeActivity(snapshot, liveIds, flaggedLines) : null;
  const running = snapshot?.run_active ?? false;

  return (
    <header className="shrink-0 border-b border-border px-5 pb-2.5 pt-3">
      <div className="flex items-center gap-3">
        <h1
          className="min-w-0 truncate font-mono text-[13.5px] text-[color:var(--color-ink-emphasis)]"
          title={file}
        >
          {file}
        </h1>
        {snapshot ? <FileStateChip state={snapshot.state} /> : null}
        <div className="ml-auto flex shrink-0 items-center gap-2.5">
          {nav && nav.flagged > 0 ? (
            <div className="flex items-center gap-1.5 text-[11.5px] tabular-nums" aria-label="Lines that need a look">
              <span className="text-muted-foreground">{nav.flagged.toLocaleString()} left</span>
              <NavButton label="Previous line that needs a look" onClick={nav.onPrev}>
                <CaretLeftIcon size={11} weight="bold" />
              </NavButton>
              <span className="min-w-[44px] text-center text-foreground/85">
                {nav.position ?? "–"} / {nav.flagged.toLocaleString()}
              </span>
              <NavButton label="Next line that needs a look" onClick={nav.onNext}>
                <CaretRightIcon size={11} weight="bold" />
              </NavButton>
            </div>
          ) : null}
          {nav && (nav.flagged > 0 || onlyFlagged) ? (
            <button
              type="button"
              aria-pressed={onlyFlagged}
              onClick={onToggleOnlyFlagged}
              className={cn(
                "inline-flex h-[22px] items-center gap-1.5 rounded-full border px-2.5 text-[11px] outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50",
                onlyFlagged
                  ? "border-[color:var(--color-gold)] bg-[color:color-mix(in_oklch,var(--color-gold)_10%,transparent)] text-[color:var(--color-gold)]"
                  : "border-[color:var(--input)] text-foreground/85 hover:border-[color:var(--color-gold)]",
              )}
            >
              {onlyFlagged ? <CheckSquareIcon size={12} weight="bold" /> : <SquareIcon size={12} />}
              Only flagged
            </button>
          ) : null}
          {snapshot && (running || !nav || nav.flagged === 0) ? (
            <span className="text-[11.5px] tabular-nums text-muted-foreground">
              <b className="font-semibold text-[color:var(--color-gold-hi)]">
                {snapshot.translated.toLocaleString()}
              </b>{" "}
              / {plural(snapshot.total, "line")}
            </span>
          ) : null}
        </div>
      </div>
      {activity ? (
        <p className={cn("mt-1 truncate text-[12px]", TONE_CLS[activity.tone])}>
          <span aria-live="polite">{activity.text}</span>
          {activity.until ? (
            <>
              <span className="text-muted-foreground"> · </span>
              <Countdown until={activity.until} />
            </>
          ) : activity.since !== null ? (
            <>
              <span className="text-muted-foreground"> · </span>
              <Elapsed since={activity.since} />
            </>
          ) : null}
        </p>
      ) : null}
      {refreshError ? (
        <p className="mt-0.5 truncate text-[11px] text-[color:var(--color-danger)]" title={refreshError}>
          Couldn’t refresh: {refreshError}
        </p>
      ) : null}
    </header>
  );
}
