import type { EpisodeLine } from "@/types/generated/EpisodeLine";
import type { EpisodeSnapshot } from "@/types/generated/EpisodeSnapshot";
import type { VerifyIssue } from "@/types/generated/VerifyIssue";

// Pure helpers for the episode window: no React, no IPC, so they can be
// sanity-checked with a one-off script.

// ── text ─────────────────────────────────────────────────────────────────────

/** Split display text on ASS breaks (`\N`, `\n`) and real newlines. `\h` (ASS
 *  hard space) becomes a no-break space so it doesn't show as a literal. */
export function splitBreaks(text: string): string[] {
  return text.replace(/\\h/g, " ").split(/\\[Nn]|\r?\n/);
}

/** Centiseconds → `H:MM:SS`, or `H:MM:SS.cc` with `centis`. */
export function formatCs(cs: number, centis = false): string {
  const total = Math.max(0, Math.floor(cs));
  const h = Math.floor(total / 360_000);
  const m = Math.floor(total / 6_000) % 60;
  const s = Math.floor(total / 100) % 60;
  const base = `${h}:${pad2(m)}:${pad2(s)}`;
  return centis ? `${base}.${pad2(total % 100)}` : base;
}

/** Elapsed milliseconds → `34s`, `2m 05s`, `1h 03m`. */
export function formatElapsed(ms: number): string {
  const secs = Math.max(0, Math.floor(ms / 1000));
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ${pad2(secs % 60)}s`;
  return `${Math.floor(mins / 60)}h ${pad2(mins % 60)}m`;
}

function pad2(n: number): string {
  return n < 10 ? `0${n}` : String(n);
}

// ── ids ──────────────────────────────────────────────────────────────────────

/** `[261, 262, …, 388, 400]` → `261–388, 400`. Past `maxParts` runs the tail
 *  collapses to `and N more` (N = remaining ids), so one header line holds it. */
export function compressRanges(ids: readonly number[], maxParts = 6): string {
  const sorted = [...ids].sort((a, b) => a - b);
  const runs: Array<[number, number]> = [];
  for (const id of sorted) {
    const last = runs[runs.length - 1];
    if (last && id <= last[1] + 1) last[1] = Math.max(last[1], id);
    else runs.push([id, id]);
  }
  const label = ([a, b]: [number, number]) => (a === b ? `${a}` : `${a}–${b}`);
  if (runs.length <= maxParts) return runs.map(label).join(", ");
  const shown = runs.slice(0, maxParts - 1);
  const rest = runs.slice(maxParts - 1).reduce((n, [a, b]) => n + (b - a + 1), 0);
  return `${shown.map(label).join(", ")} and ${rest} more`;
}

/** Whether a line is out with the model right now. */
export function isLive(line: EpisodeLine): boolean {
  return line.status === "sending" || line.status === "resending";
}

/** Indices (into `lines`) of the lines being sent or re-sent, ascending. */
export function liveIndices(lines: readonly EpisodeLine[]): number[] {
  const out: number[] = [];
  lines.forEach((l, i) => {
    if (isLive(l)) out.push(i);
  });
  return out;
}

/** Verify issues grouped by line; file-level issues (`line_id` 0) are left out. */
export function issuesByLine(issues: readonly VerifyIssue[]): Map<number, VerifyIssue[]> {
  const out = new Map<number, VerifyIssue[]>();
  for (const it of issues) {
    if (it.line_id <= 0) continue;
    const list = out.get(it.line_id);
    if (list) list.push(it);
    else out.set(it.line_id, [it]);
  }
  return out;
}

// ── header activity line ─────────────────────────────────────────────────────

export type ActivityTone = "live" | "idle" | "done" | "warn" | "failed";

/** One line under the file name. `since` (epoch ms) asks the header to append a
 *  ticking ` · 34s`; `until` (epoch ms) a countdown ` · 18s` instead. */
export type ActivityLine = {
  text: string;
  tone: ActivityTone;
  since: number | null;
  until?: number | null;
};

const BUSY_LABEL: Partial<Record<EpisodeSnapshot["state"], string>> = {
  translating: "Translating",
  retranslating: "Retranslating",
  cleanup: "Cleaning up",
  verifying: "Verifying",
};

export function describeActivity(
  s: EpisodeSnapshot,
  liveIds: readonly number[],
  flaggedLines: number,
): ActivityLine {
  if (s.state === "failed") {
    const why = s.error ?? s.detail;
    return { text: why ? `Failed: ${why}` : "Failed", tone: "failed", since: null };
  }
  if (s.state === "stopped") {
    return {
      text: s.error ? `Not finished · ${s.error}` : "Not finished · the run stopped",
      tone: "idle",
      since: null,
    };
  }
  if (s.state === "done" || s.state === "warning") {
    if (flaggedLines > 0) {
      return {
        text: `Done · ${flaggedLines} ${flaggedLines === 1 ? "line needs" : "lines need"} a look`,
        tone: "warn",
        since: null,
      };
    }
    // Every flagged line is settled; the file keeps its .warning name until
    // it's marked done.
    return s.state === "warning"
      ? { text: "Done · nothing left to look at", tone: "done", since: null }
      : { text: "Done", tone: "done", since: null };
  }

  const a = s.activity;
  if (a && s.throttle) {
    // A wait the provider asked for: never model time.
    return {
      text: `Waiting — ${s.throttle.provider} asked us to slow down`,
      tone: "idle",
      since: null,
      until: s.throttle.until_ms,
    };
  }
  if (a && s.activity_since_ms === null) {
    // Announced, not out yet: waiting for a free slot or a retry.
    return { text: `${withLines("Waiting to send", liveIds)}${part(a)}`, tone: "idle", since: null };
  }
  if (a) {
    const since = s.activity_since_ms;
    switch (a.kind) {
      case "batch":
        return {
          text: `${withLines("Sending", liveIds)} · batch ${a.batch} of ${a.total_batches}`,
          tone: "live",
          since,
        };
      case "cleanup":
        return {
          text: liveIds.length > 0 ? `Cleaning up ${countLines(liveIds.length)}` : "Cleaning up",
          tone: "live",
          since,
        };
      case "retranslate":
        return { text: `${withLines("Retranslating", liveIds)}${part(a)}`, tone: "live", since };
    }
  }

  if (s.state === "pending") {
    return s.run_active
      ? { text: "Waiting to start", tone: "idle", since: null }
      : { text: "The run ended before this episode started", tone: "idle", since: null };
  }
  // Busy state but the run is over (cancelled between events): say so rather
  // than claim work that isn't happening.
  if (!s.run_active) {
    return { text: "The run ended before this episode finished", tone: "idle", since: null };
  }
  const label = BUSY_LABEL[s.state] ?? "Working";
  return { text: s.detail ? `${label} · ${s.detail}` : label, tone: "live", since: null };
}

/** ` · batch 2 of 3`, ` · attempt 1 of 3`; nothing for a single retranslation
 *  (a reviewer's own request) or the cleanup pass. */
function part(a: NonNullable<EpisodeSnapshot["activity"]>): string {
  switch (a.kind) {
    case "batch":
      return ` · batch ${a.batch} of ${a.total_batches}`;
    case "retranslate":
      return a.max_attempts > 1 ? ` · attempt ${a.attempt} of ${a.max_attempts}` : "";
    case "cleanup":
      return "";
  }
}

/** Seconds left until `until` (epoch ms), never below zero. */
export function secondsLeft(until: number, now: number): number {
  return Math.max(0, Math.ceil((until - now) / 1000));
}

/** `Sending lines 261–388`, `Sending line 7`, or just `Sending` with no ids. */
function withLines(verb: string, ids: readonly number[]): string {
  if (ids.length === 0) return verb;
  return `${verb} ${ids.length === 1 ? "line" : "lines"} ${compressRanges(ids)}`;
}

function countLines(n: number): string {
  return `${n.toLocaleString()} ${n === 1 ? "line" : "lines"}`;
}

// ── skeleton ─────────────────────────────────────────────────────────────────

/** Rough length of the translation-to-be in Latin characters: an East Asian
 *  wide character usually becomes about three. */
function lengthUnits(s: string): number {
  let n = 0;
  for (const ch of s) {
    const c = ch.codePointAt(0) ?? 0;
    const wide =
      (c >= 0x1100 && c <= 0x115f) ||
      (c >= 0x2e80 && c <= 0xa4cf) ||
      (c >= 0xac00 && c <= 0xd7a3) ||
      (c >= 0xf900 && c <= 0xfaff) ||
      (c >= 0xfe30 && c <= 0xfe4f) ||
      (c >= 0xff00 && c <= 0xff60) ||
      (c >= 0xffe0 && c <= 0xffe6) ||
      c >= 0x20000;
    n += wide ? 3 : 1;
  }
  return n;
}

/** Stable pseudo-random fraction in [0, 1) from two integers (murmur3 finaliser). */
function hash01(a: number, b: number): number {
  let h = Math.imul(a ^ 0x9e3779b9, 0x85ebca6b) ^ Math.imul(b + 0x632be5ab, 0xc2b2ae35);
  h ^= h >>> 16;
  h = Math.imul(h, 0x85ebca6b);
  h ^= h >>> 13;
  h = Math.imul(h, 0xc2b2ae35);
  h ^= h >>> 16;
  return (h >>> 0) / 0x1_0000_0000;
}

/** Latin characters that roughly fill one line of the Translation column at the
 *  window's default width. */
const UNITS_PER_BAR = 64;
const MAX_BARS = 4;

/**
 * Skeleton bar widths (percent of the cell) for a line being sent: one bar per
 * wrapped line the translation is likely to take, following the original's
 * breaks and length. Seeded by the line id, so a row never jitters between
 * renders or remounts.
 */
export function skeletonBars(id: number, source: string): number[] {
  const bars: number[] = [];
  const segments = splitBreaks(source).map((seg) => seg.trim()).filter(Boolean);
  for (const seg of segments.length > 0 ? segments : [""]) {
    const fill = Math.max(lengthUnits(seg), 8) / UNITS_PER_BAR;
    const full = Math.floor(fill);
    for (let i = 0; i < full; i++) {
      bars.push(Math.round(90 + 10 * hash01(id, bars.length)));
    }
    const rest = fill - full;
    if (rest > 0.12 || full === 0) {
      const jitter = (hash01(id, bars.length) - 0.5) * 16;
      bars.push(Math.round(Math.min(96, Math.max(16, rest * 100 + jitter))));
    }
  }
  return bars.slice(0, MAX_BARS);
}

/** First-guess row height (px) before a row is measured: one 18px text line per
 *  original break plus the row's vertical padding and border. */
export function estimateRowHeight(line: EpisodeLine | undefined): number {
  if (!line) return 33;
  return 15 + 18 * Math.max(1, splitBreaks(line.source).length);
}
