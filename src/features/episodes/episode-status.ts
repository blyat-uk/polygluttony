import type { EpisodeRecord } from "@/types/generated/EpisodeRecord";
import type { FileStateKind } from "@/types/generated/FileStateKind";
import type { SourceFile } from "@/types/generated/SourceFile";
import type { VerifyIssue } from "@/types/generated/VerifyIssue";
import type { FileRow, Throttle } from "@/stores/translation-store";
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

const needsLook = (issues: VerifyIssue[] | null): EpisodeStatus => ({
  kind: "needs_look",
  label: issues && issues.length > 0 ? `Needs a look · ${issues.length}` : "Needs a look",
  color: COLOR.look,
  live: false,
  detail: null,
  detailTone: "muted",
  issues: issues ?? [],
});

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
 */
export function resolveStatus({ file, live, running, record, throttle, now }: StatusInputs): EpisodeStatus {
  const disk = file.translation;

  if (live && running) {
    if (live.state === "done") return done();
    if (live.state === "warning") return needsLook(live.issues);
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
    if (disk === "needs_look") return needsLook(record.issues);
    return fresh();
  }

  if (disk === "done") return done();
  if (disk === "needs_look") return needsLook(live?.state === "warning" ? live.issues : null);
  return fresh();
}

/** "Old Veil of Shadow - 07.warning file is still here" for an episode that has
 *  both a clean and a `.warning` output. */
export function leftoverNote(file: SourceFile, name: string): string | null {
  return file.both_variants ? `Old ${name}.warning file is still here` : null;
}
