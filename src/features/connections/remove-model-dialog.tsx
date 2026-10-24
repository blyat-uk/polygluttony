import { useState } from "react";
import { toast } from "sonner";
import type { ConnectionsConfig } from "@/types/generated/ConnectionsConfig";
import type { Model } from "@/types/generated/Model";
import type { ProviderInfo } from "@/types/generated/ProviderInfo";
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
import {
  forgottenKeys,
  forgottenKeysSentence,
  labelOf,
  removalOutcome,
  useLibraryCapabilities,
  type LibraryCaps,
} from "./per-step-model";
import { stepNames } from "./steps";
import { useConnectionMutations } from "./use-connections";

/**
 * Confirm removing a library model, saying exactly what happens next: which steps move to
 * which model, whether web search turns off, and which saved key is forgotten. Removing the
 * last model goes back to setup (the page shows first-run once the library is empty).
 */
export function RemoveModelDialog({
  model,
  config,
  providers,
  onClose,
  onRemoved,
}: {
  /** The model to remove; null = closed. */
  model: Model | null;
  config: ConnectionsConfig;
  providers: ProviderInfo[];
  onClose: () => void;
  /** After the backend removed it (e.g. close the edit form it was opened from). */
  onRemoved?: (cfg: ConnectionsConfig) => void;
}) {
  const { removeModel } = useConnectionMutations();
  const caps = useLibraryCapabilities(config.models);
  // Keep the last words on screen while the dialog animates closed (by then the model may
  // already be gone from `config`).
  const live = model ? { model, copy: removalCopy(model, config, providers, caps) } : null;
  const [last, setLast] = useState(live);
  if (live && (live.model !== last?.model || live.copy !== last?.copy)) setLast(live);
  const shown = live ?? last;

  const remove = (target: Model) => {
    const label = labelOf(target, providers);
    removeModel.mutate(target.id, {
      onSuccess: (cfg) => {
        toast.success(`${label} removed`);
        onRemoved?.(cfg);
      },
      onError: (e) => toast.error(`Couldn’t remove ${label}: ${String(e)}`),
    });
  };

  return (
    <AlertDialog
      open={model !== null}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      {shown ? (
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove {labelOf(shown.model, providers)}?</AlertDialogTitle>
            <AlertDialogDescription>{shown.copy}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep it</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => remove(shown.model)}>
              Remove
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      ) : null}
    </AlertDialog>
  );
}

/** "Find names and Translate will use Claude Haiku 5.5 instead. Look up names online will turn off.
 *  Your saved Anthropic API key will be forgotten too." */
function removalCopy(
  model: Model,
  cfg: ConnectionsConfig,
  providers: ProviderInfo[],
  caps: LibraryCaps,
): string {
  const remaining = cfg.models.filter((m) => m.id !== model.id);
  const keyNote = forgottenKeysSentence(forgottenKeys([model], remaining, cfg, providers));
  const outcome = removalOutcome(cfg, model.id, caps);

  const parts: string[] = [];
  if (outcome.kind === "last") {
    parts.push(
      "It’s your only model, so you’ll be back to setup: nothing can build a glossary or translate until you connect one again.",
    );
  } else {
    const { fallback, moved, everyStep, webSearch } = outcome;
    if (moved.length > 0) {
      const who = everyStep ? "Every step" : stepNames(moved);
      parts.push(`${who} will use ${labelOf(fallback, providers)} instead.`);
    }
    if (webSearch === "off") {
      // Until every model's capabilities are in, "none can search" isn't known yet.
      parts.push(
        caps.settled
          ? "Look up names online will turn off."
          : "Look up names online will move to a model that can search the web, or turn off.",
      );
    } else if (webSearch !== null) {
      parts.push(`Look up names online will use ${labelOf(webSearch.to, providers)} instead.`);
    }
    if (moved.length === 0 && webSearch === null) parts.push("No step uses it.");
  }
  if (keyNote) parts.push(keyNote);
  return parts.join(" ");
}
