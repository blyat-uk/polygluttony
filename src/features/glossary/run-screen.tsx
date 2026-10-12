import { useState, type ReactNode } from "react";
import { Stop } from "@phosphor-icons/react";
import { toast } from "sonner";
import { ipc } from "@/lib/ipc";
import { plural } from "@/lib/utils";
import { useGlossaryRun } from "@/stores/glossary-store";
import { PageHeader } from "@/components/page-header";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { LogPanel, LogToggleButton } from "@/components/log-drawer";

/** Shared full-screen chrome for glossary runs (build ④ / import ②):
 *  header → optional phase line → bar → action row [Cancel | note | …Logs] →
 *  log drawer below the row. */
export function RunScreen({
  title,
  description,
  phaseLine,
  cancelNote,
  body,
}: {
  title: string;
  description: string;
  /** Build shows its phase label here; import has a single phase → undefined. */
  phaseLine?: ReactNode;
  cancelNote: string;
  /** Custom body (e.g. the live extraction console); replaces the default bar. */
  body?: ReactNode;
}) {
  const done = useGlossaryRun((s) => s.done);
  const total = useGlossaryRun((s) => s.total);
  const logs = useGlossaryRun((s) => s.logs);
  const [logsOpen, setLogsOpen] = useState(false);

  return (
    <div className="flex h-full flex-col">
      <PageHeader title={title} description={description} />
      <div className="flex-1 overflow-auto p-5">
        {body ?? (
          <>
            {phaseLine}
            <div className="flex items-center gap-3">
              <Progress value={total > 0 ? (done / total) * 100 : 0} className="flex-1" />
              <span className="text-[11px] text-muted-foreground tabular-nums">
                {done}/{plural(total, "batch", "batches")}
              </span>
            </div>
          </>
        )}
      </div>
      {/* Action row — Logs toggle far right, drawer expands below (Translate pattern). */}
      <div className="flex items-center gap-3 border-t border-border bg-[color:var(--popover)] px-5 py-3">
        <Button
          variant="destructive"
          onClick={() => ipc.cancelGlossaryBuild().catch((e: unknown) => toast.error(String(e)))}
        >
          <Stop className="size-4" /> Cancel
        </Button>
        <span className="text-[11px] text-muted-foreground">{cancelNote}</span>
        <span className="flex-1" />
        <LogToggleButton open={logsOpen} count={logs.length} onToggle={() => setLogsOpen((o) => !o)} />
      </div>
      {logsOpen ? <LogPanel lines={logs} /> : null}
    </div>
  );
}
