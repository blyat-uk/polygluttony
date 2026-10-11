import { Fragment, memo, useMemo, useState } from "react";
import { WarningCircleIcon } from "@phosphor-icons/react";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import type { EpisodeLine } from "@/types/generated/EpisodeLine";
import type { VerifyIssue } from "@/types/generated/VerifyIssue";
import { formatCs, isLive, skeletonBars, splitBreaks } from "./format";

/** Shared by the header row and every body row so the columns line up:
 *  `#`, Time, then Original and Translation splitting the rest evenly. */
export const ROW_GRID = "grid grid-cols-[60px_72px_minmax(0,1fr)_minmax(0,1fr)]";

/** Per-column padding, header and body alike. The `#` column's wide left
 *  padding is where a flagged line's marker sits. */
export const CELL_PAD = ["pl-5 pr-2", "px-2", "px-3", "pl-3 pr-5"] as const;

const SHIMMER_MS = 1400;

/** The app's gold signal shimmer (as on the old batch meter). Every instance
 *  starts at the same phase of a shared clock, so a block of skeleton rows
 *  sweeps together however rows mount while scrolling. Hidden under reduced
 *  motion; the track it sits on stays as a static bar. */
function Shimmer({ className }: { className?: string }) {
  const [delay] = useState(() => -(performance.now() % SHIMMER_MS));
  return (
    <i
      aria-hidden
      className={cn(
        "pointer-events-none block motion-reduce:hidden [background:linear-gradient(90deg,transparent,var(--color-gold-deep),var(--color-gold),var(--color-gold-deep),transparent)] [background-size:220%_100%] [animation:sig-shimmer_1.4s_linear_infinite]",
        className,
      )}
      style={{ animationDelay: `${delay}ms` }}
    />
  );
}

/** Display text with `\N` / `\n` as real line breaks. */
function Breaks({ text }: { text: string }) {
  const parts = splitBreaks(text);
  return (
    <>
      {parts.map((part, i) => (
        <Fragment key={i}>
          {i > 0 ? <br /> : null}
          {part}
        </Fragment>
      ))}
    </>
  );
}

/** Bars standing in for a translation that's on its way: one per likely
 *  wrapped line, each the height of a text line so the row barely moves when
 *  the words land. */
function Skeleton({ id, source }: { id: number; source: string }) {
  const bars = useMemo(() => skeletonBars(id, source), [id, source]);
  return (
    <span className="block">
      <span className="sr-only">Translating</span>
      {bars.map((width, i) => (
        <span key={i} aria-hidden className="flex h-[18px] items-center">
          <span
            className="relative h-[7px] overflow-hidden rounded-full bg-[color:color-mix(in_oklch,var(--color-gold)_11%,transparent)]"
            style={{ width: `${width}%` }}
          >
            <Shimmer className="absolute inset-0 opacity-55" />
          </span>
        </span>
      ))}
    </span>
  );
}

function TranslationCell({ line, fresh }: { line: EpisodeLine; fresh: boolean }) {
  const text = line.translation;
  switch (line.status) {
    case "sending":
      return <Skeleton id={line.id} source={line.source} />;
    case "resending":
      // The old text stays readable but dimmed under the shimmer until the new
      // text replaces it.
      return text === null ? (
        <Skeleton id={line.id} source={line.source} />
      ) : (
        <span className="relative block">
          <span className="sr-only">Re-sending: </span>
          <span className="text-foreground/40">
            <Breaks text={text} />
          </span>
          <Shimmer className="absolute inset-0 rounded-sm opacity-[0.18]" />
        </span>
      );
    case "translated":
      if (text === null) break;
      // Keyed by the text so a changed translation remounts and fades in again.
      return (
        <span
          key={text}
          className={cn("block", fresh && "motion-safe:[animation:ep-land_1.1s_ease-out]")}
        >
          <Breaks text={text} />
        </span>
      );
    case "untranslated":
      return <span className="text-[11.5px] italic text-muted-foreground/70">not translated</span>;
    case "waiting":
      break;
  }
  return (
    <span className="text-muted-foreground/35" aria-label="Waiting">
      —
    </span>
  );
}

/** Amber marker on a flagged line; hover or focus lists what verify found. */
function IssueMarker({ id, issues }: { id: number; issues: VerifyIssue[] }) {
  const n = issues.length;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          aria-label={`Line ${id}: ${n} ${n === 1 ? "issue" : "issues"}`}
          className="absolute left-[3px] top-[1.5px] inline-flex size-[15px] items-center justify-center rounded-sm text-[color:var(--color-amber)] outline-none hover:text-[color:var(--color-gold-hi)] focus-visible:ring-1 focus-visible:ring-[color:var(--color-amber)]"
        >
          <WarningCircleIcon size={13} weight="fill" />
        </button>
      </TooltipTrigger>
      <TooltipContent side="right" align="start" className="max-w-sm flex-col items-start gap-1.5 py-2 text-left">
        {issues.map((it, i) => (
          <span key={i} className="block leading-snug">
            <span className="mr-1.5 text-[10px] font-semibold uppercase tracking-wide opacity-70">
              {it.issue_type}
            </span>
            {it.description}
          </span>
        ))}
      </TooltipContent>
    </Tooltip>
  );
}

export type LineRowProps = {
  line: EpisodeLine;
  index: number;
  start: number;
  /** Verify issues naming this line; undefined when it has none. */
  issues: VerifyIssue[] | undefined;
  /** Its translation landed moments ago: play the one-shot fade. */
  fresh: boolean;
  measure: (node: HTMLDivElement | null) => void;
};

export const LineRow = memo(function LineRow({ line, index, start, issues, fresh, measure }: LineRowProps) {
  // A line being re-sent reads as live first; the flag comes back once it lands.
  const edge = isLive(line) ? "live" : issues ? "flag" : null;

  return (
    <div
      ref={measure}
      data-index={index}
      role="row"
      aria-rowindex={index + 2}
      className={cn(
        ROW_GRID,
        "absolute left-0 top-0 w-full items-start border-b border-border/55 py-[7px] text-[12.5px] leading-[18px]",
        edge === "live" && "bg-[color:color-mix(in_oklch,var(--color-gold)_3.5%,transparent)]",
        edge === "flag" && "bg-[color:color-mix(in_oklch,var(--color-amber)_4%,transparent)]",
        !edge && "hover:bg-[color:color-mix(in_oklch,var(--color-bg-hover)_55%,transparent)]",
      )}
      style={{ transform: `translateY(${start}px)` }}
    >
      {edge ? (
        // Runs over the bottom hairline so a batch of rows reads as one block.
        <span
          aria-hidden
          className={cn(
            "absolute -bottom-px left-0 top-0 w-[2px]",
            edge === "live"
              ? "bg-[color:color-mix(in_oklch,var(--color-gold)_75%,transparent)]"
              : "bg-[color:var(--color-amber)]",
          )}
        />
      ) : null}

      <div role="cell" className={cn(CELL_PAD[0], "relative text-[11px] tabular-nums text-muted-foreground/75")}>
        {issues ? <IssueMarker id={line.id} issues={issues} /> : null}
        {line.id}
      </div>

      <div
        role="cell"
        className={cn(CELL_PAD[1], "text-[11px] tabular-nums text-muted-foreground")}
        title={`${formatCs(line.start_cs, true)} → ${formatCs(line.end_cs, true)}`}
      >
        {formatCs(line.start_cs)}
      </div>

      <div role="cell" className={cn(CELL_PAD[2], "break-words text-foreground/75")}>
        <Breaks text={line.source} />
      </div>

      <div role="cell" className={cn(CELL_PAD[3], "break-words text-foreground")}>
        <TranslationCell line={line} fresh={fresh} />
      </div>
    </div>
  );
});
