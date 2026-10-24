import { useEffect, useState } from "react";
import type { ProjectView } from "@/types/generated/ProjectView";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { plural } from "@/lib/utils";
import { useGlossaryRun } from "@/stores/glossary-store";
import { EpisodeChecklist } from "./episode-checklist";
import { episodes } from "./episodes";
import { TidyOption } from "./glossary-build-options";
import { startBuild } from "./start-build";

/** "Rebuild from scratch…": throw the terms away and scan the picked episodes. */
export function RebuildDialog({
  view,
  count,
  open,
  onOpenChange,
}: {
  view: ProjectView;
  count: number;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const busy = useGlossaryRun((s) => s.busy);
  const [selected, setSelected] = useState<string[]>([]);
  const [tidy, setTidy] = useState(true);

  useEffect(() => {
    if (!open) return;
    setSelected(view.files.map((f) => f.name));
    setTidy(true);
  }, [open, view.files]);

  const run = () => {
    onOpenChange(false);
    startBuild(view, { kind: "rebuild", files: selected, tidy });
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[85vh] flex-col gap-4 sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Rebuild from scratch?</DialogTitle>
          <DialogDescription>
            Throws away the current {plural(count, "term")} and builds a new glossary from the
            episodes you pick. A copy is saved first; bring it back with Restore an earlier
            version…
          </DialogDescription>
        </DialogHeader>
        <EpisodeChecklist
          files={view.files}
          selected={selected}
          onChange={setSelected}
          className="min-h-0 flex-1"
        />
        <TidyOption checked={tidy} onChange={setTidy} />
        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            variant="destructive"
            disabled={selected.length === 0 || busy !== null}
            onClick={run}
          >
            Rebuild from {episodes(selected.length)}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
