import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import type { GlossaryMeta } from "@/types/generated/GlossaryMeta";
import type { ProjectView } from "@/types/generated/ProjectView";
import { Button } from "@/components/ui/button";
import { HelpText } from "@/components/help-text";
import { usePrimaryAction } from "@/hooks/use-primary-action";
import { useGlossaryRun } from "@/stores/glossary-store";
import { projectKey } from "@/features/project/use-project";
import { defaultAddSelection, episodes } from "./episodes";
import { EpisodeChecklist } from "./episode-checklist";
import { TidyOption } from "./glossary-build-options";
import { PanelLabel, SidePanel } from "./side-panel";
import { startBuild } from "./start-build";
import { titleCase } from "./model";

/**
 * "Add new episodes": scan picked episodes and add only the terms the
 * glossary doesn't have yet. Existing terms keep their translations; there's
 * no lookup here (Improve → Look up names online covers that, with review).
 */
export function AddEpisodesPanel({
  view,
  meta,
  preset,
  onClose,
}: {
  view: ProjectView;
  meta: GlossaryMeta;
  /** Episodes asked for by the Episodes page. */
  preset: string[] | null;
  onClose: () => void;
}) {
  const busy = useGlossaryRun((s) => s.busy);
  const [initial] = useState(() => defaultAddSelection(view.files, meta.scanned_files, preset));
  const [selected, setSelected] = useState<string[]>(initial.selected);
  const [tidy, setTidy] = useState(true);
  const scanned = new Set(meta.scanned_files);
  const canRun = selected.length > 0 && busy === null;

  // A file just copied into the folder shows up without a Rescan: read the
  // folder again on open, and tick what that adds the way the panel would have.
  const qc = useQueryClient();
  useEffect(() => {
    void qc.invalidateQueries({ queryKey: projectKey(view.folder) });
  }, [qc, view.folder]);
  const known = useRef(new Set(view.files.map((f) => f.name)));
  useEffect(() => {
    const added = view.files.filter((f) => !known.current.has(f.name));
    if (added.length === 0) return;
    for (const f of added) known.current.add(f.name);
    if (preset) return;
    const tick = defaultAddSelection(added, meta.scanned_files).selected;
    if (tick.length > 0) setSelected((sel) => [...sel, ...tick.filter((n) => !sel.includes(n))]);
  }, [view.files, preset, meta.scanned_files]);

  const run = () => {
    if (!canRun) return;
    startBuild(view, { kind: "scan", files: selected, tidy });
  };
  usePrimaryAction(run, canRun);

  return (
    <SidePanel
      title="Add new episodes"
      onClose={onClose}
      footer={
        <>
          <span className="mr-auto text-[11.5px] text-muted-foreground">
            World type: {titleCase(view.prefs.world_override ?? view.detected_world)}
          </span>
          <Button disabled={!canRun} onClick={run}>
            Scan {episodes(selected.length)}
          </Button>
        </>
      }
    >
      <p className="mb-4 text-[12.5px] text-muted-foreground">
        PG reads these episodes and adds the names it hasn’t seen yet. The terms you have keep
        their translations.
      </p>
      <PanelLabel>Episodes</PanelLabel>
      <EpisodeChecklist
        files={view.files}
        selected={selected}
        onChange={setSelected}
        tag={(f) => (scanned.has(f.name) ? "in glossary" : null)}
      />
      {initial.unknownHistory && !preset ? (
        <HelpText>
          PG doesn’t know yet which episodes built this glossary, so it ticked the ones that
          aren’t translated.
        </HelpText>
      ) : null}
      <div className="mt-5">
        <TidyOption
          checked={tidy}
          onChange={setTidy}
          label="Tidy up the new terms"
          help="Fixes inconsistent spellings in what the scan finds and drops terms too generic to be useful."
        />
      </div>
    </SidePanel>
  );
}
