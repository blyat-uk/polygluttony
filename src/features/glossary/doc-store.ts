// The glossary as the workspace edits it: optimistic commits, saves in order,
// rollback on a failed save, and a session undo/redo stack.
//
// The react-query cache (`glossaryKey(folder)`) holds what's on screen. A
// commit writes the cache first, then queues the save. A folder's saves run one
// at a time so each keeps its own snapshot kind. When one fails, the cache goes back to
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
let nextEntryId = 1;

/** One folder's saves. Each folder keeps its own queue, so a failed save in
 *  one folder never rolls back another folder's cache or drops its saves. */
interface FolderSaves {
  /** The last state confirmed on disk: where a failed save rolls back to. */
  saved: GlossaryState | null;
  /** The last state put on screen (the cache). */
  lastWritten: GlossaryState | null;
  /** The commit `saved` came from. */
  savedSeq: number;
  /** Bumped when a save fails: the saves queued behind it are dropped. */
  generation: number;
  inFlight: number;
  chain: Promise<void>;
  /** A reload came back while saves were going: read the file again once
   *  they're done, so a change made elsewhere meanwhile still shows. */
  reloadAfter: boolean;
}

const folders = new Map<string, FolderSaves>();

function savesFor(folder: string): FolderSaves {
  let f = folders.get(folder);
  if (!f) {
    f = {
      saved: null,
      lastWritten: null,
      savedSeq: 0,
      generation: 0,
      inFlight: 0,
      chain: Promise.resolve(),
      reloadAfter: false,
    };
    folders.set(folder, f);
  }
  return f;
}

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
  savesFor(folder).lastWritten = s;
  // Rail badge + the cached ProjectView (staleTime: Infinity) stay honest.
  if (useAppStore.getState().workdir === folder) {
    useAppStore.getState().setGlossaryTerms(s.doc.count);
  }
  queryClient.setQueryData<ProjectView>(projectKey(folder), (v) =>
    v ? { ...v, glossary_terms: s.doc.count } : v,
  );
}

/** The workspace now edits `folder`, with a fresh undo history. */
function startHistory(folder: string) {
  const status: SaveStatus = savesFor(folder).inFlight > 0 ? "saving" : "saved";
  set({ folder, undo: [], redo: [], status, error: null, failed: null });
}

/** `loaded` is what's on disk now: it replaces whatever the store held. */
function resetFor(folder: string, loaded: GlossaryState) {
  const f = savesFor(folder);
  f.saved = loaded;
  f.lastWritten = loaded;
  f.savedSeq = seq;
  startHistory(folder);
}

/**
 * Call whenever the loaded glossary changes. Our own writes are ignored; a
 * different doc from disk (text editor, a scan, Restore, Import, an episode
 * window's Add to glossary) starts a new history, because undoing past it
 * would overwrite that change.
 */
export function syncLoaded(folder: string, loaded: GlossaryState) {
  const f = savesFor(folder);
  const switched = get().folder !== folder;
  if (f.lastWritten && sameDoc(f.lastWritten.doc, loaded.doc)) {
    if (switched) startHistory(folder);
    return;
  }
  if (f.inFlight > 0 && f.lastWritten) {
    // A refetch raced our own save and came back with the older file: keep
    // ours, and read the file again once the saves are done.
    f.reloadAfter = true;
    writeState(folder, f.lastWritten);
    if (switched) startHistory(folder);
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
  if (get().folder !== folder) {
    if (savesFor(folder).lastWritten) startHistory(folder);
    else resetFor(folder, prev);
  }
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
  const f = savesFor(folder);
  const gen = f.generation;
  f.inFlight++;
  if (get().folder === folder) set({ status: "saving", error: null, failed: null });
  f.chain = f.chain.then(async () => {
    try {
      // A save before this one failed: the cache was rolled back, drop this one.
      if (gen !== f.generation) return;
      markLocalSave();
      await ipc.saveGlossary(folder, state.doc, state.meta, snapshot ?? undefined);
      f.savedSeq = mySeq;
      f.saved = state;
    } catch (e: unknown) {
      if (gen !== f.generation) return;
      f.generation++;
      const wanted = f.lastWritten ?? state;
      if (f.saved) writeState(folder, f.saved);
      if (get().folder === folder) {
        set((s) => ({
          undo: s.undo.filter((x) => x.seq <= f.savedSeq),
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
      f.inFlight--;
      if (f.inFlight === 0) {
        if (get().folder === folder && get().status === "saving") set({ status: "saved" });
        if (f.reloadAfter) {
          f.reloadAfter = false;
          void queryClient.invalidateQueries({ queryKey: glossaryKey(folder) });
        }
      }
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
