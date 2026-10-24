import { useRef, useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import type { SetupSummary } from "@/types/generated/SetupSummary";
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
import { ipc } from "@/lib/ipc";
import { plural } from "@/lib/utils";
import { listNames } from "./steps";

const ZIP_FILTER = [{ name: "Polygluttony export", extensions: ["zip"] }];

/** A picked export that would replace something here, waiting on the confirm dialog. */
type PendingImport = { path: string; summary: SetupSummary };

export type SetupTransfer = {
  /** Export setup…: pick where, then write the zip. */
  exportSetup: () => Promise<void>;
  /** Import setup… and the first-run link: pick a zip, confirm if it replaces anything,
   *  apply. A replaced setup is saved first, and the toast's Undo brings it back. */
  startImport: () => Promise<void>;
  /** An export or import is in flight. */
  busy: boolean;
  /** The confirm dialog; render it once. */
  dialog: ReactNode;
};

/**
 * Carry a setup to another computer in one zip: Export setup… writes it, Import setup… reads
 * one back — asking first when there's something here to replace, and saving that first so
 * it can be undone — then refreshes every query. One flow for the header buttons and the
 * first-run link.
 */
export function useSetupTransfer(): SetupTransfer {
  const qc = useQueryClient();
  const [busy, setBusy] = useState(false);
  // `busy` lands a render late; this stops a quick second click from starting another run.
  const running = useRef(false);
  const [pending, setPending] = useState<PendingImport | null>(null);

  /** One export or import at a time. Errors are already user-facing. */
  const run = async (task: () => Promise<void>) => {
    if (running.current) return;
    running.current = true;
    setBusy(true);
    try {
      await task();
    } catch (e: unknown) {
      toast.error(String(e));
    } finally {
      running.current = false;
      setBusy(false);
    }
  };

  /** Import `path`. `undo`: it's the backup of a setup an import just replaced. */
  const apply = async (path: string, undo = false) => {
    let summary: SetupSummary;
    try {
      summary = await ipc.importSetup(path);
    } finally {
      // Everything may have changed: connections (which re-sync the shell store), prompts,
      // web search, the open folder's view — even on a failure, which may come after the
      // config was saved. Not awaited: a live model list still loading from a provider
      // would hold up the toast.
      void qc.invalidateQueries();
    }
    const description = summary.skipped.length > 0 ? skippedCopy(summary.skipped) : undefined;
    if (undo) {
      toast.success("Your previous setup is back");
      return;
    }
    const backup = summary.backup;
    toast.success(backup ? "Setup replaced" : "Setup imported", {
      description,
      // Long enough to notice and reach for Undo.
      duration: backup ? 10_000 : undefined,
      action: backup ? { label: "Undo", onClick: () => void run(() => apply(backup, true)) } : undefined,
    });
  };

  const exportSetup = () =>
    run(async () => {
      const dest = await saveDialog({
        defaultPath: `polygluttony-export-${localDate()}.zip`,
        filters: ZIP_FILTER,
      });
      if (!dest) return;
      await ipc.exportSetup(dest);
      toast.success("Setup exported", {
        description: "It holds your API keys in plain text — keep it somewhere private.",
      });
    });

  const startImport = () =>
    run(async () => {
      const path = await openDialog({ multiple: false, directory: false, filters: ZIP_FILTER });
      if (typeof path !== "string") return;
      const summary = await ipc.inspectSetup(path);
      if (summary.replaces_existing) setPending({ path, summary });
      else await apply(path);
    });

  return {
    exportSetup,
    startImport,
    busy,
    dialog: (
      <ReplaceSetupDialog
        pending={pending}
        onClose={() => setPending(null)}
        onReplace={(path) => void run(() => apply(path))}
      />
    ),
  };
}

/**
 * Confirm replacing this setup with the picked export's: what goes, what the file brings,
 * and that recent folders stay. Only asked when there's something here to replace.
 */
function ReplaceSetupDialog({
  pending,
  onClose,
  onReplace,
}: {
  /** The picked export; null = closed. */
  pending: PendingImport | null;
  onClose: () => void;
  onReplace: (path: string) => void;
}) {
  // Keep the last words on screen while the dialog animates closed.
  const [last, setLast] = useState(pending);
  if (pending && pending !== last) setLast(pending);
  const shown = pending ?? last;

  return (
    <AlertDialog
      open={pending !== null}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      {shown ? (
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Replace your setup?</AlertDialogTitle>
            <AlertDialogDescription>
              Your models, API keys, step settings, default languages and edited prompts here
              will be replaced by the ones in the file. Recent folders stay as they are.
              <span className="mt-2 block">The file holds {holdsCopy(shown.summary)}.</span>
              <span className="mt-2 block">
                Your current setup is saved first, so you can undo this.
              </span>
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => onReplace(shown.path)}>
              Replace
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      ) : null}
    </AlertDialog>
  );
}

/** "Claude Sonnet 5.5 and GPT-5, and 3 edited prompts", "Claude Sonnet 5.5 and no edited
 *  prompts", "no models and 1 edited prompt". */
function holdsCopy({ models, edited_prompts }: SetupSummary): string {
  const prompts =
    edited_prompts === 0 ? "no edited prompts" : plural(edited_prompts, "edited prompt");
  if (models.length === 0) return `no models and ${prompts}`;
  // Two lists in a row: the comma keeps the last model apart from the prompts.
  return `${listNames(models)}${models.length > 1 ? "," : ""} and ${prompts}`;
}

/** "Skipped 2 files this version doesn’t know: a, b" — the first few names, then a count. */
function skippedCopy(skipped: string[]): string {
  const SHOWN = 5;
  const names = skipped.slice(0, SHOWN).join(", ");
  const more = skipped.length > SHOWN ? ` and ${skipped.length - SHOWN} more` : "";
  return `Skipped ${plural(skipped.length, "file")} this version doesn’t know: ${names}${more}`;
}

/** Today as "2026-10-13" in local time (`toISOString` would give the UTC date). */
function localDate(): string {
  const d = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}
