import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { emit, type UnlistenFn } from "@tauri-apps/api/event";
import { Window } from "@tauri-apps/api/window";
import {
  EPISODE_GO_TO_LINE_EVENT,
  GLOSSARY_FOCUS_TERM_EVENT,
  ipc,
  onBackendEvent,
  type EpisodeGoToLine,
  type GlossaryFocusTerm,
} from "@/lib/ipc";
import type { EpisodeLine } from "@/types/generated/EpisodeLine";
import type { VerifyIssue } from "@/types/generated/VerifyIssue";
import {
  ADD_SHORTCUT,
  AddToGlossary,
  draftFrom,
  SelectionButton,
  useLineSelection,
  type GlossaryDraft,
} from "./add-to-glossary";
import { EpisodeFooter, type Notice, type TermBanner } from "./episode-footer";
import { EpisodeHeader, type Navigator } from "./episode-header";
import { liveIndices } from "./format";
import { LineTable, type GoTo } from "./line-table";
import {
  flaggedIds,
  flaggedPosition,
  flagsByLine,
  plainError,
  reviewBlocked,
  stepFlagged,
} from "./review";
import { ReviewContext, type ReviewActions } from "./review-context";
import { useEpisode } from "./use-episode";

const NO_LINES: EpisodeLine[] = [];
const NO_ISSUES: VerifyIssue[] = [];

/** A quiet message in place of the table. */
function Notice_({ children, tone = "muted" }: { children: ReactNode; tone?: "muted" | "faint" }) {
  return (
    <div className="flex flex-1 items-center justify-center px-8 text-center">
      <div
        className={
          tone === "faint"
            ? "text-[12px] text-muted-foreground/60"
            : "max-w-md text-[12.5px] text-muted-foreground"
        }
      >
        {children}
      </div>
    </div>
  );
}

/** The text selected in the window, when it's short enough to be a name. */
function selectedTerm(): string | null {
  const text = window.getSelection()?.toString().replace(/\s+/g, " ").trim() ?? "";
  return text && text.length <= 60 ? text : null;
}

/** "Fix in glossary": the main window shows the Glossary with this term,
 *  and comes to the front when the platform lets it. */
async function focusTermInMain(folder: string, source: string) {
  await emit(GLOSSARY_FOCUS_TERM_EVENT, { folder, source } satisfies GlossaryFocusTerm);
  try {
    const main = await Window.getByLabel("main");
    if (main) {
      await main.unminimize();
      await main.show();
      await main.setFocus();
    }
  } catch {
    // Focusing another window is a nicety; the event is what matters.
  }
}

/** One episode: its lines as the run produces them, and afterwards a review
 *  of the lines that need a look. */
export function EpisodeWindow({
  file,
  folder,
  initialLine,
}: {
  file: string;
  /** Null when the opener didn't say: live view only, no review. */
  folder: string | null;
  initialLine: number | null;
}) {
  const { view, error, landed } = useEpisode(file, folder);

  useEffect(() => {
    document.title = `${file} — Polygluttony`;
  }, [file]);

  const snapshot = view?.kind === "live" || view?.kind === "stored" ? view : null;
  const lines = snapshot?.lines ?? NO_LINES;
  const flags = useMemo(() => flagsByLine(lines, snapshot?.issues ?? NO_ISSUES), [lines, snapshot?.issues]);
  const flagged = useMemo(() => flaggedIds(flags), [flags]);

  const [onlyFlagged, setOnlyFlagged] = useState(false);
  const shown = useMemo(
    () => (onlyFlagged ? lines.filter((l) => flags.has(l.id)) : lines),
    [onlyFlagged, lines, flags],
  );
  const live = useMemo(() => liveIndices(shown), [shown]);
  const liveIds = useMemo(() => live.map((i) => shown[i].id), [live, shown]);

  // ── selection and going to a line ─────────────────────────────────────────
  const [selected, setSelected] = useState<number | null>(initialLine);
  const [goTo, setGoTo] = useState<GoTo | null>(initialLine ? { id: initialLine, seq: 1 } : null);
  const goToSeq = useRef(1);
  const flagsRef = useRef(flags);
  flagsRef.current = flags;
  const goToLine = useCallback((id: number) => {
    // A line the filter hides comes back into view.
    if (!flagsRef.current.has(id)) setOnlyFlagged(false);
    setSelected(id);
    goToSeq.current += 1;
    setGoTo({ id, seq: goToSeq.current });
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: UnlistenFn | undefined;
    onBackendEvent<EpisodeGoToLine>(EPISODE_GO_TO_LINE_EVENT, (e) => {
      if (e.payload.file === file) goToLine(e.payload.line);
    })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [file, goToLine]);

  const nav: Navigator | null =
    snapshot && folder
      ? {
          flagged: flagged.length,
          position: flaggedPosition(flagged, selected),
          onPrev: () => {
            const id = stepFlagged(flagged, selected, -1);
            if (id !== null) goToLine(id);
          },
          onNext: () => {
            const id = stepFlagged(flagged, selected, 1);
            if (id !== null) goToLine(id);
          },
        }
      : null;

  // ── review actions ─────────────────────────────────────────────────────────
  const blocked =
    snapshot && (view?.kind === "live" || view?.kind === "stored")
      ? reviewBlocked(snapshot, view.kind, folder !== null)
      : null;
  const [busy, setBusy] = useState(false);
  const [editingId, setEditingId] = useState<number | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [banner, setBanner] = useState<TermBanner | null>(null);

  // Whatever was asked for has shown up (or the window moved on).
  useEffect(() => setBusy(false), [view]);
  // A run takes the episode: no editor left open over it.
  useEffect(() => {
    if (blocked !== null) setEditingId(null);
  }, [blocked]);

  const fail = useCallback((e: unknown) => {
    setBusy(false);
    setNotice({ text: plainError(e), tone: "error" });
  }, []);

  const actions = useMemo<ReviewActions | null>(() => {
    if (!folder) return null;
    return {
      enabled: blocked === null,
      editingId,
      busy,
      select: setSelected,
      startEdit: (id) => {
        setSelected(id);
        setEditingId(id);
        setNotice(null);
      },
      cancelEdit: () => setEditingId(null),
      saveEdit: async (id, text) => {
        setBusy(true);
        try {
          await ipc.saveLine(folder, file, id, text);
          setEditingId(null);
          setNotice({ text: `Saved line ${id}.`, tone: "info" });
          return true;
        } catch (e) {
          fail(e);
          return false;
        }
      },
      retranslate: (ids) => {
        setBusy(true);
        setNotice(null);
        ipc.retranslateLines(folder, file, ids).catch(fail);
      },
      looksFine: (id, issues) => {
        setBusy(true);
        setNotice(null);
        void (async () => {
          try {
            for (const it of issues) {
              await ipc.dismissIssue(folder, file, id, it.issue_type, it.term);
            }
          } catch (e) {
            fail(e);
          }
        })();
      },
      fixInGlossary: (term) => {
        const source = term ?? selectedTerm();
        if (!source) {
          setNotice({ text: "Select the name in the original line first.", tone: "info" });
          return;
        }
        focusTermInMain(folder, source).catch(fail);
      },
    };
  }, [folder, file, blocked, editingId, busy, fail]);

  const markDone = () => {
    if (!folder) return;
    setBusy(true);
    setNotice(null);
    ipc
      .markEpisodeDone(folder, file)
      .then(() => setNotice({ text: "Done. The file now has its clean name.", tone: "info" }))
      .catch(fail);
  };

  const retranslateTerm = () => {
    if (!folder || !banner) return;
    setBusy(true);
    ipc
      .retranslateTerm(folder, banner.source)
      .then(() => setBanner(null))
      .catch(fail);
  };

  // ── add to glossary ────────────────────────────────────────────────────────
  const mainRef = useRef<HTMLElement>(null);
  const selection = useLineSelection(mainRef);
  const [draft, setDraft] = useState<GlossaryDraft | null>(null);
  const selectionRef = useRef(selection);
  selectionRef.current = selection;

  const openDraft = useCallback(
    (fallback?: HTMLElement) => {
      if (!folder) return;
      const r = fallback?.getBoundingClientRect();
      const anchor = r
        ? { left: r.left, top: r.top - 4, width: r.width, height: 0 }
        : { left: window.innerWidth / 2 - 160, top: window.innerHeight / 3, width: 0, height: 0 };
      setDraft(draftFrom(selectionRef.current, anchor));
    },
    [folder],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "e") {
        e.preventDefault();
        openDraft();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [openDraft]);

  const onAdded = (source: string, target: string, label: string) => {
    setDraft(null);
    setNotice({ text: `Added ${source} → ${target} to ${label}.`, tone: "info" });
    if (!folder) return;
    ipc
      .findTermLines(folder, source)
      .then((found) => {
        const lines = found.reduce((n, t) => n + t.line_ids.length, 0);
        setBanner(
          lines > 0
            ? {
                source,
                lines,
                episodes: found.length,
                onlyThis: found.length === 1 && found[0].file === file,
              }
            : null,
        );
      })
      .catch(() => setBanner(null));
  };

  // ── body ──────────────────────────────────────────────────────────────────
  let body: ReactNode;
  if (view === null) {
    body = error ? (
      <Notice_>
        <p className="text-foreground/85">Couldn’t load this episode.</p>
        <p className="mt-1 break-words text-[11.5px]">{error}</p>
      </Notice_>
    ) : (
      <Notice_ tone="faint">Loading lines…</Notice_>
    );
  } else if (view.kind === "not_in_run") {
    body = (
      <Notice_>
        {folder
          ? "This episode hasn’t been translated yet."
          : "This episode isn’t part of the current run."}
      </Notice_>
    );
  } else if (view.kind === "unreadable") {
    body = (
      <Notice_>
        <p className="text-foreground/85">Couldn’t read this episode.</p>
        <p className="mt-1 break-words text-[11.5px]">{view.message}</p>
      </Notice_>
    );
  } else if (lines.length === 0) {
    body = <Notice_>This episode has no dialogue lines.</Notice_>;
  } else if (shown.length === 0) {
    body = <Notice_>Nothing left to look at.</Notice_>;
  } else {
    body = (
      <LineTable
        lines={shown}
        live={live}
        flags={flags}
        landed={landed}
        selectedId={selected}
        goTo={goTo}
      />
    );
  }

  return (
    <ReviewContext.Provider value={actions}>
      <div className="flex h-screen flex-col bg-background text-foreground">
        <EpisodeHeader
          file={file}
          snapshot={snapshot}
          liveIds={liveIds}
          flaggedLines={flagged.length}
          refreshError={view !== null ? error : null}
          nav={nav}
          onlyFlagged={onlyFlagged}
          onToggleOnlyFlagged={() => setOnlyFlagged((on) => !on)}
        />
        <main ref={mainRef} className="relative flex min-h-0 flex-1 flex-col">
          {body}
        </main>
        {snapshot && folder ? (
          <EpisodeFooter
            state={snapshot.state}
            flagged={flagged.length}
            blocked={blocked}
            busy={busy}
            notice={notice}
            banner={banner}
            shortcut={ADD_SHORTCUT}
            onRetranslateAll={() => actions?.retranslate(flagged)}
            onMarkDone={markDone}
            onAddToGlossary={(button) => openDraft(button)}
            onRetranslateTerm={retranslateTerm}
          />
        ) : null}
        {folder && selection && !draft ? (
          <SelectionButton sel={selection} onOpen={() => openDraft()} />
        ) : null}
        {folder && draft ? (
          <AddToGlossary
            key={`${draft.source}|${draft.target}|${draft.anchor.left}|${draft.anchor.top}`}
            folder={folder}
            draft={draft}
            onClose={() => setDraft(null)}
            onAdded={onAdded}
          />
        ) : null}
      </div>
    </ReviewContext.Provider>
  );
}
