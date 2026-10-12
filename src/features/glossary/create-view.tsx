import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Sparkle, Books } from "@phosphor-icons/react";
import { toast } from "sonner";
import type { ProjectView } from "@/types/generated/ProjectView";
import type { WorldType } from "@/types/generated/WorldType";
import { ipc } from "@/lib/ipc";
import { plural } from "@/lib/utils";
import { useGlossaryRun } from "@/stores/glossary-store";
import { referenceKey, referenceStatusKey, useImportReference } from "./use-import-reference";
import { useImportGlossary } from "./use-import-glossary";
import { Button } from "@/components/ui/button";
import { HelpText } from "@/components/help-text";
import { PageHeader } from "@/components/page-header";
import { GlossaryBuildOptions } from "./glossary-build-options";

// ── CreateView ────────────────────────────────────────────────────────────────

export function CreateView({ view }: { view: ProjectView }) {
  const qc = useQueryClient();
  const { startOp, setBuildMode, endOp } = useGlossaryRun.getState();
  const busy = useGlossaryRun((s) => s.busy);
  const summary = useGlossaryRun((s) => s.summary);
  const error = useGlossaryRun((s) => s.error);
  const openReview = useGlossaryRun((s) => s.openReview);

  // Generate card state
  const [normalize, setNormalize] = useState(true);
  const [personalize, setPersonalize] = useState(false);
  const [context, setContext] = useState("");

  // Reference import status chip / summary
  const { data: refStatus } = useQuery({
    queryKey: referenceStatusKey(view.folder),
    queryFn: () => ipc.referenceStatus(view.folder),
  });
  const { data: refTerms } = useQuery({
    queryKey: referenceKey(view.folder),
    queryFn: () => ipc.loadReference(view.folder),
    enabled: refStatus?.source === "cached",
  });

  const selected = view.prefs.selected_files;
  const effectiveWorld: WorldType = view.prefs.world_override ?? view.detected_world;
  const canGenerate = selected.length > 0 && busy === null;

  // ── generate action ──────────────────────────────────────────────────────────

  const generate = () => {
    startOp("build", view.folder);
    setBuildMode("append");
    // Rejected invoke = run never started; un-stick the page (step-3 lesson).
    ipc
      .startGlossaryBuild({
        folder: view.folder,
        mode: "append",
        files: selected,
        worldType: effectiveWorld,
        sourceLang: view.prefs.source_lang,
        targetLang: view.prefs.target_lang,
        normalize,
        personalize,
        personalizeContext: context,
      })
      .catch((e: unknown) => {
        endOp();
        toast.error(String(e));
      });
  };

  // ── import action (shared with the review screen's empty state) ─────────────

  const importFiles = useImportReference(view.folder);
  const importGlossary = useImportGlossary(view.folder);

  // ── clear reference action ────────────────────────────────────────────────────

  const clearRef = async () => {
    await ipc.clearReference(view.folder);
    await qc.invalidateQueries({ queryKey: referenceStatusKey(view.folder) });
    await qc.invalidateQueries({ queryKey: referenceKey(view.folder) });
  };

  // ── render ────────────────────────────────────────────────────────────────────

  return (
    <div className="flex h-full flex-col">
      <PageHeader
        title="Build your glossary"
        description="A glossary keeps character names & terms consistent across episodes. Pick how to create it:"
      />

      <div className="flex-1 overflow-auto p-5 flex flex-col gap-4">

        <HelpText>
          A glossary is your show’s name &amp; term dictionary — e.g. it keeps 张伟 as “Zhang Wei”
          and 灵气 as “spiritual energy” in every episode, instead of changing each time.
        </HelpText>

        {/* Error surfacing — hard requirement: failed/empty build returns here, user MUST see why */}
        {error ? (
          <p className="text-sm text-[color:var(--color-danger)]">{error}</p>
        ) : null}
        {summary?.cancelled ? (
          <p className="text-sm text-[color:var(--color-alert)]">
            Build cancelled — {plural(summary.terms_final, "term")} kept.
          </p>
        ) : null}
        {summary && summary.errors.length > 0 ? (
          <div className="rounded-md border border-[color:var(--color-danger)]/30 bg-[color:var(--color-danger)]/5 px-4 py-3 text-sm">
            <p className="font-medium text-foreground mb-1.5">
              Last build finished with {summary.errors.length} issue
              {summary.errors.length !== 1 ? "s" : ""}
              {summary.terms_final > 0
                ? ` — ${plural(summary.terms_final, "term")} ${summary.terms_final === 1 ? "was" : "were"} still saved:`
                : ":"}
            </p>
            <ul className="space-y-0.5">
              {summary.errors.map((msg, i) => (
                <li key={i} className="text-[12px] text-muted-foreground">
                  {msg}
                </li>
              ))}
            </ul>
          </div>
        ) : null}

        {/* Two-column card layout per spec */}
        <div className="grid grid-cols-2 gap-4">
          {/* Generate card */}
          <div className="rounded-lg border border-border bg-[color:var(--card)] p-5">
            <div className="mb-3 flex items-center gap-2">
              <Sparkle weight="duotone" className="size-4 text-primary" />
              <h2 className="text-sm font-semibold text-foreground">Generate from these subtitles</h2>
            </div>
            <p className="mb-4 text-[12.5px] text-muted-foreground">
              Scan the {plural(view.files.length, "file")} and extract names, terms &amp; places.
              Most common choice.
            </p>

            <GlossaryBuildOptions
              normalize={normalize}
              onNormalizeChange={setNormalize}
              personalize={personalize}
              onPersonalizeChange={setPersonalize}
              context={context}
              onContextChange={setContext}
              normalizeHelp="Fixes inconsistent spellings and drops terms too generic to be useful."
            />
          </div>

          {/* Import card */}
          <div className="rounded-lg border border-border bg-[color:var(--card)] p-5">
            <div className="mb-3 flex items-center gap-2">
              <Books weight="duotone" className="size-4 text-primary" />
              <h2 className="text-sm font-semibold text-foreground">Import from existing translations</h2>
            </div>
            <p className="mb-4 text-[12.5px] text-muted-foreground">
              Point me to .ass files you&apos;ve already translated well — their wording guides the
              new glossary.
            </p>

            <div className="flex items-center gap-3">
              <Button
                variant="secondary"
                onClick={() => void importFiles()}
                disabled={busy !== null}
              >
                {busy === "import" ? "Importing…" : "Choose files…"}
              </Button>

              {/* ref/ folder chip — cached imports get the summary block below */}
              {refStatus?.source === "ref_dir" ? (
                <span className="inline-flex items-center gap-1.5 rounded-full border border-border bg-[color:var(--color-bg-raised)] px-2.5 py-1 text-[11px] text-muted-foreground">
                  ref/ folder detected · {plural(refStatus.count, "file")}
                </span>
              ) : null}
            </div>

            {/* Cached-import summary: per-category counts + review entry point */}
            {refStatus?.source === "cached" ? (
              <div className="mt-3 rounded-md border border-border bg-[color:var(--color-bg-raised)] px-3 py-2 text-[11.5px]">
                <p className="text-foreground">
                  <span className="font-semibold">{plural(refStatus.count, "reference term")}</span> ·
                  imported
                </p>
                {refTerms ? (
                  <p className="mt-0.5 text-muted-foreground">
                    {(
                      [
                        "characters",
                        "cultivation",
                        "skills",
                        "locations",
                        "items",
                        "organizations",
                      ] as const
                    )
                      .filter((c) => refTerms[c].length > 0)
                      .map((c) => `${refTerms[c].length} ${c}`)
                      .join(" · ")}
                  </p>
                ) : null}
                <p className="mt-1 flex gap-3">
                  <button
                    type="button"
                    className="text-primary hover:underline"
                    onClick={() => openReview(view.folder)}
                  >
                    View / edit
                  </button>
                  <button
                    type="button"
                    className="text-[color:var(--color-danger)] hover:underline"
                    onClick={() => void clearRef()}
                  >
                    ✕ Clear
                  </button>
                </p>
              </div>
            ) : null}
          </div>
        </div>
      </div>

      {/* Footer bar */}
      <div className="flex items-center gap-3 border-t border-border bg-[color:var(--popover)] px-5 py-3">
        <Button onClick={generate} disabled={!canGenerate}>
          Generate glossary →
        </Button>
        <Button
          variant="secondary"
          onClick={() => void importGlossary()}
          disabled={busy !== null}
        >
          Start from an existing glossary…
        </Button>
        <span className="text-[11px] text-muted-foreground">
          {selected.length === 0
            ? "Select files in Project first."
            : `${selected.length} file${selected.length !== 1 ? "s" : ""} · world: ${effectiveWorld}`}
        </span>
      </div>
    </div>
  );
}
