import type { EpisodeRecord } from "@/types/generated/EpisodeRecord";
import type { FileStateKind } from "@/types/generated/FileStateKind";
import type { SourceFile } from "@/types/generated/SourceFile";
import type { VerifyIssue } from "@/types/generated/VerifyIssue";
import type { FileRow, Throttle } from "@/stores/translation-store";
import { plural } from "@/lib/utils";
import { STATE_COLOR, STATE_LABEL } from "@/features/translate/file-state-chip";

export type StatusKind =
  | "live"
  | "done"
  | "needs_look"
  | "new"
  | "not_finished"
  | "not_started"
  | "failed";

export interface EpisodeStatus {
  kind: StatusKind;
  /** Chip text: "Done", "Needs a look · 3", "Translating…". */
  label: string;
  /** Chip dot colour (a CSS colour). */
  color: string;
  /** The dot pulses: live work. */
  live: boolean;
  /** Muted text after the chip: progress, a reason, a wait. */
  detail: string | null;
  /** The detail is a retry/wait note (amber) rather than plain progress. */
  detailTone: "muted" | "alert" | "danger";
  /** Open issues, when known (for the fold-out). */
  issues: VerifyIssue[];
}

const IN_FLIGHT = new Set<FileStateKind>(["translating", "retranslating", "cleanup", "verifying"]);

const COLOR = {
  done: "var(--color-success)",
  look: "var(--color-alert)",
  new: "var(--color-gold)",
  idle: "var(--muted-foreground)",
  failed: "var(--color-danger)",
};

const done = (): EpisodeStatus => ({
  kind: "done",
  label: "Done",
  color: COLOR.done,
  live: false,
  detail: null,
  detailTone: "muted",
  issues: [],
});

/** The engine lists only the first few untranslated lines as issues, then one
 *  "…and N more untranslated lines" entry on line 0, which isn't a line. */
const isLine = (it: VerifyIssue) => it.line_id > 0;

/** Untranslated lines the issues account for: the listed ones plus the
 *  summary entry's "N more". */
export function untranslatedInIssues(issues: readonly VerifyIssue[]): number {
  let n = 0;
  for (const it of issues) {
    if (it.issue_type !== "untranslated") continue;
    if (isLine(it)) n++;
    else n += Number(/\d[\d,]*/.exec(it.description)?.[0].replace(/,/g, "") ?? 0);
  }
  return n;
}

/** Lines that need a look, counted the way the episode window counts them. */
export interface LookCount {
  /** Lines left untranslated. */
  missing: number;
  /** Other lines an issue names (a name, a meaning). */
  others: number;
  lines: number;
}

/**
 * `missing` comes from the line counts when they're known (the issues list
 * only the first few untranslated lines); otherwise from the issues. Every
 * other issue counts once per line, and the "…and N more" entry never does.
 */
export function lookCount(issues: readonly VerifyIssue[], missing: number | null): LookCount {
  const gone = Math.max(0, missing ?? untranslatedInIssues(issues));
  const untranslatedIds = new Set(
    issues.filter((i) => isLine(i) && i.issue_type === "untranslated").map((i) => i.line_id),
  );
  const others = new Set(
    issues
      .filter((i) => isLine(i) && i.issue_type !== "untranslated" && !untranslatedIds.has(i.line_id))
      .map((i) => i.line_id),
  ).size;
  return { missing: gone, others, lines: gone + others };
}

/** A finished row of the run: lines it left untranslated. The counts can lag
 *  a last clean-up, so the issues get a say too. */
export function rowMissing(row: FileRow): number {
  // Done means every line came back.
  if (row.state === "done") return 0;
  const fromCounts = row.total > 0 ? Math.max(0, row.total - row.translated) : 0;
  return Math.max(fromCounts, untranslatedInIssues(row.issues));
}

/** A stored result: lines it left untranslated, when its counts are known. */
export function recordMissing(record: EpisodeRecord): number | null {
  return record.total_lines > 0 ? Math.max(0, record.total_lines - record.translated_lines) : null;
}

/** "Needs a look · 3", or with lines missing "Needs a look · 175 lines missing"
 *  ("…, 2 more" when other lines need a look too). */
export function lookLabel(c: LookCount): string {
  if (c.lines === 0) return "Needs a look";
  if (c.missing === 0) return `Needs a look · ${c.lines.toLocaleString()}`;
  const missing = `${plural(c.missing, "line")} missing`;
  return c.others > 0
    ? `Needs a look · ${missing}, ${c.others.toLocaleString()} more`
    : `Needs a look · ${missing}`;
}

/** By line; summary entries (line 0) last. */
export function byLine(issues: readonly VerifyIssue[]): VerifyIssue[] {
  const at = (i: VerifyIssue) => (isLine(i) ? i.line_id : Number.MAX_SAFE_INTEGER);
  return [...issues].sort((a, b) => at(a) - at(b));
}

const needsLook = (issues: VerifyIssue[] | null, missing: number | null): EpisodeStatus => {
  const list = issues ?? [];
  return {
    kind: "needs_look",
    label: lookLabel(lookCount(list, missing)),
    color: COLOR.look,
    live: false,
    detail: null,
    detailTone: "muted",
    issues: byLine(list),
  };
};

const fresh = (): EpisodeStatus => ({
  kind: "new",
  label: "New",
  color: COLOR.new,
  live: false,
  detail: null,
  detailTone: "muted",
  issues: [],
});

const notFinished = (detail: string | null): EpisodeStatus => ({
  kind: "not_finished",
  label: "Not finished",
  color: COLOR.idle,
  live: false,
  detail,
  detailTone: "muted",
  issues: [],
});

const notStarted = (): EpisodeStatus => ({
  kind: "not_started",
  label: "Not started",
  color: COLOR.idle,
  live: false,
  detail: null,
  detailTone: "muted",
  issues: [],
});

const failed = (reason: string | null): EpisodeStatus => ({
  kind: "failed",
  label: "Failed",
  color: COLOR.failed,
  live: false,
  detail: reason,
  detailTone: "danger",
  issues: [],
});

/** "200/612 · batch 1 of 3", plus "↺ 1 · a reply came back incomplete" after a retry. */
export function progressText(row: FileRow): string | null {
  if (row.total === 0) return null;
  let text = `${row.translated}/${row.total}`;
  if (row.totalBatches > 0) text += ` · batch ${row.batch} of ${row.totalBatches}`;
  return text;
}

export function retryText(row: FileRow): string | null {
  if (row.retries === 0) return null;
  return row.retryReason ? `↺ ${row.retries} · ${row.retryReason}` : `↺ ${row.retries}`;
}

/** "Waiting — Anthropic asked us to slow down · 18s"; null once the wait is over. */
export function throttleText(throttle: Throttle | null, now: number): string | null {
  if (!throttle) return null;
  const left = Math.ceil((throttle.until - now) / 1000);
  if (left <= 0) return null;
  return `Waiting — ${throttle.provider} asked us to slow down · ${left}s`;
}

export interface StatusInputs {
  file: SourceFile;
  /** This file's row in the current or last run of THIS folder. */
  live: FileRow | undefined;
  /** That run is still going. */
  running: boolean;
  /** That run was started elsewhere (an episode window's review, a glossary
   *  term's retranslation): its row counts only while it runs. */
  external?: boolean;
  /** The stored result of the episode's last run. */
  record: EpisodeRecord | undefined;
  throttle: Throttle | null;
  now: number;
}

/**
 * One status per episode, from the freshest source that knows:
 *
 * 1. The run, while it's going (live chip, progress, retries, waits) — and
 *    after it, for what the disk can't tell: Failed, Not finished, Not started.
 * 2. The stored result of the episode's last run (issue count, failure reason),
 *    trusted for Done / Needs a look only while the disk still has an output.
 * 3. The disk: a clean output is Done, a `.warning` one Needs a look, none New.
 *
 * A finished run's Done / Needs a look only adds detail (the issue count) when
 * the disk agrees, so a file fixed or renamed by hand afterwards reads right.
 *
 * A run started elsewhere retranslates a few lines of finished episodes: once
 * it ends, the stored result and the disk speak again. A failure it hit shows
 * as a note, never as "Failed" over a translated episode.
 */
export function resolveStatus(inputs: StatusInputs): EpisodeStatus {
  const { file, live, running, external, record, throttle, now } = inputs;
  const disk = file.translation;

  if (live && !running && external) {
    const base = resolveStatus({ ...inputs, live: undefined, external: false });
    if (live.state !== "failed" || !live.error || base.kind === "failed") return base;
    return { ...base, detail: `Couldn’t retranslate: ${live.error}`, detailTone: "danger" };
  }

  if (live && running) {
    if (live.state === "done") return done();
    if (live.state === "warning") return needsLook(live.issues, rowMissing(live));
    if (live.state === "failed") return failed(live.error);
    if (live.state === "stopped") return notFinished("the run stopped");
    const inFlight = IN_FLIGHT.has(live.state);
    const wait = inFlight ? throttleText(throttle, now) : null;
    const retry = inFlight ? retryText(live) : null;
    const progress = inFlight ? progressText(live) : null;
    return {
      kind: "live",
      label: STATE_LABEL[live.state],
      color: STATE_COLOR[live.state],
      live: inFlight,
      detail: wait ?? ([progress, retry].filter(Boolean).join(" ") || null),
      detailTone: wait || retry ? "alert" : "muted",
      issues: [],
    };
  }

  if (live && !running) {
    if (live.state === "failed") return failed(live.error);
    if (live.state === "stopped" || IN_FLIGHT.has(live.state)) return notFinished("the run stopped");
    if (live.state === "pending") return notStarted();
  }

  if (record) {
    if (record.outcome === "failed") return failed(record.reason);
    if (record.outcome === "stopped") return notFinished(record.reason ?? "the run stopped");
    if (disk === "done") return done();
    if (disk === "needs_look") return needsLook(record.issues, recordMissing(record));
    return fresh();
  }

  if (disk === "done") return done();
  if (disk === "needs_look") {
    return live?.state === "warning" ? needsLook(live.issues, rowMissing(live)) : needsLook(null, null);
  }
  return fresh();
}

/** "Old Veil of Shadow - 07.warning file is still here" for an episode that has
 *  both a clean and a `.warning` output. */
export function leftoverNote(file: SourceFile, name: string): string | null {
  return file.both_variants ? `Old ${name}.warning file is still here` : null;
}
