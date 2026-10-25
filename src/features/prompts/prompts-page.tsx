import { useEffect, useState } from "react";
import { useBlocker } from "@tanstack/react-router";
import { Warning } from "@phosphor-icons/react";
import { toast } from "sonner";
import type { PromptId } from "@/types/generated/PromptId";
import { PageHeader } from "@/components/page-header";
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
import { usePrompts, usePromptText, usePromptMutations } from "./use-prompts";
import { PromptList } from "./prompt-list";
import { PromptEditor } from "./prompt-editor";

export function PromptsPage() {
  const { data: prompts } = usePrompts();
  const m = usePromptMutations();
  const [selected, setSelected] = useState<PromptId | null>(null);
  const [draft, setDraft] = useState<string | null>(null);
  const [pendingSelect, setPendingSelect] = useState<PromptId | null>(null);
  const { data: loaded, error: loadError } = usePromptText(selected);

  useEffect(() => {
    if (!selected && prompts?.length) setSelected(prompts[0].id);
  }, [prompts, selected]);

  const meta = prompts?.find((p) => p.id === selected);
  const dirty = draft !== null && loaded !== undefined && draft !== loaded;

  // Leaving with unsaved edits asks first, in the app's own dialog (window.confirm does
  // nothing in WKWebView on macOS).
  const leaving = useBlocker({ shouldBlockFn: () => dirty, disabled: !dirty, withResolver: true });

  const select = (id: PromptId) => {
    if (id === selected) return;
    if (dirty) {
      setPendingSelect(id);
      return;
    }
    setSelected(id);
    setDraft(null);
  };

  return (
    <div className="flex h-full flex-col">
      <PageHeader
        title="Prompts"
        description="Customize the prompts sent to the AI. Restore any prompt to its default at any time."
      />
      <div className="flex min-h-0 flex-1">
        <PromptList prompts={prompts} selected={selected} onSelect={select} />
        {meta && loadError ? (
          <div className="flex min-w-0 flex-1 flex-col items-center justify-center gap-3 p-6">
            <Warning className="size-8 text-[color:var(--color-alert)]" />
            <p className="max-w-sm text-center text-[12.5px] text-[color:var(--color-alert)]">
              {String(loadError)}
            </p>
            <Button
              variant="outline"
              size="sm"
              onClick={async () => {
                try {
                  await m.reset.mutateAsync(meta.id);
                  setDraft(null);
                  toast.success(`"${meta.name}" restored to default`);
                } catch (e) {
                  toast.error(String(e));
                }
              }}
            >
              Restore default
            </Button>
          </div>
        ) : meta && loaded !== undefined ? (
          <PromptEditor
            key={meta.id}
            meta={meta}
            loaded={loaded}
            draft={draft}
            onDraftChange={setDraft}
            saving={m.save.isPending}
            onSave={async (text) => {
              try {
                await m.save.mutateAsync({ id: meta.id, text });
                setDraft(null);
                toast.success(`Saved "${meta.name}"`);
              } catch (e) {
                toast.error(String(e));
              }
            }}
            onDiscard={() => setDraft(null)}
            onReset={async () => {
              // At once, with Undo: the edited text saved back.
              const previous = loaded;
              const { id, name } = meta;
              try {
                await m.reset.mutateAsync(id);
                setDraft(null);
              } catch (e) {
                toast.error(String(e));
                return;
              }
              toast.success(`Restored "${name}"`, {
                duration: 10_000,
                action: {
                  label: "Undo",
                  onClick: () =>
                    void m.save
                      .mutateAsync({ id, text: previous })
                      .then(() => toast.success(`Your "${name}" edit is back`))
                      .catch((e: unknown) => toast.error(String(e))),
                },
              });
            }}
          />
        ) : null}
      </div>

      <AlertDialog
        open={pendingSelect !== null}
        onOpenChange={(open) => {
          if (!open) setPendingSelect(null);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Discard unsaved changes?</AlertDialogTitle>
            <AlertDialogDescription>
              &ldquo;{meta?.name}&rdquo; has edits you haven&apos;t saved. Switching prompts will
              discard them.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep editing</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                if (pendingSelect) {
                  setSelected(pendingSelect);
                  setDraft(null);
                  setPendingSelect(null);
                }
              }}
            >
              Discard changes
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog
        open={leaving.status === "blocked"}
        onOpenChange={(open) => {
          if (!open) leaving.reset?.();
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Leave without saving?</AlertDialogTitle>
            <AlertDialogDescription>
              &ldquo;{meta?.name}&rdquo; has edits you haven&apos;t saved. Leaving discards them.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep editing</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => leaving.proceed?.()}>
              Discard and leave
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
