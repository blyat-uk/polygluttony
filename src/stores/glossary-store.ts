import { create } from "zustand"
import { toast } from "sonner"
import type { BuildMode } from "@/types/generated/BuildMode"
import type { GlossaryBuildSummary } from "@/types/generated/GlossaryBuildSummary"
import type { GlossaryDiff } from "@/types/generated/GlossaryDiff"
import type { GlossaryEvent } from "@/types/generated/GlossaryEvent"
import type { GlossaryPhase } from "@/types/generated/GlossaryPhase"
import type { LogLevel } from "@/types/generated/LogLevel"
import type { ReferenceSummary } from "@/types/generated/ReferenceSummary"
import type { ProjectView } from "@/types/generated/ProjectView"
import { useAppStore } from "@/stores/app-store"
import { queryClient } from "@/lib/query-client"
import { projectKey } from "@/features/project/use-project"
import { glossaryLoadKey } from "@/features/episodes/queries"

/** HH:MM:SS receive-time stamp for log lines (close enough to emit time). */
const now = () => new Date().toLocaleTimeString("en-GB", { hour12: false })

const MAX_LOG_LINES = 500

/** `normalize` is Tidy up; `lookup` is Look up names online on the glossary as it is. */
export type GlossaryOp = "build" | "normalize" | "lookup" | "import"

export interface GlossaryLogLine {
  at: string
  level: LogLevel
  message: string
}

interface GlossaryRunStore {
  busy: GlossaryOp | null
  /** The folder this run (or its results) belongs to; null = never ran. */
  folder: string | null
  phase: GlossaryPhase | null
  phaseDetail: string | null
  done: number
  total: number
  /** Live extraction terms streaming in per batch (category key → terms). */
  glossTerms: Record<string, { source: string; target: string }[]>
  glossTermCount: number
  /** Episodes a build has been through every batch of, in the order they finished. */
  filesRead: string[]
  logs: GlossaryLogLine[]
  summary: GlossaryBuildSummary | null
  lastDiff: GlossaryDiff | null
  error: string | null
  /** Bumped on Done / FileChanged — the page refetches the glossary query. */
  fileTick: number
  /** Free-text shown in the import run description ("40 translated files"). */
  opDetail: string | null
  /** Mode of the running build; null when the op isn't a build. Drives the
   *  progress screen's title and its cancel note — cancelling a re-generate
   *  does NOT restore the previous glossary, cancelling an append is harmless. */
  buildMode: BuildMode | null
  /** ③ Reference review screen visibility + the import that opened it. */
  reviewOpen: boolean
  /** Update screen (Append / Re-generate) visibility, folder-tagged like reviewOpen. */
  updateOpen: boolean
  lastImport: ReferenceSummary | null
  startOp: (op: GlossaryOp, folder: string, detail?: string) => void
  /** Set right after `startOp("build", …)` — `startOp` clears it, so a stale
   *  mode can never leak from one run into the next. */
  setBuildMode: (mode: BuildMode) => void
  endOp: () => void
  setLastDiff: (d: GlossaryDiff) => void
  applyEvent: (e: GlossaryEvent) => void
  openReview: (folder: string, lastImport?: ReferenceSummary) => void
  closeReview: () => void
  openUpdate: (folder: string) => void
  closeUpdate: () => void
  /** Cross-page request: show this term in the glossary (search + highlight). Set
   *  from "Fix in glossary" in an episode window, consumed by the Glossary page. */
  pendingFocusTerm: { folder: string; source: string } | null
  requestFocusTerm: (folder: string, source: string) => void
  clearFocusTerm: () => void
  reset: () => void
}

export const useGlossaryRun = create<GlossaryRunStore>((set) => ({
  pendingFocusTerm: null,
  requestFocusTerm: (folder, source) => set({ pendingFocusTerm: { folder, source } }),
  clearFocusTerm: () => set({ pendingFocusTerm: null }),
  busy: null,
  folder: null,
  phase: null,
  phaseDetail: null,
  done: 0,
  total: 0,
  glossTerms: {},
  glossTermCount: 0,
  filesRead: [],
  logs: [],
  summary: null,
  lastDiff: null,
  error: null,
  fileTick: 0,
  opDetail: null,
  buildMode: null,
  reviewOpen: false,
  updateOpen: false,
  lastImport: null,

  // busy is set optimistically before the invoke; a rejected invoke must call
  // endOp() or the page soft-locks (step-3 lesson).
  startOp: (op, folder, detail) =>
    set((s) => ({
      busy: op,
      folder,
      opDetail: detail ?? null,
      buildMode: null,
      phase: null,
      phaseDetail: null,
      done: 0,
      total: 0,
      glossTerms: {},
      glossTermCount: 0,
      filesRead: [],
      logs: [],
      error: null,
      summary: op === "build" ? null : s.summary,
    })),
  setBuildMode: (buildMode) => set({ buildMode }),
  endOp: () => set({ busy: null }),
  setLastDiff: (lastDiff) => set({ lastDiff }),
  openReview: (folder, lastImport) =>
    set((s) => ({
      reviewOpen: true,
      // The review belongs to a folder; tagging it lets the folder-change
      // reset close a review opened before any run this session.
      folder: s.folder ?? folder,
      lastImport: lastImport ?? s.lastImport,
    })),
  closeReview: () => set({ reviewOpen: false, lastImport: null }),
  openUpdate: (folder) =>
    set((s) => ({
      updateOpen: true,
      // Same folder-tagging as openReview: lets the folder-change reset close
      // an update screen opened before any run this session.
      folder: s.folder ?? folder,
    })),
  closeUpdate: () => set({ updateOpen: false }),

  applyEvent: (e) =>
    set((s) => {
      switch (e.kind) {
        case "phase":
          return { phase: e.phase, phaseDetail: e.detail }
        case "progress":
          // Completion-order emission can deliver counts out of order → clamp
          // with max(). BUT a `done: 0` (or a total change) starts a NEW
          // sequence (reference phase → extraction phase) — accept it as a
          // reset instead of clamping it away.
          return {
            done:
              e.done === 0 || e.total !== s.total ? e.done : Math.max(s.done, e.done),
            total: e.total,
          }
        case "terms": {
          // Stream each batch's newly-found terms into their category lanes.
          const next: Record<string, { source: string; target: string }[]> = { ...s.glossTerms }
          for (const h of e.hits) {
            next[h.category] = [...(next[h.category] ?? []), { source: h.source, target: h.target }]
          }
          return { glossTerms: next, glossTermCount: s.glossTermCount + e.hits.length }
        }
        case "file_read":
          return { filesRead: [...s.filesRead, e.file] }
        case "log":
          return {
            logs: [
              ...s.logs.slice(-(MAX_LOG_LINES - 1)),
              { at: now(), level: e.level, message: e.message },
            ],
          }
        case "done": {
          // Keep the rail badge live (cross-store side effect, deliberate).
          useAppStore.getState().setGlossaryTerms(e.summary.terms_final)
          // Patch the cached ProjectView (staleTime: Infinity) so Translate's
          // "(N terms)" estimate is fresh even when no glossary view is mounted.
          // Patch, not invalidate — open_folder is a full re-analyze.
          if (s.folder) {
            queryClient.setQueryData<ProjectView>(projectKey(s.folder), (v) =>
              v ? { ...v, glossary_terms: e.summary.terms_final || null } : v,
            )
            // The Episodes page's glossary row and tick read the new file.
            void queryClient.invalidateQueries({ queryKey: glossaryLoadKey(s.folder) })
          }
          return {
            busy: null,
            summary: e.summary,
            lastDiff: e.summary.diff.has_changes ? e.summary.diff : s.lastDiff,
            fileTick: s.fileTick + 1,
          }
        }
        case "error":
          toast.error(e.message)
          return {
            busy: null,
            error: e.message,
            logs: [
              ...s.logs.slice(-(MAX_LOG_LINES - 1)),
              { at: now(), level: "error" as LogLevel, message: e.message },
            ],
          }
        case "file_changed":
          if (s.folder) void queryClient.invalidateQueries({ queryKey: glossaryLoadKey(s.folder) })
          return { fileTick: s.fileTick + 1 }
        default:
          return {}
      }
    }),

  reset: () =>
    set({
      busy: null, folder: null, phase: null, phaseDetail: null, done: 0, total: 0,
      glossTerms: {}, glossTermCount: 0, filesRead: [],
      logs: [], summary: null, lastDiff: null, error: null,
      opDetail: null, buildMode: null, reviewOpen: false, updateOpen: false, lastImport: null,
    }),
}))
