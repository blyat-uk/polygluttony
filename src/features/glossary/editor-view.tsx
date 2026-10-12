import { Fragment, useEffect, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  ArrowsDownUp,
  Books,
  DownloadSimple,
  MagnifyingGlass,
  NotePencil,
  Play,
  Plus,
  Sparkle,
  Stop,
  UploadSimple,
  X,
} from "@phosphor-icons/react";
import { toast } from "sonner";
import type { ProjectView } from "@/types/generated/ProjectView";
import type { GlossaryDoc } from "@/types/generated/GlossaryDoc";
import type { GlossaryDiff } from "@/types/generated/GlossaryDiff";
import type { GlossaryBuildSummary } from "@/types/generated/GlossaryBuildSummary";
import type { NormalizeReview } from "@/types/generated/NormalizeReview";
import type { Language } from "@/types/generated/Language";
import { ipc } from "@/lib/ipc";
import { formatRelativeTime } from "@/lib/relative-time";
import { plural } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";
import { useGlossaryRun } from "@/stores/glossary-store";
import { projectKey } from "@/features/project/use-project";
import { glossaryKey, markLocalSave } from "./glossary-page";
import { referenceKey } from "./use-import-reference";
import { glossaryBackupKey, useImportGlossary } from "./use-import-glossary";
import { DiffReview } from "./diff-review";
import { PageHeader } from "@/components/page-header";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { SectionHelp } from "@/components/section-help";
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

// Category keys — keep in sync with `CATEGORIES` in src-tauri/src/glossary/model.rs.
const CATEGORIES = [
  "characters",
  "cultivation",
  "skills",
  "locations",
  "items",
  "organizations",
] as const;
type Category = (typeof CATEGORIES)[number];

const LABELS: Record<Category, string> = {
  characters: "Characters",
  cultivation: "Cultivation",
  skills: "Skills",
  locations: "Locations",
  items: "Items",
  organizations: "Organizations",
};

const SELECT_CLS =
  "h-9 rounded-md border border-input bg-[color:var(--card)] px-2 text-sm";

// The post-build diff is shown once per build *per session*, not once per mount —
// a useRef would re-pop the dialog every time the user navigates back here.
let shownSummary: GlossaryBuildSummary | null = null;

export function EditorView({ view, doc }: { view: ProjectView; doc: GlossaryDoc }) {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const { startOp, endOp, setLastDiff } = useGlossaryRun.getState();
  const busy = useGlossaryRun((s) => s.busy);
  const error = useGlossaryRun((s) => s.error);
  const lastDiff = useGlossaryRun((s) => s.lastDiff);
  const summary = useGlossaryRun((s) => s.summary);
  const openReview = useGlossaryRun((s) => s.openReview);
  const openUpdate = useGlossaryRun((s) => s.openUpdate);

  const { data: languages } = useQuery({
    queryKey: ["languages"],
    queryFn: ipc.listLanguages,
    staleTime: Infinity,
  });
  const langName = (code: string) =>
    languages?.find((l: Language) => l.code === code)?.name ?? code;
  const sourceName = langName(view.prefs.source_lang);
  const targetName = langName(view.prefs.target_lang);

  // Reference-term count for the toolbar button (also the import entry point
  // once a glossary exists — the review screen's empty state offers Import).
  const { data: refTerms } = useQuery({
    queryKey: referenceKey(view.folder),
    queryFn: () => ipc.loadReference(view.folder),
  });
  const refCount = refTerms
    ? CATEGORIES.reduce((n, c) => n + refTerms[c].length, 0)
    : 0;

  // An import replaces glossary.prev.json — the single undo slot. Disclose what
  // is in there before the user trades it away (same wording as UpdateView).
  // staleTime 0: this view remounts after every build, and the backup it
  // describes may have just been replaced by that build (see UpdateView).
  const { data: backup } = useQuery({
    queryKey: glossaryBackupKey(view.folder),
    queryFn: () => ipc.glossaryBackupStatus(view.folder),
    staleTime: 0,
  });
  const backupNote = backup
    ? `Replacing the existing backup, which holds ${backup.count} term${backup.count !== 1 ? "s" : ""} from ${formatRelativeTime(Number(backup.modified))}.`
    : "No backup exists yet, so your current terms will be saved to glossary.prev.json.";

  const [search, setSearch] = useState("");
  const [addCat, setAddCat] = useState<Category>("characters");
  const [addSrc, setAddSrc] = useState("");
  const [addTgt, setAddTgt] = useState("");
  const [editing, setEditing] = useState<{ cat: Category; source: string } | null>(null);
  const [review, setReview] = useState<NormalizeReview | null>(null);
  const [infoDiff, setInfoDiff] = useState<GlossaryDiff | null>(null);
  const [confirmImport, setConfirmImport] = useState(false);

  // Build finished while we were elsewhere (or just now) → surface its diff +
  // any non-fatal errors. CreateView owns the built-nothing case.
  // Guard: only surface results for the folder this run was actually started in.
  useEffect(() => {
    if (!summary || shownSummary === summary) return;
    if (useGlossaryRun.getState().folder !== view.folder) return;
    shownSummary = summary;
    for (const err of summary.errors) toast.warning(err);
    if (summary.diff.has_changes) setInfoDiff(summary.diff);
  }, [summary, view.folder]);

  // ── auto-save (O14): every mutation persists the whole doc ──────────────────

  const persist = (next: GlossaryDoc) => {
    // Optimistic: cache + rail badge first, the atomic write follows.
    qc.setQueryData(glossaryKey(view.folder), next);
    useAppStore.getState().setGlossaryTerms(next.count);
    // Keep the cached ProjectView honest too, or a later prefs save in Project
    // would sync a stale term count back into the shell.
    qc.setQueryData<ProjectView>(projectKey(view.folder), (v) =>
      v ? { ...v, glossary_terms: next.count } : v,
    );
    // Mark the save timestamp before the async write so the file-watcher
    // suppression window (I3) starts immediately.
    markLocalSave();
    ipc.saveGlossary(view.folder, next).catch((e: unknown) => toast.error(String(e)));
  };

  const recount = (terms: GlossaryDoc["terms"]) =>
    CATEGORIES.reduce((n, c) => n + Object.keys(terms[c] ?? {}).length, 0);

  const withTerm = (cat: Category, source: string, translation: string): GlossaryDoc => {
    const terms = {
      ...doc.terms,
      [cat]: { ...(doc.terms[cat] ?? {}), [source]: translation },
    };
    return { ...doc, terms, count: recount(terms) };
  };

  const withoutTerm = (cat: Category, source: string): GlossaryDoc => {
    const rest = { ...(doc.terms[cat] ?? {}) };
    delete rest[source];
    const terms = { ...doc.terms, [cat]: rest };
    return { ...doc, terms, count: recount(terms) };
  };

  // ── actions ──────────────────────────────────────────────────────────────────

  const addTerm = () => {
    const source = addSrc.trim();
    const translation = addTgt.trim();
    if (!source || !translation) return;
    // Backend merge semantics: a source must be unique across ALL categories.
    if (CATEGORIES.some((c) => Object.prototype.hasOwnProperty.call(doc.terms[c] ?? {}, source))) {
      toast.error(`"${source}" is already in the glossary`);
      return;
    }
    persist(withTerm(addCat, source, translation));
    setAddSrc("");
    setAddTgt("");
  };

  const commitEdit = (cat: Category, source: string, value: string) => {
    setEditing(null);
    const translation = value.trim();
    if (!translation || translation === doc.terms[cat]?.[source]) return;
    persist(withTerm(cat, source, translation));
  };

  const normalizeCancelled = useRef(false);

  const normalize = () => {
    normalizeCancelled.current = false;
    startOp("normalize", view.folder);
    ipc
      .normalizeGlossary(view.folder)
      .then((r) => {
        // A user-cancel resolves Ok with a no-changes review — discard it
        // (commands/glossary.rs cancel semantics) instead of presenting it.
        if (normalizeCancelled.current) {
          toast.info("Normalization cancelled.");
        } else if (r.diff.has_changes) {
          setReview(r);
        } else {
          toast.info("No changes — the glossary is already consistent.");
        }
      })
      .catch((e: unknown) => toast.error(String(e)))
      .finally(() => endOp());
  };

  const cancelNormalize = () => {
    normalizeCancelled.current = true;
    ipc.cancelGlossaryBuild().catch((e: unknown) => toast.error(String(e)));
  };

  const acceptReview = () => {
    if (!review) return;
    persist(review.normalized);
    setLastDiff(review.diff);
    setReview(null);
    toast.success("Normalization applied");
  };

  const exportGlossary = async () => {
    const dest = await saveDialog({
      defaultPath: "glossary.json",
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (!dest) return;
    try {
      await ipc.exportGlossary(view.folder, dest);
      toast.success("Glossary exported");
    } catch (e: unknown) {
      toast.error(String(e));
    }
  };

  const openInEditor = () =>
    ipc.openGlossaryEditor(view.folder).catch((e: unknown) => toast.error(String(e)));

  const importGlossary = useImportGlossary(view.folder);

  // ── filtering ────────────────────────────────────────────────────────────────

  const q = search.trim().toLowerCase();
  const sections = CATEGORIES.map((cat) => {
    const entries = Object.entries(doc.terms[cat] ?? {});
    const rows = q
      ? entries.filter(
          ([source, translation]) =>
            source.toLowerCase().includes(q) || translation.toLowerCase().includes(q),
        )
      : entries;
    return { cat, rows };
  }).filter((s) => s.rows.length > 0);

  // ── render ───────────────────────────────────────────────────────────────────

  return (
    <div className="flex h-full flex-col">
      <PageHeader
        title={`Glossary · ${plural(doc.count, "term")}`}
        description="Double-click any term to edit. Changes auto-save to glossary.json."
        actions={
          <div className="flex items-center gap-2">
            <Button size="sm" onClick={() => openUpdate(view.folder)} disabled={busy !== null}>
              <Plus className="size-4" /> Update glossary
            </Button>
            <Button size="sm" variant="secondary" onClick={normalize} disabled={busy !== null}>
              <Sparkle className="size-4" />
              {busy === "normalize" ? "Normalizing…" : "Normalize"}
            </Button>
            {busy === "normalize" ? (
              <Button size="sm" variant="destructive" onClick={cancelNormalize}>
                <Stop className="size-4" /> Cancel
              </Button>
            ) : null}
            {lastDiff?.has_changes ? (
              <Button size="sm" variant="secondary" onClick={() => setInfoDiff(lastDiff)}>
                <ArrowsDownUp className="size-4" /> View changes
              </Button>
            ) : null}
            <Button size="sm" variant="secondary" onClick={() => openReview(view.folder)} disabled={busy !== null}>
              <Books className="size-4" />
              Reference terms{refCount ? ` (${refCount})` : ""}
            </Button>
            <Button size="sm" variant="secondary" onClick={openInEditor}>
              <NotePencil className="size-4" /> Open in editor
            </Button>
            <Button
              size="sm"
              variant="secondary"
              onClick={() => setConfirmImport(true)}
              disabled={busy !== null}
            >
              <UploadSimple className="size-4" /> Import glossary…
            </Button>
            <Button size="sm" variant="secondary" onClick={() => void exportGlossary()}>
              <DownloadSimple className="size-4" /> Export
            </Button>
          </div>
        }
      />

      {error ? (
        <div className="px-5 pt-3">
          <p className="text-sm text-[color:var(--color-danger)]">{error}</p>
        </div>
      ) : null}

      <div className="flex-1 overflow-auto p-5">
        <SectionHelp title="What the six categories mean">
          <ul className="ml-1 space-y-1 text-[11.5px] text-muted-foreground">
            <li>
              <span className="font-medium text-foreground">Characters</span> — people: names,
              titles, forms of address.
            </li>
            <li>
              <span className="font-medium text-foreground">Cultivation</span> — realms, stages, and
              energy systems (xianxia/wuxia).
            </li>
            <li>
              <span className="font-medium text-foreground">Skills</span> — named techniques, moves,
              and abilities.
            </li>
            <li>
              <span className="font-medium text-foreground">Locations</span> — places: sects,
              cities, realms, landmarks.
            </li>
            <li>
              <span className="font-medium text-foreground">Items</span> — objects: weapons, pills,
              artifacts, treasures.
            </li>
            <li>
              <span className="font-medium text-foreground">Organizations</span> — groups: clans,
              sects, factions, dynasties.
            </li>
          </ul>
        </SectionHelp>

        {/* Search + add-term row */}
        <div className="mb-4 flex items-center gap-2">
          <div className="relative w-56 shrink-0">
            <MagnifyingGlass className="absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
            <Input
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Search terms…"
              className="pl-8"
            />
          </div>
          <div className="flex-1" />
          <select
            aria-label="Category"
            disabled={busy !== null}
            className={SELECT_CLS}
            value={addCat}
            onChange={(e) => setAddCat(e.target.value as Category)}
          >
            {CATEGORIES.map((c) => (
              <option key={c} value={c}>
                {LABELS[c]}
              </option>
            ))}
          </select>
          <Input
            value={addSrc}
            disabled={busy !== null}
            onChange={(e) => setAddSrc(e.target.value)}
            placeholder="Source term"
            className="w-40"
          />
          <Input
            value={addTgt}
            disabled={busy !== null}
            onChange={(e) => setAddTgt(e.target.value)}
            placeholder="Translation"
            className="w-40"
            onKeyDown={(e) => {
              if (e.key === "Enter") addTerm();
            }}
          />
          <Button variant="secondary" disabled={busy !== null} onClick={addTerm}>
            <Plus className="size-4" /> Add term
          </Button>
        </div>

        {/* Terms table */}
        {sections.length === 0 ? (
          <p className="text-sm text-muted-foreground">No terms match &#x201C;{search}&#x201D;.</p>
        ) : (
          <table className="w-full table-fixed border-collapse text-[12.5px]">
            <thead>
              <tr className="border-b border-border text-left text-[11px] uppercase tracking-wide text-muted-foreground">
                <th className="w-[45%] px-2 py-1.5 font-semibold">{sourceName}</th>
                <th className="px-2 py-1.5 font-semibold">{targetName}</th>
              </tr>
            </thead>
            <tbody>
              {sections.map(({ cat, rows }) => (
                <Fragment key={cat}>
                  <tr>
                    <td colSpan={2} className="px-2 pt-3 pb-1">
                      <span className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                        {LABELS[cat]}
                      </span>
                      <span className="ml-2 text-[11px] text-muted-foreground tabular-nums">
                        {rows.length}
                      </span>
                    </td>
                  </tr>
                  {rows.map(([source, translation]) => (
                    <tr
                      key={source}
                      className="group border-b border-border/50 hover:bg-[color:var(--card)]"
                      onDoubleClick={() => { if (!busy) setEditing({ cat, source }); }}
                    >
                      <td className="truncate px-2 py-1 text-foreground">{source}</td>
                      <td className="px-2 py-1">
                        <div className="flex items-center gap-2">
                          {editing?.cat === cat && editing.source === source ? (
                            <InlineEdit
                              initial={translation}
                              onCommit={(v) => commitEdit(cat, source, v)}
                              onCancel={() => setEditing(null)}
                            />
                          ) : (
                            <span className="min-w-0 flex-1 truncate text-foreground">
                              {translation}
                            </span>
                          )}
                          <button
                            type="button"
                            aria-label={`Delete ${source}`}
                            disabled={busy !== null}
                            className="invisible shrink-0 text-muted-foreground transition-colors group-hover:visible hover:text-[color:var(--color-danger)] disabled:pointer-events-none disabled:opacity-40"
                            onClick={() => persist(withoutTerm(cat, source))}
                          >
                            <X className="size-3.5" />
                          </button>
                        </div>
                      </td>
                    </tr>
                  ))}
                </Fragment>
              ))}
            </tbody>
          </table>
        )}
      </div>

      {/* Footer bar */}
      <div className="flex items-center gap-3 border-t border-border bg-[color:var(--popover)] px-5 py-3">
        <span className="text-[11px] text-muted-foreground">Looks good?</span>
        <Button onClick={() => void navigate({ to: "/translate" })}>
          <Play className="size-4" /> Go to Translate
        </Button>
      </div>

      {review ? (
        <DiffReview
          diff={review.diff}
          mode="review"
          onAccept={acceptReview}
          onClose={() => setReview(null)}
        />
      ) : null}
      {infoDiff ? (
        <DiffReview diff={infoDiff} mode="info" onClose={() => setInfoDiff(null)} />
      ) : null}

      <AlertDialog open={confirmImport} onOpenChange={setConfirmImport}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Replace this glossary?</AlertDialogTitle>
            <AlertDialogDescription>
              The current {doc.count} term{doc.count !== 1 ? "s" : ""} will be replaced by the
              glossary you pick. A copy is kept as glossary.prev.json.
              <span
                className={`mt-2 block ${backup ? "font-medium text-[color:var(--color-danger)]" : ""}`}
              >
                {backupNote}
                {backup ? " Those terms will no longer be recoverable." : ""}
              </span>
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction onClick={() => void importGlossary()}>
              Choose file…
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

/** Uncontrolled inline editor: Enter commits, Escape cancels, blur commits. */
function InlineEdit({
  initial,
  onCommit,
  onCancel,
}: {
  initial: string;
  onCommit: (value: string) => void;
  onCancel: () => void;
}) {
  // Enter/Escape unmount the input, which fires blur — `done` stops the
  // blur-commit from double-firing (or resurrecting a cancelled edit).
  const done = useRef(false);
  return (
    <Input
      autoFocus
      defaultValue={initial}
      className="h-6 min-w-0 flex-1 px-1.5 text-[12.5px] md:text-[12.5px]"
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          done.current = true;
          onCommit(e.currentTarget.value);
        } else if (e.key === "Escape") {
          done.current = true;
          onCancel();
        }
      }}
      onBlur={(e) => {
        if (!done.current) onCommit(e.currentTarget.value);
      }}
    />
  );
}
