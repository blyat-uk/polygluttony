// The glossary as the workspace edits it: optimistic commits, saves in order,
// rollback on a failed save, and a session undo/redo stack.
//
// The react-query cache (`glossaryKey(folder)`) holds what's on screen. A
// commit writes the cache first, then queues the save. Saves run one at a time
// so each keeps its own snapshot kind. When one fails, the cache goes back to
// the last saved state, the edits that never reached disk leave the undo stack,
// and the header offers Retry with the latest wanted state.

import { create } from "zustand";
import { toast } from "sonner";
import type { GlossaryChangeKind } from "@/types/generated/GlossaryChangeKind";
import type { GlossaryLoad } from "@/types/generated/GlossaryLoad";
import type { ProjectView } from "@/types/generated/ProjectView";
import { ipc } from "@/lib/ipc";
import { queryClient } from "@/lib/query-client";
import { useAppStore } from "@/stores/app-store";
import { projectKey } from "@/features/project/use-project";
import { glossaryKey, markLocalSave } from "./keys";
import { shortcut } from "./keyboard";
import { sameDoc, type GlossaryState } from "./model";

export type SaveStatus = "saved" | "saving" | "error";

interface HistoryEntry {
  id: number;
  /** The commit that put this entry where it is (for dropping unsaved ones). */
  seq: number;
  label: string;
  before: GlossaryState;
  after: GlossaryState;
}

interface DocStore {
  folder: string | null;
  undo: HistoryEntry[];
  redo: HistoryEntry[];
  status: SaveStatus;
  error: string | null;
  /** What Retry saves after a failure. */
  failed: { state: GlossaryState; snapshot: GlossaryChangeKind | null; label: string } | null;
}

const MAX_HISTORY = 100;
const CHANGE_TOAST = "glossary-change";
const SAVE_TOAST = "glossary-save";

export const useGlossaryDoc = create<DocStore>(() => ({
  folder: null,
  undo: [],
  redo: [],
  status: "saved",
  error: null,
  failed: null,
}));

// Non-reactive bookkeeping.
let seq = 0;
let savedSeq = 0;
let saved: GlossaryState | null = null;
let lastWritten: GlossaryState | null = null;
let generation = 0;
let inFlight = 0;
let chain: Promise<void> = Promise.resolve();
let nextEntryId = 1;

const set = useGlossaryDoc.setState;
const get = useGlossaryDoc.getState;

/** The state on screen for `folder`, or null when the glossary isn't loaded. */
export function readState(folder: string): GlossaryState | null {
  const d = queryClient.getQueryData<GlossaryLoad>(glossaryKey(folder));
  return d && d.status === "ok" ? { doc: d.doc, meta: d.meta } : null;
}

function writeState(folder: string, s: GlossaryState) {
  const load: GlossaryLoad = { status: "ok", doc: s.doc, meta: s.meta };
  queryClient.setQueryData<GlossaryLoad>(glossaryKey(folder), load);
  lastWritten = s;
  // Rail badge + the cached ProjectView (staleTime: Infinity) stay honest.
  if (useAppStore.getState().workdir === folder) {
    useAppStore.getState().setGlossaryTerms(s.doc.count);
  }
  queryClient.setQueryData<ProjectView>(projectKey(folder), (v) =>
    v ? { ...v, glossary_terms: s.doc.count } : v,
  );
}

function resetFor(folder: string, loaded: GlossaryState) {
  saved = loaded;
  lastWritten = loaded;
  savedSeq = seq;
  set({ folder, undo: [], redo: [], status: "saved", error: null, failed: null });
}

/**
 * Call whenever the loaded glossary changes. Our own writes are ignored; a
 * different doc from disk (text editor, a scan, Restore, Import) starts a new
 * history, because undoing past it would overwrite that change.
 */
export function syncLoaded(folder: string, loaded: GlossaryState) {
  if (get().folder !== folder || !lastWritten) {
    resetFor(folder, loaded);
    return;
  }
  if (sameDoc(lastWritten.doc, loaded.doc)) return;
  if (inFlight > 0) {
    // A refetch raced our own save and came back with the older file: keep ours.
    writeState(folder, lastWritten);
    return;
  }
  resetFor(folder, loaded);
}

/** Start over for a folder (e.g. the glossary was deleted or rebuilt). */
export function forgetHistory(folder: string) {
  if (get().folder === folder) set({ undo: [], redo: [] });
}

export interface CommitOptions {
  /** History label, e.g. "Deleted 九婴 → Jiuying". */
  label: string;
  /** Save the current version to the glossary history first. */
  snapshot?: GlossaryChangeKind;
  /** false: no undo entry and no toast (e.g. remembering the show name). */
  undoable?: boolean;
  /** Toast text when it differs from the label; false = no toast. */
  toast?: string | false;
}

/** Show `next`, save it, and record it for Undo. */
export function commit(folder: string, next: GlossaryState, opts: CommitOptions) {
  const prev = readState(folder);
  if (!prev) return;
  if (get().folder !== folder) resetFor(folder, prev);
  writeState(folder, next);
  const mySeq = ++seq;
  if (opts.undoable !== false) {
    const entry: HistoryEntry = { id: nextEntryId++, seq: mySeq, label: opts.label, before: prev, after: next };
    set((s) => ({ undo: [...s.undo.slice(-(MAX_HISTORY - 1)), entry], redo: [] }));
    if (opts.toast !== false) {
      toast(opts.toast ?? opts.label, {
        id: CHANGE_TOAST,
        action: { label: "Undo", onClick: () => undoEntry(folder, entry.id) },
      });
    }
  }
  enqueueSave(folder, next, mySeq, opts.snapshot ?? null, opts.label);
}

function enqueueSave(
  folder: string,
  state: GlossaryState,
  mySeq: number,
  snapshot: GlossaryChangeKind | null,
  label: string,
) {
  const gen = generation;
  inFlight++;
  if (get().folder === folder) set({ status: "saving", error: null, failed: null });
  chain = chain.then(async () => {
    try {
      // A save before this one failed: the cache was rolled back, drop this one.
      if (gen !== generation) return;
      markLocalSave();
      await ipc.saveGlossary(folder, state.doc, state.meta, snapshot ?? undefined);
      savedSeq = mySeq;
      saved = state;
    } catch (e: unknown) {
      if (gen !== generation) return;
      generation++;
      const wanted = lastWritten ?? state;
      if (saved) writeState(folder, saved);
      if (get().folder === folder) {
        set((s) => ({
          undo: s.undo.filter((x) => x.seq <= savedSeq),
          redo: [],
          status: "error",
          error: String(e),
          failed: { state: wanted, snapshot, label },
        }));
      }
      toast.error(`Couldn't save the glossary. ${String(e)}`, {
        id: SAVE_TOAST,
        action: { label: "Retry", onClick: () => retrySave(folder) },
      });
    } finally {
      inFlight--;
      if (inFlight === 0 && get().status === "saving") set({ status: "saved" });
    }
  });
}

/** Save again what the failed save tried to write. */
export function retrySave(folder: string) {
  const f = get().failed;
  if (!f || get().folder !== folder) return;
  toast.dismiss(SAVE_TOAST);
  commit(folder, f.state, { label: f.label, snapshot: f.snapshot ?? undefined, toast: false });
}

/** Undo/redo never touch the show name: it's remembered, not edited. */
function keepShowName(target: GlossaryState, cur: GlossaryState): GlossaryState {
  if (target.meta.show_name === cur.meta.show_name) return target;
  return { doc: target.doc, meta: { ...target.meta, show_name: cur.meta.show_name } };
}

export function undo(folder: string): boolean {
  const s = get();
  const e = s.undo[s.undo.length - 1];
  const cur = readState(folder);
  if (!e || !cur || s.folder !== folder) return false;
  const next = keepShowName(e.before, cur);
  writeState(folder, next);
  const mySeq = ++seq;
  set({ undo: s.undo.slice(0, -1), redo: [...s.redo, { ...e, seq: mySeq }] });
  enqueueSave(folder, next, mySeq, null, `Undo: ${e.label}`);
  toast(`Undone: ${e.label}`, {
    id: CHANGE_TOAST,
    action: { label: "Redo", onClick: () => void redo(folder) },
  });
  return true;
}

export function redo(folder: string): boolean {
  const s = get();
  const e = s.redo[s.redo.length - 1];
  const cur = readState(folder);
  if (!e || !cur || s.folder !== folder) return false;
  const next = keepShowName(e.after, cur);
  writeState(folder, next);
  const mySeq = ++seq;
  set({ redo: s.redo.slice(0, -1), undo: [...s.undo, { ...e, seq: mySeq }] });
  enqueueSave(folder, next, mySeq, null, e.label);
  toast(e.label, {
    id: CHANGE_TOAST,
    action: { label: "Undo", onClick: () => undoEntry(folder, e.id) },
  });
  return true;
}

/** A toast's Undo: only when its change is still the latest one. */
function undoEntry(folder: string, id: number) {
  const top = get().undo[get().undo.length - 1];
  if (top?.id === id) undo(folder);
  else {
    toast.info(`Newer changes came after this one. Press ${shortcut("Z")} to step back.`, {
      id: CHANGE_TOAST,
    });
  }
}
