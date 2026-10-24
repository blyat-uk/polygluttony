import { useState } from "react";
import { toast } from "sonner";
import { WarningCircle } from "@phosphor-icons/react";
import { ipc } from "@/lib/ipc";
import { usePrimaryAction } from "@/hooks/use-primary-action";
import { Button } from "@/components/ui/button";
import { PageHeader } from "@/components/page-header";
import { RestoreDialog } from "./restore-dialog";

/** glossary.json exists but can't be read. Never offer to build over it. */
export function UnreadableView({
  folder,
  message,
  line,
}: {
  folder: string;
  message: string;
  line: number | null;
}) {
  const [restoreOpen, setRestoreOpen] = useState(false);
  const openInEditor = () =>
    ipc.openGlossaryEditor(folder).catch((e: unknown) => toast.error(String(e)));
  usePrimaryAction(() => void openInEditor(), true);

  return (
    <div className="flex h-full flex-col">
      <PageHeader title="Glossary" />
      <div className="flex flex-1 flex-col items-center justify-center gap-4 p-10 text-center">
        <WarningCircle weight="fill" className="size-9 text-[color:var(--color-danger)]" />
        <div className="flex max-w-md flex-col gap-1.5">
          <h2 className="text-base font-semibold text-foreground">
            glossary.json has a mistake{line !== null ? ` on line ${line}` : ""}.
          </h2>
          <p className="text-[13px] text-muted-foreground">
            PG can’t read it, so it won’t change it. Fix it in your text editor and this page
            reloads by itself, or go back to a copy from before.
          </p>
          <p className="mt-1 font-mono text-[11.5px] break-words text-muted-foreground">{message}</p>
        </div>
        <div className="flex items-center gap-2">
          <Button onClick={() => void openInEditor()}>Open in your text editor</Button>
          <Button variant="outline" onClick={() => setRestoreOpen(true)}>
            Restore an earlier version…
          </Button>
        </div>
      </div>
      <RestoreDialog folder={folder} open={restoreOpen} onOpenChange={setRestoreOpen} />
    </div>
  );
}
