import { useState, type ReactNode } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Check, Translate } from "@phosphor-icons/react";
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
import { Button } from "@/components/ui/button";
import { RecentItem } from "@/components/recent-item";
import { folderTitle } from "@/lib/folder-title";
import { cn } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";
import { modelById } from "@/features/connections/steps";
import { useModelTest } from "@/features/connections/test-status";
import { useConnectionsConfig } from "@/features/connections/use-connections";
import { useRecents, useRecentMutations } from "./use-recents";
import { usePickFolder } from "./use-pick-folder";

/**
 * Before a folder is open: two steps where the gold button is always the next one —
 * Connect until a model can translate, then Open folder — and the recent folders, which
 * show whether or not a model is ready yet.
 */
export function WelcomePage() {
  const navigate = useNavigate();
  const connected = useAppStore((s) => s.hasUsableConnection);
  const modelLabel = useAppStore((s) => s.translationLabel);
  const config = useConnectionsConfig();
  const main = config.data ? modelById(config.data, config.data.steps?.translate.model) : undefined;
  const { test } = useModelTest(main);
  const { data: recents } = useRecents();
  const recentM = useRecentMutations();
  // A folder with no subtitles stays here; say so under the steps rather than in a toast.
  const [emptyPath, setEmptyPath] = useState<string | null>(null);
  const folder = usePickFolder({ onEmpty: setEmptyPath });
  const [confirmClear, setConfirmClear] = useState(false);

  const openPath = (path: string) => {
    setEmptyPath(null);
    folder.openPath(path);
  };
  const pick = () => {
    setEmptyPath(null);
    void folder.pick();
  };

  return (
    <div className="flex min-h-full flex-col items-center justify-center gap-6 p-10">
      <div className="flex flex-col items-center gap-1 text-center">
        <Translate weight="fill" className="size-10 text-primary" />
        <h1 className="text-[22px] font-semibold text-foreground">Polygluttony</h1>
        <p className="text-sm text-muted-foreground">
          Translate .ass subtitles with the AI you connect.
        </p>
      </div>

      <div className="w-full max-w-xl space-y-2">
        {connected ? (
          <Step
            done
            title="Model connected"
            hint={[modelLabel, test.state === "works" ? "Works" : "Ready"].filter(Boolean).join(" · ")}
          >
            <Button variant="ghost" onClick={() => void navigate({ to: "/connections" })}>
              Change
            </Button>
          </Step>
        ) : (
          <Step
            n={1}
            next
            title="Connect an AI model"
            hint="Anthropic, OpenAI, Google, or a model on your computer (Ollama, LM Studio)."
          >
            <Button onClick={() => void navigate({ to: "/connections" })}>Connect</Button>
          </Step>
        )}
        <Step
          n={2}
          next={connected}
          title="Open a folder of subtitles"
          hint="Or drag a folder onto the window."
        >
          <Button
            variant={connected ? "default" : "outline"}
            disabled={folder.pending}
            onClick={pick}
          >
            Open folder
          </Button>
        </Step>
        {emptyPath ? (
          <p className="pt-1 text-center text-[12px] text-[color:var(--color-alert)]">
            No subtitle files in {folderTitle(emptyPath)}. Pick the folder that holds the .ass
            files.
          </p>
        ) : null}
      </div>

      {recents && recents.length > 0 ? (
        <div className="w-full max-w-xl space-y-2">
          <div className="flex items-baseline justify-between gap-3">
            <h2 className="text-[13px] font-semibold text-muted-foreground">Recent folders</h2>
            <button
              type="button"
              onClick={() => setConfirmClear(true)}
              className="rounded-md text-[11px] text-muted-foreground outline-none hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50"
            >
              Clear all
            </button>
          </div>
          {!connected ? (
            <p className="text-[11.5px] text-muted-foreground">
              Connect a model first to build glossaries and translate in these.
            </p>
          ) : null}
          <div className="space-y-1.5">
            {recents.map((r) => (
              <RecentItem
                key={r.path}
                recent={r}
                onOpen={() => openPath(r.path)}
                onRemove={() => recentM.remove.mutate(r.path)}
              />
            ))}
          </div>
        </div>
      ) : null}

      <AlertDialog open={confirmClear} onOpenChange={setConfirmClear}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Clear all recent folders?</AlertDialogTitle>
            <AlertDialogDescription>
              They leave this list only. The folders, their subtitles and glossaries stay where
              they are.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep them</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => recentM.clear.mutate()}>
              Clear all
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

/** One first-run step: a number (✓ once done), what to do, and its button. `next` marks
 *  the step the gold button belongs to. */
function Step({
  n,
  done = false,
  next = false,
  title,
  hint,
  children,
}: {
  n?: number;
  done?: boolean;
  next?: boolean;
  title: string;
  hint: string;
  children: ReactNode;
}) {
  return (
    <div
      className={cn(
        "flex items-center gap-3 rounded-2xl border bg-[color:var(--card)] px-4 py-3",
        next ? "border-[color:var(--color-gold-deep)]" : "border-border",
      )}
    >
      <span
        className={cn(
          "flex size-6 shrink-0 items-center justify-center rounded-full border text-[12px]",
          done
            ? "border-[color:var(--color-jade)] bg-[color-mix(in_oklch,var(--color-jade)_14%,transparent)] text-[color:var(--color-jade)]"
            : next
              ? "border-[color:var(--color-gold)] text-primary"
              : "border-border text-muted-foreground",
        )}
      >
        {done ? <Check weight="bold" className="size-3.5" aria-label="Done" /> : n}
      </span>
      <div className="min-w-0 flex-1">
        <div className="text-[13px] font-medium text-foreground">{title}</div>
        <div className="truncate text-[11px] text-muted-foreground">{hint}</div>
      </div>
      {children}
    </div>
  );
}
