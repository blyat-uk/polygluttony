import type { ErrorKind } from "@/types/generated/ErrorKind";
import type { FileResult } from "@/types/generated/FileResult";
import type { StopInfo } from "@/types/generated/StopInfo";
import type { FileRow } from "@/stores/translation-store";

/** How a finished run's files ended. */
export interface RunTally {
  done: string[];
  look: string[];
  failed: string[];
  /** Cut short or never started — what Resume carries on with. */
  unfinished: string[];
  /** Lines the finished episodes left untranslated. */
  missingLines: number;
}

/** Sort a run's files by how they ended. The results say which succeeded; the
 *  live rows say which failed, and everything else didn't finish. */
export function tallyRun(files: Record<string, FileRow>, results: FileResult[] | null): RunTally {
  const tally: RunTally = { done: [], look: [], failed: [], unfinished: [], missingLines: 0 };
  const byFile = new Map((results ?? []).map((r) => [r.file, r]));
  for (const [name, row] of Object.entries(files)) {
    const r = byFile.get(name);
    if (r?.success) {
      (r.has_warnings ? tally.look : tally.done).push(name);
      tally.missingLines += Math.max(0, r.total_lines - r.translated_lines);
    }
    else if (row.state === "done") tally.done.push(name);
    else if (row.state === "warning") tally.look.push(name);
    else if (row.state === "failed") tally.failed.push(name);
    else tally.unfinished.push(name);
  }
  return tally;
}

const episodes = (n: number) => `${n} ${n === 1 ? "episode" : "episodes"}`;
const wasnt = (n: number) => (n === 1 ? "wasn't" : "weren't");
const needs = (n: number) => (n === 1 ? "needs" : "need");

/** "Translated 4 episodes — 3 done, 1 needs a look." (", 2 failed" when some failed,
 *  " · 175 lines weren't translated" when lines are missing). */
export function finishedLine(t: RunTally): string {
  const translated = t.done.length + t.look.length;
  const failed = t.failed.length;
  if (translated === 0) {
    return failed > 0
      ? `No episodes were translated — ${failed} failed.`
      : "No episodes were translated.";
  }
  const parts: string[] = [];
  if (t.done.length > 0) parts.push(`${t.done.length} done`);
  if (t.look.length > 0) parts.push(`${t.look.length} ${needs(t.look.length)} a look`);
  if (failed > 0) parts.push(`${failed} failed`);
  const missing =
    t.missingLines > 0
      ? ` · ${t.missingLines.toLocaleString()} ${t.missingLines === 1 ? "line" : "lines"} ${wasnt(t.missingLines)} translated`
      : "";
  return `Translated ${episodes(translated)} — ${parts.join(", ")}${missing}.`;
}

/** "Stopped. 3 episodes weren't finished." */
export function stoppedLine(t: RunTally): string {
  const n = t.unfinished.length;
  return n === 0 ? "Stopped." : `Stopped. ${episodes(n)} ${wasnt(n)} finished.`;
}

/** The engine's sentence, then how many episodes it left: "Anthropic rejected the
 *  API key used for Translate. 3 episodes weren't finished." */
export function errorStopLine(info: StopInfo, t: RunTally): string {
  const n = t.unfinished.length + t.failed.length;
  const message = sentence(info.message) || "The run stopped.";
  return n === 0 ? message : `${message} ${episodes(n)} ${wasnt(n)} finished.`;
}

/** Errors that a change on the Models page fixes. */
export const fixedOnModels = (kind: ErrorKind | null) =>
  kind === "auth" || kind === "billing" || kind === "model";

/** Body of the OS notification: "3 episodes done, 1 needs a look." The first
 *  count carries the noun, the rest lean on it. */
export function notificationBody(t: RunTally, stop: StopInfo | null): string {
  const counts: [number, (n: number) => string][] = [
    [t.done.length, () => "done"],
    [t.look.length, (n) => `${needs(n)} a look`],
    [t.failed.length, () => "failed"],
    [t.unfinished.length, (n) => `${wasnt(n)} finished`],
  ];
  const parts = counts
    .filter(([n]) => n > 0)
    .map(([n, verb], i) => (i === 0 ? `${episodes(n)} ${verb(n)}` : `${n} ${verb(n)}`));
  const body = parts.length > 0 ? `${parts.join(", ")}.` : "Nothing was translated.";
  if (!stop) return body;
  if (stop.by_user) return `Stopped. ${body}`;
  return `${sentence(stop.message) || "The run stopped."} ${body}`;
}

/** Capitalised, with a full stop: "another operation is already running" →
 *  "Another operation is already running." */
export function sentence(message: string): string {
  const s = message.trim();
  if (!s) return s;
  const capped = s[0].toUpperCase() + s.slice(1);
  return /[.!?]$/.test(capped) ? capped : `${capped}.`;
}
