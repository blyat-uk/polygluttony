import { useEffect, type ReactNode } from "react"
import { useNavigate } from "@tanstack/react-router"
import { getCurrentWebview } from "@tauri-apps/api/webview"
import { getCurrentWindow } from "@tauri-apps/api/window"
import { NavRail } from "@/components/nav-rail"
import { StatusBar } from "@/components/status-bar"
import { AtmosphereBackdrop } from "@/components/atmosphere-backdrop"
import { NAVIGATE_EVENT } from "@/components/ui/error-toast"
import { usePickFolder } from "@/features/welcome/use-pick-folder"
import { runPrimaryAction } from "@/hooks/use-primary-action"
import { toast } from "sonner"
import {
  GLOSSARY_CHANGED_EVENT,
  GLOSSARY_FOCUS_TERM_EVENT,
  RESULTS_CHANGED_EVENT,
  ipc,
  onBackendEvent,
  type GlossaryChanged,
  type GlossaryFocusTerm,
} from "@/lib/ipc"
import { folderTitle } from "@/lib/folder-title"
import { queryClient } from "@/lib/query-client"
import { folderResultsKey } from "@/features/episodes/queries"
import { glossaryKey } from "@/features/glossary/keys"
import { projectKey } from "@/features/project/use-project"
import { useAppStore } from "@/stores/app-store"
import { useTranslationRun } from "@/stores/translation-store"
import { useGlossaryRun } from "@/stores/glossary-store"
import type { RunEvent } from "@/types/generated/RunEvent"
import type { GlossaryEvent } from "@/types/generated/GlossaryEvent"
import type { ResultsChanged } from "@/types/generated/ResultsChanged"

/** Ctrl/Cmd + 1…5: the rail's pages, top to bottom. */
export const PAGE_SHORTCUTS = [
  { key: "1", to: "/episodes", label: "Episodes" },
  { key: "2", to: "/glossary", label: "Glossary" },
  { key: "3", to: "/connections", label: "Models" },
  { key: "4", to: "/prompts", label: "Prompts" },
  { key: "5", to: "/help", label: "Help" },
] as const

/** What an app-wide key press asks for, if anything. */
export type Shortcut = { kind: "page"; to: string } | { kind: "open" } | { kind: "primary" }

/** Is focus somewhere the user types? Those keys belong to the field. */
function isTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false
  if (target.isContentEditable) return true
  if (target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) return true
  if (target instanceof HTMLInputElement) {
    return !["checkbox", "radio", "button", "submit", "reset", "range", "color", "file"].includes(
      target.type,
    )
  }
  return false
}

/**
 * Pure: the shortcut a key press means. Ctrl/Cmd+Enter runs the page's main action even
 * from a text field (that's where one presses it); the others never fire while typing, and
 * nothing here touches Ctrl+Z / C / V / A.
 */
export function shortcutFor(
  e: Pick<
    KeyboardEvent,
    "key" | "code" | "ctrlKey" | "metaKey" | "shiftKey" | "altKey" | "repeat" | "isComposing"
  >,
  typing: boolean,
): Shortcut | null {
  if (!(e.ctrlKey || e.metaKey) || e.altKey || e.shiftKey || e.isComposing) return null
  if (e.key === "Enter") return e.repeat ? null : { kind: "primary" }
  if (typing || e.repeat) return null
  if (e.key === "o" || e.key === "O") return { kind: "open" }
  // By key, or by the physical digit row (AZERTY's Ctrl+1 is "&").
  const page = PAGE_SHORTCUTS.find((p) => p.key === e.key || e.code === `Digit${p.key}`)
  return page ? { kind: "page", to: page.to } : null
}

/** A dropped subtitle file stands for the folder it's in. */
export function folderOf(path: string): string {
  return /\.(ass|ssa)$/i.test(path) ? path.replace(/[/\\][^/\\]+$/, "") || path : path
}

/** A dialog is up: its own keys win, and the page behind it shouldn't change. */
function dialogOpen(): boolean {
  return !!document.querySelector('[role="dialog"][data-state="open"], [role="alertdialog"][data-state="open"]')
}

export function AppLayout({ children }: { children: ReactNode }) {
  const navigate = useNavigate()
  const folder = usePickFolder()
  const { pick, openPath } = folder
  const applyEvent = useTranslationRun((s) => s.applyEvent)
  const applyGlossaryEvent = useGlossaryRun((s) => s.applyEvent)
  const running = useTranslationRun((s) => s.running)
  const gBusy = useGlossaryRun((s) => s.busy)
  const intensity = running || gBusy ? 1 : 0

  useEffect(() => {
    const un = onBackendEvent<RunEvent>("translation://event", (e) => applyEvent(e.payload))
    return () => {
      un.then((f) => f())
    }
  }, [applyEvent])

  // A review edit in an episode window (or a run) stored a result: the
  // Episodes list refetches that folder's results, and the folder view (which
  // output files exist), so Mark done shows Done at once.
  useEffect(() => {
    const un = onBackendEvent<ResultsChanged>(RESULTS_CHANGED_EVENT, (e) => {
      void queryClient.invalidateQueries({ queryKey: folderResultsKey(e.payload.folder) })
      void queryClient.invalidateQueries({ queryKey: projectKey(e.payload.folder) })
    })
    return () => {
      un.then((f) => f())
    }
  }, [])

  useEffect(() => {
    const un = onBackendEvent<GlossaryEvent>("glossary://event", (e) =>
      applyGlossaryEvent(e.payload),
    )
    return () => {
      un.then((f) => f())
    }
  }, [applyGlossaryEvent])

  // An episode window's "Add to glossary" saved the glossary: reload it, even
  // right after a save of our own, so a later edit here can't save over the new term.
  useEffect(() => {
    const un = onBackendEvent<GlossaryChanged>(GLOSSARY_CHANGED_EVENT, (e) => {
      void queryClient.invalidateQueries({ queryKey: glossaryKey(e.payload.folder) })
    })
    return () => {
      un.then((f) => f())
    }
  }, [])

  // "Fix in glossary" in an episode window: show that term on the Glossary page, up front.
  // A term from another folder than the open one only gets a note.
  useEffect(() => {
    const un = onBackendEvent<GlossaryFocusTerm>(GLOSSARY_FOCUS_TERM_EVENT, (e) => {
      const { folder: termFolder, source } = e.payload
      const main = getCurrentWindow()
      void main
        .unminimize()
        .then(() => main.setFocus())
        .catch(() => undefined)
      if (useAppStore.getState().workdir !== termFolder) {
        useGlossaryRun.getState().clearFocusTerm()
        toast.info(`That term is in ${folderTitle(termFolder)}. Open that folder to see it.`)
        return
      }
      // The term may have just been added in that window: reload the glossary
      // before showing it, so the page finds the term.
      void queryClient
        .fetchQuery({
          queryKey: glossaryKey(termFolder),
          queryFn: () => ipc.loadGlossary(termFolder),
          staleTime: 0,
        })
        .catch(() => undefined)
        .then(() => {
          if (useAppStore.getState().workdir !== termFolder) return
          useGlossaryRun.getState().requestFocusTerm(termFolder, source)
          void navigate({ to: "/glossary" })
        })
    })
    return () => {
      un.then((f) => f())
    }
  }, [navigate])

  // An error toast's "Open Models" (and the like): go where the fix is.
  useEffect(() => {
    const onGo = (e: Event) => {
      const to = (e as CustomEvent<string>).detail
      if (typeof to === "string") void navigate({ to: to as never })
    }
    window.addEventListener(NAVIGATE_EVENT, onGo)
    return () => window.removeEventListener(NAVIGATE_EVENT, onGo)
  }, [navigate])

  // A folder dropped anywhere on the window opens it.
  useEffect(() => {
    const unlisten = getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === "drop" && event.payload.paths.length > 0) {
        openPath(folderOf(event.payload.paths[0]))
      }
    })
    return () => {
      void unlisten.then((u) => u())
    }
  }, [openPath])

  // Ctrl/Cmd+Enter runs the page's main action. In the capture phase, so a
  // focused control that handles Enter itself (a checkbox) can't swallow it;
  // when an action ran, nothing else acts on the same key press.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (dialogOpen()) return
      if (shortcutFor(e, isTyping(e.target))?.kind !== "primary") return
      if (runPrimaryAction()) {
        e.preventDefault()
        e.stopPropagation()
      }
    }
    window.addEventListener("keydown", onKey, true)
    return () => window.removeEventListener("keydown", onKey, true)
  }, [])

  // The other app-wide shortcuts.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || dialogOpen()) return
      const shortcut = shortcutFor(e, isTyping(e.target))
      if (!shortcut || shortcut.kind === "primary") return
      e.preventDefault()
      if (shortcut.kind === "open") void pick()
      else void navigate({ to: shortcut.to as never })
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [navigate, pick])

  return (
    <>
      <AtmosphereBackdrop intensity={intensity} />
      <div className="relative z-10 grid h-screen grid-cols-[auto_1fr] grid-rows-[1fr_auto] bg-transparent text-foreground">
        <div className="row-span-2">
          <NavRail />
        </div>
        <main className="min-h-0 overflow-auto">{children}</main>
        <StatusBar />
      </div>
    </>
  )
}
