import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import type { GlossaryBuildSummary } from "@/types/generated/GlossaryBuildSummary";
import type { Language } from "@/types/generated/Language";
import type { ProjectView } from "@/types/generated/ProjectView";
import type { TermRef } from "@/types/generated/TermRef";
import { ipc } from "@/lib/ipc";
import { folderTitle } from "@/lib/folder-title";
import { plural } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";
import { useGlossaryRun } from "@/stores/glossary-store";
import { Button } from "@/components/ui/button";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { useImportGlossary } from "./use-import-glossary";
import { commit, readState, redo, undo } from "./doc-store";
import {
  addTerm,
  deleteTerms,
  editTarget,
  markReviewed,
  moveTerms,
  renameSource,
} from "./edits";
import { changedKeys, filterRows, matchesSearch, matchesShow } from "./filters";
import { glossaryKey } from "./keys";
import { hasMod, isTextField } from "./keyboard";
import { episodes } from "./episodes";
import {
  buildRows,
  CATEGORY_LABEL,
  categoryOf,
  termKey,
  titleCase,
  type Category,
  type GlossaryState,
  type TermRow,
} from "./model";
import { applyReplace, type ReplaceOptions } from "./replace";
import { useSearchModel } from "./glossary-build-options";
import { WorkspaceHeader } from "./workspace-header";
import { TermToolbar } from "./term-toolbar";
import { TermTable, type AddDraft, type Highlight } from "./term-table";
import type { EditField } from "./term-row";
import { BulkBar } from "./bulk-bar";
import { ReplaceDialog } from "./replace-dialog";
import { RestoreDialog } from "./restore-dialog";
import { RebuildDialog } from "./rebuild-dialog";
import { ImprovePanel } from "./improve-panel";
import { AddEpisodesPanel } from "./add-episodes-panel";
import { useGlossaryUi, type ImproveMode, type ScopeChoice } from "./ui-store";

// A build's summary is announced once per session, not once per mount.
let seenSummary: GlossaryBuildSummary | null = null;

// The "Next: Translate episodes →" link lasts until the user leaves the page.
// Counted, so StrictMode's mount → unmount → mount doesn't clear it.
let workspaceMounts = 0;

const refOf = (r: TermRow): TermRef => ({ category: r.category, source: r.source });

/** The glossary workspace: search, filter, edit in place, bulk actions, Undo. */
export function EditorView({ view, state }: { view: ProjectView; state: GlossaryState }) {
  const folder = view.folder;
  const qc = useQueryClient();
  const busy = useGlossaryRun((s) => s.busy) !== null;
  const hasModel = useAppStore((s) => s.hasUsableConnection);
  const searchModel = useSearchModel();
  const ui = useGlossaryUi();
  const { search, show, category, panel } = ui;

  const { data: languages } = useQuery({
    queryKey: ["languages"],
    queryFn: ipc.listLanguages,
    staleTime: Infinity,
  });
  const langName = (code: string) => languages?.find((l: Language) => l.code === code)?.name ?? code;
  const sourceName = langName(view.prefs.source_lang);
  const targetName = langName(view.prefs.target_lang);

  // ── rows, filters, selection ──────────────────────────────────────────────

  const rows = useMemo(() => buildRows(state), [state]);
  const { visible, categoryCounts, showCounts } = useMemo(
    () => filterRows(rows, { search, show, category }, state.meta),
    [rows, search, show, category, state.meta],
  );

  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  // Drop ticks for rows that went away (deleted, moved, undone).
  useEffect(() => {
    setSelected((s) => {
      if (s.size === 0) return s;
      const keys = new Set(rows.map((r) => r.key));
      const next = new Set([...s].filter((k) => keys.has(k)));
      return next.size === s.size ? s : next;
    });
  }, [rows]);
  const selectedRows = useMemo(() => rows.filter((r) => selected.has(r.key)), [rows, selected]);
  const selectedRefs = useMemo(() => selectedRows.map(refOf), [selectedRows]);

  const selectRows = useCallback((keys: string[], on: boolean) => {
    setSelected((s) => {
      const next = new Set(s);
      for (const k of keys) {
        if (on) next.add(k);
        else next.delete(k);
      }
      return next;
    });
  }, []);

  const searchRef = useRef<HTMLInputElement>(null);
  const addSourceRef = useRef<HTMLInputElement>(null);
  const [draft, setDraft] = useState<AddDraft>({
    source: "",
    target: "",
    category: category ?? "characters",
  });
  // The add row follows the category chip.
  useEffect(() => {
    if (category) setDraft((d) => ({ ...d, category }));
  }, [category]);

  const [highlight, setHighlight] = useState<Highlight | null>(null);
  const flashRow = (key: string, strong = false) =>
    setHighlight((h) => ({ key, strong, n: (h?.n ?? 0) + 1 }));

  const clearFilters = () => {
    ui.setSearch("");
    ui.setShow("all");
    ui.setCategory(null);
  };

  const [replaceOpen, setReplaceOpen] = useState(false);
  const [restoreOpen, setRestoreOpen] = useState(false);
  const [rebuildOpen, setRebuildOpen] = useState(false);
  const [importOpen, setImportOpen] = useState(false);

  // ── edits ─────────────────────────────────────────────────────────────────

  /** What's on screen now (fresher than the `state` prop inside callbacks). */
  const current = () => readState(folder) ?? state;

  /** Offer to retranslate finished lines that use a term whose translation changed. */
  const offerRetranslate = (source: string) => {
    ipc
      .findTermLines(folder, source)
      .then((hits) => {
        const lines = hits.reduce((n, h) => n + h.line_ids.length, 0);
        if (lines === 0) return;
        toast(`Retranslate ${plural(lines, "line")} that use ${source} in ${episodes(hits.length)}`, {
          id: `term-lines:${source}`,
          duration: 12000,
          action: {
            label: "Retranslate",
            onClick: () => {
              ipc
                .retranslateTerm(folder, source)
                .then((n) => toast.success(`Retranslating ${plural(n, "line")}.`))
                .catch((e: unknown) => toast.error(`Couldn't retranslate. ${String(e)}`));
            },
          },
        });
      })
      // Not knowing which lines use a term is no reason to interrupt an edit.
      .catch(() => {});
  };

  const add = (): boolean => {
    const s = current();
    const r = addTerm(s, draft.category, draft.source, draft.target);
    if (!r.ok) {
      toast.error(r.error);
      return false;
    }
    const source = draft.source.trim();
    const target = draft.target.trim();
    commit(folder, r.state, { label: `Added ${source} → ${target}` });
    setDraft((d) => ({ ...d, source: "", target: "" }));
    // Make sure the new row is on screen before lighting it up.
    const row: TermRow = {
      key: termKey(draft.category, source),
      category: draft.category,
      source,
      target,
      origin: "yours",
      isNew: false,
    };
    const shown =
      matchesShow(row, show, changedKeys(r.state.meta)) &&
      matchesSearch(row, search.trim().toLowerCase()) &&
      (category === null || category === row.category);
    if (!shown) clearFilters();
    flashRow(row.key);
    return true;
  };

  /** Null when the edit went in (or changed nothing); otherwise why it was
   *  refused, shown under the field that stays open. */
  const commitEdit = (row: TermRow, field: EditField, value: string): string | null => {
    const s = current();
    const r = field === "target" ? editTarget(s, refOf(row), value) : renameSource(s, refOf(row), value);
    if (!r.ok) return r.error;
    if (r.state === s) return null;
    const v = value.trim();
    if (field === "target") {
      commit(folder, r.state, { label: `Changed ${row.source}: ${row.target} → ${v}` });
      offerRetranslate(row.source);
    } else {
      commit(folder, r.state, { label: `Renamed ${row.source} → ${v}` });
    }
    return null;
  };

  const moveRows = (list: TermRow[], to: Category, bulk: boolean) => {
    const moving = list.filter((r) => r.category !== to);
    if (!moving.length) return;
    const next = moveTerms(current(), moving.map(refOf), to);
    commit(folder, next, {
      label:
        moving.length === 1
          ? `Moved ${moving[0].source} to ${CATEGORY_LABEL[to]}`
          : `Moved ${plural(moving.length, "term")} to ${CATEGORY_LABEL[to]}`,
      snapshot: bulk ? "bulk_edit" : undefined,
    });
    if (bulk) setSelected(new Set(list.map((r) => termKey(to, r.source))));
  };

  const deleteRows = (list: TermRow[], bulk: boolean) => {
    if (!list.length) return;
    commit(folder, deleteTerms(current(), list.map(refOf)), {
      label:
        list.length === 1
          ? `Deleted ${list[0].source} → ${list[0].target}`
          : `Deleted ${plural(list.length, "term")}`,
      snapshot: bulk ? "bulk_edit" : undefined,
    });
    if (bulk) setSelected(new Set());
  };

  const reviewRows = (list: TermRow[]) => {
    const fresh = list.filter((r) => r.isNew);
    if (!fresh.length) return;
    commit(folder, markReviewed(current(), fresh.map(refOf)), {
      label:
        fresh.length === 1
          ? `Marked ${fresh[0].source} reviewed`
          : `Marked ${plural(fresh.length, "term")} reviewed`,
    });
  };

  const applyReplaceDialog = (keys: string[], opts: ReplaceOptions) => {
    const { state: next, changed } = applyReplace(current(), keys, opts);
    setReplaceOpen(false);
    if (!changed) {
      toast.info("Nothing to replace any more.");
      return;
    }
    commit(folder, next, {
      label: `Replaced “${opts.find}” with “${opts.replace}” in ${plural(changed, "term")}`,
      snapshot: "replace",
    });
  };

  // ── panels ────────────────────────────────────────────────────────────────

  /** A paid result waiting for review keeps its panel until it's applied or discarded. */
  const reviewPending = () => {
    const imp = useGlossaryUi.getState().improve;
    if (!imp || imp.stage === "setup") return false;
    ui.openPanel({ kind: "improve" });
    toast.info(
      imp.stage === "running"
        ? "Wait for the run to finish, or press Stop."
        : "Apply or discard the proposed changes first.",
    );
    return true;
  };

  const openImprove = (mode: ImproveMode, scope?: ScopeChoice) => {
    if (reviewPending()) return;
    useGlossaryUi.setState({
      panel: { kind: "improve" },
      improve: {
        mode,
        scope: scope ?? (selected.size > 0 ? { kind: "selected" } : { kind: "all" }),
        showName: state.meta.show_name ?? folderTitle(folder),
        stage: "setup",
        runId: 0,
        runCount: 0,
        changes: [],
        ticked: [],
      },
    });
  };

  const openAddEpisodes = (preset: string[] | null = null) => {
    if (reviewPending()) return;
    useGlossaryUi.setState({ panel: { kind: "add-episodes", preset }, improve: null });
  };

  // ── cross-page requests ───────────────────────────────────────────────────

  // Episodes → "Add new episodes" with these files ticked.
  const pendingAdd = useGlossaryRun((s) => s.pendingAddEpisodes);
  useEffect(() => {
    if (!pendingAdd || pendingAdd.folder !== folder) return;
    useGlossaryRun.getState().clearAddEpisodes();
    openAddEpisodes(pendingAdd.files);
    // openAddEpisodes reads the stores; only a new request matters here.
  }, [pendingAdd, folder]);

  // Episode window → "Fix in glossary": show that term.
  const pendingFocus = useGlossaryRun((s) => s.pendingFocusTerm);
  useEffect(() => {
    if (!pendingFocus) return;
    useGlossaryRun.getState().clearFocusTerm();
    if (pendingFocus.folder !== folder) return;
    const source = pendingFocus.source;
    // The cache, not `rows`: the glossary was just reloaded (the term may have
    // been added in that window) and this render can predate it.
    const cat = categoryOf(current().doc, source);
    ui.setShow("all");
    ui.setCategory(null);
    if (cat) {
      ui.setSearch(source);
      flashRow(termKey(cat, source), true);
    } else {
      // Not in the glossary yet: start the add row with it.
      ui.setSearch("");
      setDraft((d) => ({ ...d, source, target: "" }));
      toast.info(`${source} isn’t in the glossary yet. Type its translation in the first row.`);
      setTimeout(() => addSourceRef.current?.focus(), 0);
    }
  }, [pendingFocus, folder]);

  // ── after a build ─────────────────────────────────────────────────────────

  const summary = useGlossaryRun((s) => s.summary);
  useEffect(() => {
    if (!summary || seenSummary === summary) return;
    if (useGlossaryRun.getState().folder !== folder) return;
    seenSummary = summary;
    for (const err of summary.errors) toast.warning(err);
    if (!useGlossaryUi.getState().afterScan) return;
    useGlossaryUi.setState({ afterScan: false });
    const added = summary.diff.total_added;
    if (summary.cancelled) toast.info(`Stopped. ${plural(added, "new term")} kept.`);
    else if (added === 0) toast.info("No new names in those episodes.");
    else toast.success(`Added ${plural(added, "new term")}.`);
    if (added === 0) return;
    // Show the new terms once the reloaded glossary (with its NEW tags) is in.
    void qc.refetchQueries({ queryKey: glossaryKey(folder) }).then(() => {
      const s = readState(folder);
      if (s && Object.values(s.meta.unreviewed).some((l) => l.length > 0)) {
        ui.setSearch("");
        ui.setCategory(null);
        ui.setShow("new");
      }
    });
  }, [summary, folder]);

  // "Next: Translate episodes →" only right after the first build.
  const showNext = ui.firstBuild === folder;
  useEffect(() => {
    workspaceMounts++;
    return () => {
      workspaceMounts--;
      setTimeout(() => {
        if (workspaceMounts === 0 && useGlossaryRun.getState().busy === null) {
          useGlossaryUi.setState({ firstBuild: null });
        }
      }, 0);
    };
  }, []);

  // ── shortcuts: Ctrl/Cmd+F search, Ctrl/Cmd+Z undo, Ctrl/Cmd+Shift+Z redo ───

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!hasMod(e) || e.altKey) return;
      const k = e.key.toLowerCase();
      if (k !== "f" && k !== "z" && k !== "y") return;
      // A dialog over the page owns the keyboard.
      if (document.querySelector('[role="dialog"], [role="alertdialog"]')) return;
      if (k === "f") {
        if (e.shiftKey) return;
        e.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
        return;
      }
      // In a text field, Ctrl+Z is the field's own undo.
      if (isTextField(e.target)) return;
      e.preventDefault();
      if (k === "y" || e.shiftKey) redo(folder);
      else undo(folder);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [folder]);

  // ── ··· menu ──────────────────────────────────────────────────────────────

  const importGlossary = useImportGlossary(folder);

  const exportGlossary = async () => {
    const dest = await saveDialog({
      defaultPath: "glossary.json",
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (!dest) return;
    try {
      await ipc.exportGlossary(folder, dest);
      toast.success("Glossary exported.");
    } catch (e: unknown) {
      toast.error(String(e));
    }
  };

  const openInEditor = () =>
    ipc.openGlossaryEditor(folder).catch((e: unknown) => toast.error(String(e)));

  // ── render ────────────────────────────────────────────────────────────────

  const world = state.doc.world_type || view.prefs.world_override || view.detected_world;
  const canLookup = !!searchModel;

  return (
    <div className="flex h-full flex-col">
      <WorkspaceHeader
        folder={folder}
        count={state.doc.count}
        subtitle={`${folderTitle(folder)} · ${titleCase(world)}`}
        showNext={showNext}
        searchModel={searchModel}
        canRunModel={hasModel}
        busy={busy}
        actions={{
          onTidy: () => openImprove("tidy"),
          onLookup: canLookup ? () => openImprove("lookup") : undefined,
          onAddEpisodes: () => openAddEpisodes(),
          onImportGlossary: () => setImportOpen(true),
          onImportTranslated: () => useGlossaryRun.getState().openReview(folder),
          onExport: () => void exportGlossary(),
          onOpenEditor: () => void openInEditor(),
          onRestore: () => setRestoreOpen(true),
          onRebuild: () => setRebuildOpen(true),
        }}
      />

      <div className="relative flex min-h-0 flex-1 flex-col">
        <TermToolbar
          searchRef={searchRef}
          search={search}
          onSearch={ui.setSearch}
          searchPlaceholder={`Search ${sourceName} or ${targetName}…`}
          show={show}
          onShow={ui.setShow}
          showCounts={showCounts}
          category={category}
          onCategory={ui.setCategory}
          categoryCounts={categoryCounts}
          shown={visible.length}
          total={state.doc.count}
        />

        <TermTable
          rows={visible}
          selected={selected}
          onSelectRows={selectRows}
          sourceLabel={sourceName}
          targetLabel={targetName}
          disabled={false}
          draft={draft}
          onDraftChange={setDraft}
          onAdd={add}
          addSourceRef={addSourceRef}
          highlight={highlight}
          bottomPad={selected.size > 0 && !panel ? 72 : 0}
          actions={{
            onCommitEdit: commitEdit,
            onMove: (row, to) => moveRows([row], to, false),
            onDelete: (row) => deleteRows([row], false),
            onMarkReviewed: (row) => reviewRows([row]),
            onLookup: canLookup
              ? (row) => openImprove("lookup", { kind: "one", ref: refOf(row) })
              : undefined,
          }}
          empty={
            state.doc.count === 0 ? (
              <div className="p-10 text-center text-[13px] text-muted-foreground">
                No terms yet. Type one in the first row, or use Add new episodes… to scan
                episodes for names.
              </div>
            ) : (
              <div className="flex flex-col items-center gap-2 p-10 text-center text-[13px] text-muted-foreground">
                No terms match.
                <Button variant="outline" size="sm" onClick={clearFilters}>
                  Show all terms
                </Button>
              </div>
            )
          }
        />

        {selected.size > 0 && !panel ? (
          <BulkBar
            count={selectedRows.length}
            anyNew={selectedRows.some((r) => r.isNew)}
            disabled={false}
            canTidy={hasModel && !busy}
            onMove={(to) => moveRows(selectedRows, to, true)}
            onReplace={() => setReplaceOpen(true)}
            onTidy={() => openImprove("tidy", { kind: "selected" })}
            onLookup={canLookup && !busy ? () => openImprove("lookup", { kind: "selected" }) : undefined}
            onMarkReviewed={() => reviewRows(selectedRows)}
            onDelete={() => deleteRows(selectedRows, true)}
            onClear={() => setSelected(new Set())}
          />
        ) : null}

        {panel?.kind === "improve" ? (
          <ImprovePanel
            folder={folder}
            state={state}
            selected={selectedRefs}
            searchModel={searchModel}
          />
        ) : panel?.kind === "add-episodes" ? (
          <AddEpisodesPanel
            key={panel.preset?.join("\u0000") ?? ""}
            view={view}
            meta={state.meta}
            preset={panel.preset}
            onClose={ui.closePanel}
          />
        ) : null}
      </div>

      <ReplaceDialog
        open={replaceOpen}
        onOpenChange={setReplaceOpen}
        allRows={rows}
        selectedRows={selectedRows}
        onApply={applyReplaceDialog}
      />
      <RestoreDialog folder={folder} open={restoreOpen} onOpenChange={setRestoreOpen} />
      <RebuildDialog
        view={view}
        count={state.doc.count}
        open={rebuildOpen}
        onOpenChange={setRebuildOpen}
      />
      <AlertDialog open={importOpen} onOpenChange={setImportOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Replace this glossary?</AlertDialogTitle>
            <AlertDialogDescription>
              The current {plural(state.doc.count, "term")} are replaced by the glossary you pick.
              A copy is saved first; bring it back with Restore an earlier version…
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction onClick={() => void importGlossary()}>Choose file…</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
