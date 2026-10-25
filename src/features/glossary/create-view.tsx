import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { Books, UploadSimple } from "@phosphor-icons/react";
import type { ProjectView } from "@/types/generated/ProjectView";
import { ipc } from "@/lib/ipc";
import { folderTitle } from "@/lib/folder-title";
import { plural } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";
import { useGlossaryRun } from "@/stores/glossary-store";
import { usePrimaryAction } from "@/hooks/use-primary-action";
import { Button } from "@/components/ui/button";
import { PageHeader } from "@/components/page-header";
import { referenceKey, referenceStatusKey, useImportReference } from "./use-import-reference";
import { useImportGlossary } from "./use-import-glossary";
import { EpisodeChecklist } from "./episode-checklist";
import { episodes } from "./episodes";
import { LookupOption, TidyOption, useSearchModel } from "./glossary-build-options";
import { CATEGORIES, CATEGORY_LABEL, titleCase } from "./model";
import { startBuild } from "./start-build";

/** Build glossary: the first glossary for a folder. */
export function CreateView({ view }: { view: ProjectView }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const busy = useGlossaryRun((s) => s.busy);
  const summary = useGlossaryRun((s) => s.summary);
  const error = useGlossaryRun((s) => s.error);
  // The last build here ran to the end and found nothing.
  const emptyBuild = useGlossaryRun(
    (s) => s.folder === view.folder && s.summary !== null && s.summary.terms_final === 0,
  );
  const openReview = useGlossaryRun((s) => s.openReview);
  const hasModel = useAppStore((s) => s.hasUsableConnection);
  const searchModel = useSearchModel();

  // Its own episode list, all ticked; not the Episodes page's selection.
  const [selected, setSelected] = useState<string[]>(() => view.files.map((f) => f.name));
  const [tidy, setTidy] = useState(true);
  const [lookup, setLookup] = useState(false);
  const [showName, setShowName] = useState(() => folderTitle(view.folder));

  const { data: refStatus } = useQuery({
    queryKey: referenceStatusKey(view.folder),
    queryFn: () => ipc.referenceStatus(view.folder),
  });
  const { data: refTerms } = useQuery({
    queryKey: referenceKey(view.folder),
    queryFn: () => ipc.loadReference(view.folder),
    enabled: refStatus?.source === "cached",
  });

  const world = titleCase(view.prefs.world_override ?? view.detected_world);
  const useLookup = lookup && !!searchModel;
  const canBuild =
    hasModel && selected.length > 0 && busy === null && (!useLookup || showName.trim() !== "");

  const build = () => {
    if (!canBuild) return;
    startBuild(view, { kind: "first", files: selected, tidy, lookup: useLookup, showName });
  };
  usePrimaryAction(build, canBuild);

  const importTranslated = useImportReference(view.folder);
  const importGlossary = useImportGlossary(view.folder);

  const clearRef = async () => {
    await ipc.clearReference(view.folder);
    await qc.invalidateQueries({ queryKey: referenceStatusKey(view.folder) });
    await qc.invalidateQueries({ queryKey: referenceKey(view.folder) });
  };

  return (
    <div className="flex h-full flex-col">
      <PageHeader
        title="Build glossary"
        description="A glossary keeps names and terms the same in every episode: 张伟 is always “Zhang Wei”, 灵气 is always “spiritual energy”."
      />

      <div className="flex flex-1 flex-col gap-5 overflow-auto p-5">
        {/* A failed or empty build lands back here: always say why. */}
        {error ? <p className="text-sm text-[color:var(--color-danger)]">{error}</p> : null}
        {summary?.cancelled ? (
          <p className="text-sm text-[color:var(--color-alert)]">
            Stopped. {plural(summary.terms_final, "term")} kept.
          </p>
        ) : null}
        {summary && emptyBuild && !summary.cancelled && summary.errors.length === 0 && !error ? (
          <p className="text-sm text-[color:var(--color-alert)]">
            No names or terms were found in these episodes. Check the source language on
            Episodes, or pick other episodes.
          </p>
        ) : null}
        {summary && summary.errors.length > 0 ? (
          <div className="rounded-xl border border-[color:var(--color-danger)]/30 bg-[color:var(--color-danger)]/5 px-4 py-3 text-sm">
            <p className="mb-1.5 font-medium text-foreground">
              The last build ran into {plural(summary.errors.length, "problem")}
              {summary.terms_final > 0
                ? `; ${plural(summary.terms_final, "term")} ${summary.terms_final === 1 ? "was" : "were"} still saved:`
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

        <div className="grid gap-5 lg:grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)]">
          <section className="flex min-w-0 flex-col gap-2">
            <h2 className="text-[13px] font-semibold text-foreground">Episodes to read</h2>
            <EpisodeChecklist files={view.files} selected={selected} onChange={setSelected} />
          </section>

          <section className="flex min-w-0 flex-col gap-4">
            <h2 className="text-[13px] font-semibold text-foreground">Options</h2>
            <TidyOption checked={tidy} onChange={setTidy} />
            <LookupOption
              checked={lookup}
              onChange={setLookup}
              showName={showName}
              onShowNameChange={setShowName}
            />
          </section>
        </div>

        <section className="flex flex-col gap-3 border-t border-border pt-5">
          <h2 className="text-[13px] font-semibold text-foreground">Other ways to start</h2>
          <div className="flex flex-wrap items-start gap-x-8 gap-y-4">
            <div className="flex max-w-md min-w-0 flex-1 flex-col gap-2">
              <div className="flex items-center gap-2">
                <Books weight="duotone" className="size-4 text-primary" />
                <span className="text-[13px] font-medium text-foreground">
                  Import from translated episodes
                </span>
              </div>
              <p className="text-[12px] text-muted-foreground">
                Point PG at .ass files you’ve already translated well. Their wording guides the
                names it finds.
              </p>
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => void importTranslated()}
                  disabled={busy !== null}
                >
                  Choose files…
                </Button>
                {refStatus?.source === "ref_dir" ? (
                  <span className="rounded-[13px] border border-border px-2.5 py-0.5 text-[11px] text-muted-foreground">
                    ref/ folder found · {plural(refStatus.count, "file")}
                  </span>
                ) : null}
              </div>
              {refStatus?.source === "cached" ? (
                <div className="rounded-xl border border-border bg-[color:var(--card)] px-3 py-2 text-[11.5px]">
                  <p className="text-foreground">
                    <span className="font-semibold">{plural(refStatus.count, "term")}</span> from
                    translated episodes
                  </p>
                  {refTerms ? (
                    <p className="mt-0.5 text-muted-foreground">
                      {CATEGORIES.filter((c) => refTerms[c].length > 0)
                        .map((c) => `${CATEGORY_LABEL[c]} ${refTerms[c].length}`)
                        .join(" · ")}
                    </p>
                  ) : null}
                  <p className="mt-1 flex gap-3">
                    <button
                      type="button"
                      className="text-primary hover:underline"
                      onClick={() => openReview(view.folder)}
                    >
                      View
                    </button>
                    <button
                      type="button"
                      className="text-[color:var(--color-danger)] hover:underline"
                      onClick={() => void clearRef()}
                    >
                      Clear
                    </button>
                  </p>
                </div>
              ) : null}
            </div>

            <div className="flex max-w-md min-w-0 flex-1 flex-col gap-2">
              <div className="flex items-center gap-2">
                <UploadSimple weight="duotone" className="size-4 text-primary" />
                <span className="text-[13px] font-medium text-foreground">Import a glossary</span>
              </div>
              <p className="text-[12px] text-muted-foreground">
                Use a glossary.json you already have, from another folder or a friend.
              </p>
              <div>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => void importGlossary()}
                  disabled={busy !== null}
                >
                  Choose file…
                </Button>
              </div>
            </div>
          </div>
        </section>
      </div>

      <div className="flex items-center gap-3 border-t border-border bg-[color:var(--popover)] px-5 py-3">
        <Button onClick={build} disabled={!canBuild}>
          Build glossary
        </Button>
        {hasModel ? (
          <span className="text-[11.5px] text-muted-foreground">
            {selected.length === 0
              ? "Tick at least one episode."
              : `${episodes(selected.length)} · World type: ${world}`}
          </span>
        ) : (
          <span className="text-[11.5px] text-muted-foreground">
            Connect a model first ·{" "}
            <button
              type="button"
              className="font-semibold text-primary hover:underline"
              onClick={() => void navigate({ to: "/connections" })}
            >
              Open Models
            </button>
          </span>
        )}
      </div>
    </div>
  );
}
