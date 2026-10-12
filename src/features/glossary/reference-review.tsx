import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Sparkle, X } from "@phosphor-icons/react";
import { toast } from "sonner";
import type { ProjectView } from "@/types/generated/ProjectView";
import type { ReferenceTerminology } from "@/types/generated/ReferenceTerminology";
import { ipc } from "@/lib/ipc";
import { plural } from "@/lib/utils";
import { useGlossaryRun } from "@/stores/glossary-store";
import { referenceKey, referenceStatusKey, useImportReference } from "./use-import-reference";
import { PageHeader } from "@/components/page-header";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/empty-state";

// Category keys — keep in sync with `CATEGORIES` in src-tauri/src/glossary/model.rs.
const CATEGORIES = [
  "characters",
  "cultivation",
  "skills",
  "locations",
  "items",
  "organizations",
] as const;
type RefCategory = (typeof CATEGORIES)[number];

const LABELS: Record<RefCategory, string> = {
  characters: "Characters",
  cultivation: "Cultivation",
  skills: "Skills",
  locations: "Locations",
  items: "Items",
  organizations: "Organizations",
};

const countTerms = (t: ReferenceTerminology) =>
  CATEGORIES.reduce((n, c) => n + t[c].length, 0);

export function ReferenceReview({ view }: { view: ProjectView }) {
  const qc = useQueryClient();
  const busy = useGlossaryRun((s) => s.busy);
  const lastImport = useGlossaryRun((s) => s.lastImport);
  const closeReview = useGlossaryRun((s) => s.closeReview);
  const importFiles = useImportReference(view.folder);
  const { data: terms, isPending } = useQuery({
    queryKey: referenceKey(view.folder),
    queryFn: () => ipc.loadReference(view.folder),
  });

  if (isPending) return null;

  // Empty state: also what makes reference import reachable once a glossary exists.
  if (!terms || countTerms(terms) === 0) {
    return (
      <div className="flex h-full flex-col">
        <div className="flex-1">
          {lastImport?.cancelled ? (
            <div className="mx-5 mt-5 rounded-md border border-[color:var(--color-alert)]/40 bg-[color:var(--color-alert)]/10 px-4 py-2 text-[12px] text-[color:var(--color-alert)]">
              Import cancelled — no terms were extracted before the stop.
            </div>
          ) : null}
          <EmptyState
            title="Reference terms"
            description="No reference terms yet — import .ass files you've already translated well; their wording guides the glossary."
          />
        </div>
        <div className="flex items-center gap-3 border-t border-border bg-[color:var(--popover)] px-5 py-3">
          <Button disabled={busy !== null} onClick={() => void importFiles()}>
            Import files…
          </Button>
          <Button variant="secondary" onClick={closeReview}>
            Back
          </Button>
        </div>
      </div>
    );
  }

  const total = countTerms(terms);

  const removeTerm = (cat: RefCategory, term: string) => {
    const next: ReferenceTerminology = {
      ...terms,
      [cat]: terms[cat].filter((t) => t !== term),
    };
    // Optimistic; revert by refetch on failure.
    qc.setQueryData(referenceKey(view.folder), next);
    ipc.saveReference(view.folder, next).catch((e: unknown) => {
      toast.error(String(e));
      void qc.invalidateQueries({ queryKey: referenceKey(view.folder) });
    });
    void qc.invalidateQueries({ queryKey: referenceStatusKey(view.folder) });
  };

  const clearAll = async () => {
    await ipc.clearReference(view.folder);
    await qc.invalidateQueries({ queryKey: referenceKey(view.folder) });
    await qc.invalidateQueries({ queryKey: referenceStatusKey(view.folder) });
    closeReview();
  };

  // Generate exactly like CreateView's primary action (defaults: normalize on,
  // personalize off) — the natural next step after pruning.
  const generate = () => {
    const { startOp, setBuildMode, endOp, closeReview: close } = useGlossaryRun.getState();
    close();
    startOp("build", view.folder);
    setBuildMode("append");
    ipc
      .startGlossaryBuild({
        folder: view.folder,
        mode: "append",
        files: view.prefs.selected_files,
        worldType: view.prefs.world_override ?? view.detected_world,
        sourceLang: view.prefs.source_lang,
        targetLang: view.prefs.target_lang,
        normalize: true,
        personalize: false,
        personalizeContext: "",
      })
      .catch((e: unknown) => {
        // Rejected invoke = run never started; un-stick the page (step-3 lesson).
        endOp();
        toast.error(String(e));
      });
  };

  return (
    <div className="flex h-full flex-col">
      <PageHeader
        title={`Reference terms · ${total}${lastImport ? ` extracted from ${plural(lastImport.files_processed, "file")}` : ""}`}
        description="These English terms guide extraction — delete anything wrong before it steers your glossary."
      />
      <div className="flex-1 overflow-auto p-5">
        {lastImport?.cancelled ? (
          <div className="mb-3 rounded-md border border-[color:var(--color-alert)]/40 bg-[color:var(--color-alert)]/10 px-4 py-2 text-[12px] text-[color:var(--color-alert)]">
            Import cancelled — kept {plural(total, "term")}.
          </div>
        ) : null}
        {lastImport && lastImport.errors.length > 0 ? (
          <div className="mb-3 rounded-md border border-[color:var(--color-alert)]/40 bg-[color:var(--color-alert)]/10 px-4 py-2 text-[12px]">
            <p className="text-[color:var(--color-alert)]">
              ⚠ {lastImport.errors.length} batch{lastImport.errors.length !== 1 ? "es" : ""} failed —
              terms from those lines are missing (partial kept).
            </p>
            <ul className="mt-1 space-y-0.5">
              {lastImport.errors.map((e, i) => (
                <li key={i} className="text-muted-foreground">
                  {e}
                </li>
              ))}
            </ul>
          </div>
        ) : null}

        {CATEGORIES.filter((c) => terms[c].length > 0).map((cat) => (
          <div key={cat} className="mb-4">
            <div className="mb-1.5 flex items-baseline gap-2">
              <h2 className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                {LABELS[cat]}
              </h2>
              <span className="text-[11px] text-muted-foreground tabular-nums">
                {terms[cat].length}
              </span>
            </div>
            <div className="flex flex-wrap gap-1.5">
              {terms[cat].map((term) => (
                <span
                  key={term}
                  className="group inline-flex items-center gap-1 rounded-full border border-border bg-[color:var(--card)] px-2.5 py-0.5 text-[12px]"
                >
                  {term}
                  <button
                    type="button"
                    aria-label={`Delete ${term}`}
                    className="invisible text-muted-foreground transition-colors group-hover:visible hover:text-[color:var(--color-danger)]"
                    onClick={() => removeTerm(cat, term)}
                  >
                    <X className="size-3" />
                  </button>
                </span>
              ))}
            </div>
          </div>
        ))}
      </div>
      <div className="flex items-center gap-3 border-t border-border bg-[color:var(--popover)] px-5 py-3">
        <Button
          disabled={view.prefs.selected_files.length === 0 || busy !== null}
          onClick={generate}
        >
          <Sparkle className="size-4" /> Generate glossary →
        </Button>
        <Button variant="secondary" onClick={closeReview}>
          Back
        </Button>
        <span className="flex-1" />
        <button
          type="button"
          className="text-[12px] text-[color:var(--color-danger)] hover:underline"
          onClick={() => void clearAll()}
        >
          Clear all
        </button>
      </div>
    </div>
  );
}
