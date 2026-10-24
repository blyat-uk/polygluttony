import { Fragment, memo, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { EpisodeLine } from "@/types/generated/EpisodeLine";
import type { VerifyIssue } from "@/types/generated/VerifyIssue";
import { formatCs, isLive, skeletonBars, splitBreaks } from "./format";
import { dismissable, issueTag, issueTerm, type IssueTone, type LineFlag } from "./review";
import { useReview } from "./review-context";

/** Shared by the header row and every body row so the columns line up:
 *  `#`, Time, then Original and Translation splitting the rest evenly. */
export const ROW_GRID = "grid grid-cols-[60px_72px_minmax(0,1fr)_minmax(0,1fr)]";

/** Per-column padding, header and body alike. */
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

/** The translation, open for the user's own words. Enter saves, Esc cancels. */
function LineEditor({
  initial,
  onSave,
  onCancel,
}: {
  initial: string;
  onSave: (text: string) => Promise<boolean>;
  onCancel: () => void;
}) {
  const [value, setValue] = useState(initial);
  const [saving, setSaving] = useState(false);
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    ref.current?.focus();
    ref.current?.select();
  }, []);
  const save = async () => {
    if (saving || value.trim() === "") return;
    setSaving(true);
    const ok = await onSave(value);
    // On success the row closes the editor; on failure keep the text.
    if (!ok) setSaving(false);
  };
  return (
    <span className="block">
      <input
        ref={ref}
        value={value}
        disabled={saving}
        aria-label="Your translation"
        spellCheck
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            void save();
          } else if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            onCancel();
          }
        }}
        className="h-7 w-full rounded-[8px] border border-[color:var(--color-gold)] bg-[color:var(--color-surface)] px-2 text-[12.5px] text-foreground outline-none ring-[3px] ring-ring/30 disabled:opacity-60"
      />
      <span className="mt-1 block text-[10.5px] text-muted-foreground">
        Enter saves · Esc cancels{value.trim() === "" ? " · write something first" : ""}
      </span>
    </span>
  );
}

const TAG_CLS: Record<IssueTone, string> = {
  name: "text-[color:var(--color-gold)] border-[color:color-mix(in_oklch,var(--color-gold)_45%,transparent)] bg-[color:color-mix(in_oklch,var(--color-gold)_12%,transparent)]",
  untranslated:
    "text-[color:var(--color-amber)] border-[color:color-mix(in_oklch,var(--color-amber)_45%,transparent)] bg-[color:color-mix(in_oklch,var(--color-amber)_10%,transparent)]",
  meaning:
    "text-[color:var(--color-aqua)] border-[color:color-mix(in_oklch,var(--color-aqua)_45%,transparent)] bg-[color:color-mix(in_oklch,var(--color-aqua)_10%,transparent)]",
};

function Tag({ issueType }: { issueType: string }) {
  const { label, tone } = issueTag(issueType);
  return (
    <span
      className={cn(
        "inline-flex h-[17px] shrink-0 items-center rounded-[4px] border px-[5px] text-[9.5px] font-semibold tracking-[0.06em]",
        TAG_CLS[tone],
      )}
    >
      {label}
    </span>
  );
}

/** What an issue says. A name issue reads with the term and the expected
 *  name picked out. */
function IssueText({ issue }: { issue: VerifyIssue }) {
  if (issue.issue_type === "name" && issue.term && issue.expected) {
    return (
      <span>
        <span className="text-foreground">{issue.term}</span> should be{" "}
        <b className="font-semibold text-[color:var(--color-ink-emphasis)]">“{issue.expected}”</b> here,
        per your glossary.
      </span>
    );
  }
  return <span>{issue.description}</span>;
}

function Act({
  children,
  primary,
  onClick,
  disabled,
  keepSelection,
}: {
  children: ReactNode;
  primary?: boolean;
  onClick: () => void;
  disabled: boolean;
  /** Don't let the click clear the text selected in the lines. */
  keepSelection?: boolean;
}) {
  return (
    <Button
      type="button"
      size="xs"
      variant={primary ? "default" : "outline"}
      disabled={disabled}
      onMouseDown={keepSelection ? (e) => e.preventDefault() : undefined}
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
      className="h-[22px] rounded-full px-2.5 text-[11px]"
    >
      {children}
    </Button>
  );
}

/** Under a flagged line: what's wrong, and what to do about it. */
function FlagPanel({ line, flag }: { line: EpisodeLine; flag: LineFlag }) {
  const review = useReview();
  const issues = dismissable(flag);
  const untranslated = flag.untranslated || line.translation === null;
  const off = !review || !review.enabled || review.busy;
  const editing = review?.editingId === line.id;

  const actions = review ? (
    <>
      <Act primary disabled={off} onClick={() => review.retranslate([line.id])}>
        Retranslate line
      </Act>
      {untranslated ? (
        <Act disabled={off || editing} onClick={() => review.startEdit(line.id)}>
          Write it myself
        </Act>
      ) : (
        <>
          <Act disabled={off || editing} onClick={() => review.startEdit(line.id)}>
            Edit
          </Act>
          {issues.length > 0 ? (
            <Act disabled={off} onClick={() => review.looksFine(line.id, issues)}>
              Looks fine
            </Act>
          ) : null}
          <Act disabled={off} keepSelection onClick={() => review.fixInGlossary(issueTerm(flag))}>
            Fix in glossary
          </Act>
        </>
      )}
    </>
  ) : null;

  return (
    <div className={cn(ROW_GRID, "pb-2 text-[12px] leading-[17px]")}>
      <div className="col-span-3 col-start-2 flex flex-col gap-1.5 pl-2 pr-5">
        {issues.map((it, i) => (
          <div key={`${it.issue_type}-${i}`} className="flex items-start gap-2 text-foreground/80">
            <span className="pt-px">
              <Tag issueType={it.issue_type} />
            </span>
            <IssueText issue={it} />
          </div>
        ))}
        <div className="flex flex-wrap items-center gap-1.5">
          {untranslated ? <Tag issueType="untranslated" /> : null}
          {actions}
        </div>
      </div>
    </div>
  );
}

export type LineRowProps = {
  line: EpisodeLine;
  index: number;
  start: number;
  /** What this line needs a look for; undefined when nothing. */
  flag: LineFlag | undefined;
  selected: boolean;
  /** Its translation landed moments ago: play the one-shot fade. */
  fresh: boolean;
  measure: (node: HTMLDivElement | null) => void;
};

export const LineRow = memo(function LineRow({
  line,
  index,
  start,
  flag,
  selected,
  fresh,
  measure,
}: LineRowProps) {
  const review = useReview();
  // A line being re-sent reads as live first; the flag comes back once it lands.
  const live = isLive(line);
  const edge = live ? "live" : flag ? "flag" : null;
  const editing = !live && review?.editingId === line.id;
  const canEdit = !!review && review.enabled && !review.busy && !live;

  return (
    <div
      ref={measure}
      data-index={index}
      data-line={line.id}
      role="row"
      aria-rowindex={index + 2}
      aria-selected={selected}
      onClick={() => review?.select(line.id)}
      className={cn(
        "absolute left-0 top-0 w-full border-b border-border/55 text-[12.5px] leading-[18px]",
        edge === "live" && "bg-[color:color-mix(in_oklch,var(--color-gold)_3.5%,transparent)]",
        edge === "flag" && "bg-[color:color-mix(in_oklch,var(--color-amber)_5%,transparent)]",
        !edge && "hover:bg-[color:color-mix(in_oklch,var(--color-bg-hover)_55%,transparent)]",
        selected && "shadow-[inset_0_0_0_1px_color-mix(in_oklch,var(--color-gold)_45%,transparent)]",
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

      <div className={cn(ROW_GRID, "items-start py-[7px]")}>
        <div
          role="cell"
          className={cn(
            CELL_PAD[0],
            "text-[11px] tabular-nums",
            flag ? "text-[color:var(--color-amber)]" : "text-muted-foreground/75",
          )}
        >
          {line.id}
        </div>

        <div
          role="cell"
          className={cn(CELL_PAD[1], "text-[11px] tabular-nums text-muted-foreground")}
          title={`${formatCs(line.start_cs, true)} → ${formatCs(line.end_cs, true)}`}
        >
          {formatCs(line.start_cs)}
        </div>

        <div role="cell" data-col="source" className={cn(CELL_PAD[2], "break-words text-foreground/75")}>
          <Breaks text={line.source} />
        </div>

        <div
          role="cell"
          data-col="translation"
          className={cn(CELL_PAD[3], "break-words text-foreground")}
          onDoubleClick={canEdit && !editing ? () => review.startEdit(line.id) : undefined}
        >
          {editing && review ? (
            <LineEditor
              initial={line.translation ?? ""}
              onSave={(text) => review.saveEdit(line.id, text)}
              onCancel={review.cancelEdit}
            />
          ) : (
            <TranslationCell line={line} fresh={fresh} />
          )}
        </div>
      </div>

      {flag && !live ? <FlagPanel line={line} flag={flag} /> : null}
    </div>
  );
});
