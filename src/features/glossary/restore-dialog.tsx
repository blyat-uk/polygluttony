import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import type { GlossaryChangeKind } from "@/types/generated/GlossaryChangeKind";
import type { GlossarySnapshot } from "@/types/generated/GlossarySnapshot";
import { ipc } from "@/lib/ipc";
import { formatRelativeTime } from "@/lib/relative-time";
import { plural } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { glossaryKey, snapshotsKey } from "./keys";

/** Why a snapshot was taken, as the list shows it. */
export const SNAPSHOT_REASON: Record<GlossaryChangeKind, string> = {
  scan: "Before Add new episodes",
  rebuild: "Before Rebuild from scratch",
  tidy_up: "Before Tidy up",
  look_up: "Before Look up names online",
  import: "Before Import",
  restore: "Before a restore",
  replace: "Before Replace in translations",
  bulk_edit: "Before a bulk edit",
};

function when(atMs: number): { relative: string; exact: string } {
  const d = new Date(atMs);
  return {
    relative: formatRelativeTime(Math.floor(atMs / 1000)),
    exact: d.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" }),
  };
}

/** "Restore an earlier version…": the glossary's history, newest first. */
export function RestoreDialog({
  folder,
  open,
  onOpenChange,
}: {
  folder: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const qc = useQueryClient();
  const [restoring, setRestoring] = useState<string | null>(null);
  const { data, isPending, error } = useQuery({
    queryKey: snapshotsKey(folder),
    queryFn: () => ipc.listGlossarySnapshots(folder),
    enabled: open,
    staleTime: 0,
  });

  const restore = async (snap: GlossarySnapshot) => {
    setRestoring(snap.id);
    try {
      await ipc.restoreGlossarySnapshot(folder, snap.id);
      await qc.invalidateQueries({ queryKey: glossaryKey(folder) });
      void qc.invalidateQueries({ queryKey: snapshotsKey(folder) });
      onOpenChange(false);
      toast.success(
        `Restored the version from ${when(snap.at_ms).exact} (${plural(snap.count, "term")}). The one it replaced is in the list too.`,
      );
    } catch (e: unknown) {
      toast.error(`Couldn't restore that version. ${String(e)}`);
    } finally {
      setRestoring(null);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[80vh] flex-col gap-4 sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Restore an earlier version</DialogTitle>
          <DialogDescription>
            PG saves a copy of glossary.json before every bulk change. Restoring one keeps a copy
            of what you have now, so you can come back to it.
          </DialogDescription>
        </DialogHeader>
        <div className="min-h-20 flex-1 overflow-auto rounded-xl border border-border bg-[color:var(--card)]">
          {isPending ? (
            <p className="p-4 text-[12.5px] text-muted-foreground">Loading…</p>
          ) : error ? (
            <p className="p-4 text-[12.5px] text-[color:var(--color-danger)]">
              Couldn't read the history. {String(error)}
            </p>
          ) : !data?.length ? (
            <p className="p-4 text-[12.5px] text-muted-foreground">
              No earlier versions yet. A copy is saved before Tidy up, Look up names online, Add
              new episodes, Rebuild, Replace in translations, Import and bulk edits.
            </p>
          ) : (
            <ul className="divide-y divide-border/60">
              {data.map((snap) => {
                const t = when(snap.at_ms);
                return (
                  <li key={snap.id} className="flex items-center gap-3 px-4 py-2.5">
                    <div className="min-w-0 flex-1">
                      <div className="text-[13px] text-foreground">{SNAPSHOT_REASON[snap.reason]}</div>
                      <div className="text-[11.5px] tabular-nums text-muted-foreground" title={t.exact}>
                        {t.relative} · {t.exact} · {plural(snap.count, "term")}
                      </div>
                    </div>
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={restoring !== null}
                      onClick={() => void restore(snap)}
                    >
                      {restoring === snap.id ? "Restoring…" : "Restore"}
                    </Button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            Close
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
