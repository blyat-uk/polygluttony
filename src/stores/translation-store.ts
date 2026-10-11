import { create } from "zustand"
import type { FileResult } from "@/types/generated/FileResult"
import type { FileStateKind } from "@/types/generated/FileStateKind"
import type { LogLevel } from "@/types/generated/LogLevel"
import type { LogPhase } from "@/types/generated/LogPhase"
import type { RunEvent } from "@/types/generated/RunEvent"
import type { VerifyIssue } from "@/types/generated/VerifyIssue"

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
  hasWarnings: boolean | null
  error: string | null
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

/** HH:MM:SS receive-time stamp for log lines (close enough to emit time). */
const now = () => new Date().toLocaleTimeString("en-GB", { hour12: false })

interface TranslationRunState {
  running: boolean
  files: Record<string, FileRow>
  logs: LogLine[]
  results: FileResult[] | null
  start: (files: string[]) => void
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
  hasWarnings: null,
  error: null,
  issues: [],
  reachedVerify: false,
})

export const useTranslationRun = create<TranslationRunState>((set) => ({
  running: false,
  files: {},
  logs: [],
  results: null,

  start: (files) =>
    set({
      running: true,
      results: null,
      logs: [],
      files: Object.fromEntries(files.map((f) => [f, emptyRow()])),
    }),

  applyEvent: (e) =>
    set((s) => {
      const files = { ...s.files }
      const touch = (name: string, patch: Partial<FileRow>) => {
        files[name] = { ...(files[name] ?? emptyRow()), ...patch }
      }
      switch (e.kind) {
        case "state":
          touch(e.file, {
            state: e.state,
            detail: e.detail,
            ...(e.state === "verifying" ? { reachedVerify: true } : {}),
          })
          return { files }
        case "progress":
          touch(e.file, {
            translated: e.translated,
            peakTranslated: Math.max(files[e.file]?.peakTranslated ?? 0, e.translated),
            total: e.total,
            batch: e.batch,
            totalBatches: e.total_batches,
            retries: e.retries,
          })
          return { files }
        case "log":
          return {
            logs: [
              ...s.logs.slice(-(MAX_LOG_LINES - 1)),
              { at: now(), file: e.file, level: e.level, phase: e.phase, message: e.message },
            ],
          }
        case "file_done":
          touch(e.file, { hasWarnings: e.has_warnings, issues: e.issues })
          return { files }
        case "error":
          touch(e.file, { error: e.message, state: "failed" })
          return { files }
        case "run_finished":
          return { files, results: e.results, running: false }
        default:
          return {}
      }
    }),

  reset: () => set({ running: false, files: {}, logs: [], results: null }),
}))
