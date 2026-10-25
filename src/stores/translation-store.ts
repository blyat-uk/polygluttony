import { create } from "zustand"
import type { ErrorKind } from "@/types/generated/ErrorKind"
import type { FileResult } from "@/types/generated/FileResult"
import type { FileStateKind } from "@/types/generated/FileStateKind"
import type { LogLevel } from "@/types/generated/LogLevel"
import type { LogPhase } from "@/types/generated/LogPhase"
import type { RunEvent } from "@/types/generated/RunEvent"
import type { StopInfo } from "@/types/generated/StopInfo"
import type { VerifyIssue } from "@/types/generated/VerifyIssue"
import { onRunFinished } from "@/features/episodes/run-end"

const MAX_LOG_LINES = 500

export interface FileRow {
  state: FileStateKind
  detail: string | null
  translated: number
  /** Highest `translated` seen. A full retranslation restarts `translated` from
   *  zero; the run's progress bar reads this so it never moves backwards. */
  peakTranslated: number
  total: number
  batch: number
  totalBatches: number
  retries: number
  /** Why the last retry happened, in plain words; null before any retry. */
  retryReason: string | null
  hasWarnings: boolean | null
  error: string | null
  /** What kind of failure `error` was. */
  errorKind: ErrorKind | null
  issues: VerifyIssue[]
  /** True once this file entered the verifying phase (drives ✗ placement). */
  reachedVerify: boolean
}

export interface LogLine {
  at: string
  file: string | null
  level: LogLevel
  phase: LogPhase
  message: string
}

/** A provider asked us to slow down: requests wait until `until` (epoch ms). */
export interface Throttle {
  provider: string
  until: number
}

/** HH:MM:SS receive-time stamp for log lines (close enough to emit time). */
const now = () => new Date().toLocaleTimeString("en-GB", { hour12: false })

interface TranslationRunState {
  running: boolean
  /** The folder the current or last run translated; file names are only
   *  unique within it. */
  folder: string | null
  files: Record<string, FileRow>
  logs: LogLine[]
  results: FileResult[] | null
  /** Why the run ended early (from `stopped`); null while running or when it
   *  ran to the end. */
  stopInfo: StopInfo | null
  /** The user pressed Stop on this run. */
  stopRequested: boolean
  /** The latest slow-down request, until it runs out. */
  throttle: Throttle | null
  /** `start_translation` itself was refused (nothing ran); the reason. */
  startError: string | null
  /** What the last start asked for, so a refused start can be tried again. */
  lastStart: { files: string[]; resume: boolean } | null
  /** Started somewhere else (an episode window's review, a glossary term's
   *  retranslation) and picked up from `run_started`: no end-of-run banner or
   *  notification for it. */
  external: boolean
  /** The last full run's outcome (not a review started elsewhere), for the
   *  status bar's result chip. Kept until the next full run ends. */
  lastFullRun: { folder: string; files: Record<string, FileRow>; results: FileResult[] } | null
  start: (files: string[], folder: string, resume?: boolean) => void
  /** The user pressed Stop; the run winds down and sends `run_finished`. */
  requestStop: () => void
  /** `start_translation` rejected: back to idle, with the reason kept for the banner. */
  failStart: (message: string) => void
  /** Forget how the last run ended (its banner and rows), when something newer
   *  takes its place: a glossary step before the next run. The status bar's
   *  result chip stays. */
  clearOutcome: () => void
  applyEvent: (e: RunEvent) => void
  reset: () => void
}

const emptyRow = (): FileRow => ({
  state: "pending",
  detail: null,
  translated: 0,
  peakTranslated: 0,
  total: 0,
  batch: 0,
  totalBatches: 0,
  retries: 0,
  retryReason: null,
  hasWarnings: null,
  error: null,
  errorKind: null,
  issues: [],
  reachedVerify: false,
})

/** An engine that predates `stopped` fails each file it cuts short with this. */
const CANCELLED = /cancel/i

const logLine = (s: { logs: LogLine[] }, line: Omit<LogLine, "at">) => [
  ...s.logs.slice(-(MAX_LOG_LINES - 1)),
  { at: now(), ...line },
]

export const useTranslationRun = create<TranslationRunState>((set, get) => ({
  running: false,
  folder: null,
  files: {},
  logs: [],
  results: null,
  stopInfo: null,
  stopRequested: false,
  throttle: null,
  startError: null,
  lastStart: null,
  external: false,
  lastFullRun: null,

  start: (files, folder, resume = false) =>
    set({
      external: false,
      running: true,
      folder,
      results: null,
      logs: [],
      stopInfo: null,
      stopRequested: false,
      throttle: null,
      startError: null,
      lastStart: { files, resume },
      files: Object.fromEntries(files.map((f) => [f, emptyRow()])),
    }),

  requestStop: () => set({ stopRequested: true }),

  failStart: (message) =>
    set((s) => ({
      running: false,
      files: {},
      results: null,
      throttle: null,
      startError: message,
      logs: logLine(s, { file: null, level: "error", phase: "error", message }),
    })),

  clearOutcome: () =>
    set((s) =>
      s.running ? {} : { files: {}, results: null, stopInfo: null, startError: null, logs: [] },
    ),

  applyEvent: (e) => {
    // Every run opens with `run_started`. One this window didn't start (an
    // episode window retranslating a few lines) is picked up here, so the
    // Episodes list and the status bar show it too.
    if (e.kind === "run_started") {
      if (!get().running) {
        set({
          external: true,
          running: true,
          folder: e.folder,
          results: null,
          logs: [],
          stopInfo: null,
          stopRequested: false,
          throttle: null,
          startError: null,
          lastStart: null,
          files: Object.fromEntries(e.files.map((f) => [f, emptyRow()])),
        })
      }
      return
    }
    if (!get().running) return
    set((s) => {
      const files = { ...s.files }
      const touch = (name: string, patch: Partial<FileRow>) => {
        files[name] = { ...(files[name] ?? emptyRow()), ...patch }
      }
      switch (e.kind) {
        case "state": {
          // A file cut short by the user's Stop stays "Not finished", even when
          // an older engine follows its cancel error with a Failed state.
          const keepStopped =
            e.state === "failed" && s.stopRequested && files[e.file]?.state === "stopped"
          touch(e.file, {
            state: keepStopped ? "stopped" : e.state,
            detail: e.detail,
            ...(e.state === "verifying" ? { reachedVerify: true } : {}),
          })
          return { files }
        }
        case "progress": {
          const prev = files[e.file]
          touch(e.file, {
            translated: e.translated,
            peakTranslated: Math.max(prev?.peakTranslated ?? 0, e.translated),
            total: e.total,
            batch: e.batch,
            totalBatches: e.total_batches,
            retries: e.retries,
            retryReason: e.retries > 0 ? (e.retry_reason ?? prev?.retryReason ?? null) : null,
          })
          return { files }
        }
        case "log":
          return {
            logs: logLine(s, { file: e.file, level: e.level, phase: e.phase, message: e.message }),
          }
        case "file_done":
          touch(e.file, { hasWarnings: e.has_warnings, issues: e.issues })
          return { files }
        case "error":
          if (s.stopRequested && CANCELLED.test(e.message)) {
            touch(e.file, { state: "stopped", error: null, errorKind: null })
          } else {
            touch(e.file, { error: e.message, errorKind: e.error_kind, state: "failed" })
          }
          return { files }
        case "stopped":
          return { stopInfo: e.info }
        case "throttled":
          return {
            throttle: { provider: e.provider, until: Date.now() + e.seconds * 1000 },
          }
        case "run_finished":
          return {
            files,
            results: e.results,
            ...(s.external || !s.folder
              ? {}
              : { lastFullRun: { folder: s.folder, files, results: e.results } }),
            running: false,
            throttle: null,
            // An engine that predates `stopped` still ends a stopped run: say so.
            stopInfo:
              s.stopInfo ??
              (s.stopRequested ? { by_user: true, message: "", kind: null, step: null } : null),
          }
        default:
          return {}
      }
    })
    if (e.kind === "run_finished") {
      const s = get()
      if (s.folder) onRunFinished(s.folder, s.files, e.results, s.stopInfo, s.external)
    }
  },

  reset: () =>
    set({
      running: false,
      folder: null,
      files: {},
      logs: [],
      results: null,
      stopInfo: null,
      stopRequested: false,
      throttle: null,
      startError: null,
      lastStart: null,
      external: false,
      lastFullRun: null,
    }),
}))
