import { useEffect, useState } from "react";
import { FileStateChip } from "@/features/translate/file-state-chip";
import { cn } from "@/lib/utils";
import type { EpisodeSnapshot } from "@/types/generated/EpisodeSnapshot";
import { describeActivity, formatElapsed, type ActivityTone } from "./format";

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

export function EpisodeHeader({
  file,
  snapshot,
  liveIds,
  flaggedLines,
  refreshError,
}: {
  file: string;
  /** Null while loading and for episodes outside the run. */
  snapshot: EpisodeSnapshot | null;
  liveIds: number[];
  flaggedLines: number;
  refreshError: string | null;
}) {
  const activity = snapshot ? describeActivity(snapshot, liveIds, flaggedLines) : null;

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
        {snapshot ? (
          <span className="ml-auto shrink-0 text-[11.5px] tabular-nums text-muted-foreground">
            <b className="font-semibold text-[color:var(--color-gold-hi)]">
              {snapshot.translated.toLocaleString()}
            </b>{" "}
            / {snapshot.total.toLocaleString()} lines
          </span>
        ) : null}
      </div>
      {activity ? (
        <p className={cn("mt-1 truncate text-[12px]", TONE_CLS[activity.tone])}>
          <span aria-live="polite">{activity.text}</span>
          {activity.since !== null ? (
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
