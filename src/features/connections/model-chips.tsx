import { useId, type Ref } from "react";
import { Plus } from "@phosphor-icons/react";
import type { Model } from "@/types/generated/Model";
import type { ProviderInfo } from "@/types/generated/ProviderInfo";
import { cn } from "@/lib/utils";
import { findProvider, isUsable, providerName } from "./connection-model";
import { labelOf } from "./per-step-model";
import { modelColor } from "./steps";

/** A model's colour, the same everywhere it appears. Hollow = not working (no key) or off. */
export function ModelDot({
  color,
  hollow = false,
  className,
}: {
  color: string;
  hollow?: boolean;
  className?: string;
}) {
  return (
    <span
      aria-hidden
      className={cn("size-2 shrink-0 rounded-full", hollow && "border-[1.5px]", className)}
      style={hollow ? { borderColor: color } : { backgroundColor: color }}
    />
  );
}

/** The amber ring that marks a model without its API key (chips, menus). */
export function NeedsKeyMark() {
  return (
    <>
      <span
        aria-hidden
        className="size-[7px] shrink-0 rounded-full border border-[color:var(--color-amber)]"
      />
      <span className="sr-only">, needs an API key</span>
    </>
  );
}

const CHIP =
  "inline-flex h-7 max-w-full min-w-0 items-center gap-2 rounded-[14px] border text-[12px] outline-none transition-colors focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50";

/**
 * "Your models": one chip per library model (its colour, its name). A chip opens that model's
 * edit form under the row; "+ Add" opens the empty form. The open one stays highlighted, and
 * clicking it again closes the form.
 */
export function ModelChips({
  models,
  providers,
  openId,
  adding,
  formId,
  addRef,
  onEdit,
  onAdd,
  onClose,
}: {
  models: readonly Model[];
  providers: ProviderInfo[];
  /** The model whose edit form is open. */
  openId: string | null;
  /** The add form is open (from "+ Add", not from a step's menu). */
  adding: boolean;
  /** The form card's id, for aria-controls. */
  formId: string;
  addRef: Ref<HTMLButtonElement>;
  onEdit: (id: string, trigger: HTMLElement) => void;
  onAdd: (trigger: HTMLElement) => void;
  /** A second click on the open chip closes its form. */
  onClose: () => void;
}) {
  const labelId = useId();
  const labels = models.map((m) => labelOf(m, providers));
  return (
    <div role="group" aria-labelledby={labelId} className="flex flex-wrap items-center gap-1.5">
      <span
        id={labelId}
        className="mr-1 text-[10.5px] font-semibold tracking-[0.1em] text-muted-foreground uppercase"
      >
        Your models
      </span>
      {models.map((m, i) => {
        const open = openId === m.id;
        const provider = providerName(findProvider(providers, m.provider), m);
        // Two models with the same name (say, one model on two servers): add where each lives.
        const twin = labels.some((l, j) => j !== i && l === labels[i]);
        return (
          <button
            key={m.id}
            type="button"
            aria-expanded={open}
            aria-controls={open ? formId : undefined}
            title={`${labels[i]} · ${provider}${isUsable(m) ? "" : " · needs an API key"}`}
            onClick={(e) => (open ? onClose() : onEdit(m.id, e.currentTarget))}
            className={cn(
              CHIP,
              "bg-card pr-3 pl-2.5 text-foreground",
              open
                ? "border-[color-mix(in_oklch,var(--color-gold)_55%,var(--border))] bg-[color:var(--color-raised)]"
                : "border-input hover:border-[color:var(--color-gold)] hover:bg-[color:var(--color-bg-hover)]",
            )}
          >
            <ModelDot color={modelColor(m)} />
            <span className="truncate">
              <span className="sr-only">Edit </span>
              {labels[i]}
            </span>
            {twin ? (
              <span className="shrink-0 text-[11px] text-[color:var(--color-ink-dim)]">{provider}</span>
            ) : null}
            {isUsable(m) ? null : <NeedsKeyMark />}
          </button>
        );
      })}
      <button
        ref={addRef}
        type="button"
        aria-expanded={adding}
        aria-controls={adding ? formId : undefined}
        onClick={(e) => (adding ? onClose() : onAdd(e.currentTarget))}
        className={cn(
          CHIP,
          "border-dashed pr-3 pl-2.5 text-primary",
          adding
            ? "border-[color:var(--color-gold)] bg-[color:var(--color-raised)]"
            : "border-[color-mix(in_oklch,var(--color-gold)_45%,var(--border))] bg-transparent hover:border-[color:var(--color-gold)] hover:bg-[color-mix(in_oklch,var(--color-gold)_8%,transparent)]",
        )}
      >
        <Plus weight="bold" className="size-3" />
        Add<span className="sr-only"> a model</span>
      </button>
    </div>
  );
}
