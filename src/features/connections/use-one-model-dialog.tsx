import { useId, useState } from "react";
import { Check } from "@phosphor-icons/react";
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
import { cn } from "@/lib/utils";
import { findProvider, providerName } from "./connection-model";
import { ModelDot } from "./model-chips";
import {
  forgottenKeysSentence,
  labelOf,
  listNames,
  oneModelOutcome,
  type LibraryCaps,
} from "./per-step-model";
import { modelColor, stepNames, stepsUsing } from "./steps";
import { useConnectionMutations } from "./use-connections";

/**
 * "Use one model for everything": pick the model (Translate's, to start), read what will
 * happen — which models leave the list, which saved keys go — then confirm.
 */
export function UseOneModelDialog({
  open,
  onOpenChange,
  config,
  providers,
  caps,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  config: ConnectionsConfig;
  providers: ProviderInfo[];
  caps: LibraryCaps;
}) {
  const uid = useId();
  const { useOneModel } = useConnectionMutations();
  const translateModel = config.steps?.translate.model ?? config.models[0]?.id ?? "";
  const [choice, setChoice] = useState(translateModel);
  // Each time it opens, start from Translate's model again.
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) setChoice(translateModel);
  }
  const chosen = config.models.find((m) => m.id === choice) ?? config.models[0];

  const confirm = (model: Model) => {
    const label = labelOf(model, providers);
    useOneModel.mutate(model.id, {
      onSuccess: () => toast.success(`Every step uses ${label} now`),
      onError: (e) => toast.error(`Couldn’t switch to one model: ${String(e)}`),
    });
  };

  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      {chosen ? (
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Use one model for everything?</AlertDialogTitle>
          </AlertDialogHeader>

          <fieldset className="min-w-0">
            <legend className="mb-2 text-[12px] font-medium">Which model?</legend>
            <div className="flex max-h-72 flex-col gap-1.5 overflow-y-auto">
              {config.models.map((m) => {
                const checked = m.id === chosen.id;
                const used = stepsUsing(config, m.id);
                return (
                  <label
                    key={m.id}
                    className={cn(
                      "flex cursor-pointer items-center gap-2.5 rounded-xl border px-3 py-2 transition-colors has-[input:focus-visible]:ring-[3px] has-[input:focus-visible]:ring-ring/50",
                      checked
                        ? "border-[color-mix(in_oklch,var(--color-gold)_55%,var(--border))] bg-[color:var(--color-raised)]"
                        : "border-input hover:border-[color:var(--color-gold)]",
                    )}
                  >
                    <input
                      type="radio"
                      name={`${uid}-model`}
                      value={m.id}
                      checked={checked}
                      onChange={() => setChoice(m.id)}
                      className="sr-only"
                    />
                    <ModelDot color={modelColor(m)} />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-[12.5px] font-medium text-foreground">
                        {labelOf(m, providers)}
                      </span>
                      <span className="block truncate text-[11px] text-muted-foreground">
                        {providerName(findProvider(providers, m.provider), m)} ·{" "}
                        {used.length > 0 ? stepNames(used) : "no step yet"}
                      </span>
                    </span>
                    <Check
                      aria-hidden
                      weight="bold"
                      className={cn("size-3.5 shrink-0 text-primary", !checked && "invisible")}
                    />
                  </label>
                );
              })}
            </div>
          </fieldset>

          {/* Below the choice, so it reads as its result; announced as the choice changes. */}
          <AlertDialogDescription aria-live="polite" className="-mt-2 text-[12.5px] leading-5">
            {outcomeCopy(config, chosen, caps, providers)}
          </AlertDialogDescription>

          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction className="max-w-full min-w-0" onClick={() => confirm(chosen)}>
              <span className="truncate">Use {labelOf(chosen, providers)}</span>
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      ) : null}
    </AlertDialog>
  );
}

/** "Every step will use Claude Sonnet 5.5 with its current settings. Models no step uses are
 *  removed from the list: Claude Opus 5.5 and Claude Haiku 5.5. Your saved Google API key will
 *  be forgotten too." */
function outcomeCopy(
  cfg: ConnectionsConfig,
  chosen: Model,
  caps: LibraryCaps,
  providers: ProviderInfo[],
): string {
  const o = oneModelOutcome(cfg, chosen, caps, providers);
  const label = labelOf(chosen, providers);
  const settings = o.keepsSettings ? "with its current settings" : "with recommended settings";
  const parts = [
    o.webSearchStaysOn
      ? `Every step but Web search will use ${label} ${settings}; Web search stays on ${labelOf(o.webSearchStaysOn, providers)}.`
      : `Every step will use ${label} ${settings}.`,
  ];
  if (o.removed.length > 0) {
    const names = listNames(o.removed.map((m) => labelOf(m, providers)));
    parts.push(`Models no step uses are removed from the list: ${names}.`);
  }
  const keys = forgottenKeysSentence(o.forgotten);
  if (keys) parts.push(keys);
  return parts.join(" ");
}
