import { useRef } from "react";
import { CaretDown, Plus } from "@phosphor-icons/react";
import type { Model } from "@/types/generated/Model";
import type { ProviderInfo } from "@/types/generated/ProviderInfo";
import type { Step } from "@/types/generated/Step";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";
import { findProvider, isUsable, providerName } from "./connection-model";
import { ModelDot } from "./model-chips";
import { labelOf, type LibraryCaps } from "./per-step-model";
import { STEP_INFO, modelColor } from "./steps";

const OFF = "__off__";

/** Same highlight as the model picker's list (the theme's accent is the menu's own colour). */
const ITEM = "py-1.5 focus:bg-muted";
/** Secondary lines stay quiet on the highlighted item too. */
const NOTE = "block truncate text-[11px] text-muted-foreground!";

/**
 * A step's model, as a pill with the model's colour. The menu lists the library (web search:
 * models that can't search are disabled, plus "Off"), then "+ Add a model…".
 */
export function StepModelMenu({
  step,
  value,
  models,
  providers,
  caps,
  onPick,
  onAdd,
}: {
  step: Step;
  /** The step's model id; null = off (web search only). */
  value: string | null;
  models: readonly Model[];
  providers: ProviderInfo[];
  caps: LibraryCaps;
  onPick: (modelId: string | null) => void;
  /** "+ Add a model…": open the add form for this step; focus returns to `trigger` after. */
  onAdd: (trigger: HTMLElement) => void;
}) {
  const triggerRef = useRef<HTMLButtonElement>(null);
  // "+ Add a model…" opens the form only once the menu has let go of focus (while open it
  // traps focus, so the form couldn't take it); then the pill mustn't take it back either.
  const addPicked = useRef(false);
  const search = step === "web-search";
  const current = value === null ? undefined : models.find((m) => m.id === value);
  const name = STEP_INFO[step].name;

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          ref={triggerRef}
          type="button"
          aria-label={`Model for ${name}: ${current ? labelOf(current, providers) : "off"}`}
          className={cn(
            "inline-flex h-7 max-w-44 min-w-0 items-center gap-1.5 rounded-[13px] border border-input bg-[color:var(--color-raised)] pr-2 pl-2.5 text-[12px] text-foreground outline-none transition-colors hover:border-[color:var(--color-gold)] focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 aria-expanded:border-[color:var(--color-gold)]",
            !current && "text-muted-foreground",
          )}
        >
          {current ? (
            <ModelDot color={modelColor(current)} hollow={!isUsable(current)} />
          ) : (
            <ModelDot color="var(--color-ink-dim)" hollow />
          )}
          <span className="truncate">{current ? labelOf(current, providers) : "Off"}</span>
          <CaretDown className="size-3 shrink-0 text-muted-foreground" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        className="w-72"
        aria-label={`Model for ${name}`}
        onCloseAutoFocus={(e) => {
          if (!addPicked.current) return;
          addPicked.current = false;
          e.preventDefault();
          if (triggerRef.current) onAdd(triggerRef.current);
        }}
      >
        <DropdownMenuLabel className="py-1.5 text-[11px]">Model for {name}</DropdownMenuLabel>
        <DropdownMenuRadioGroup
          value={value ?? OFF}
          onValueChange={(v) => {
            const next = v === OFF ? null : v;
            if (next !== value) onPick(next);
          }}
        >
          {models.map((m) => {
            const cantSearch = search && caps.byId.get(m.id)?.web_search === false;
            const usable = isUsable(m);
            return (
              <DropdownMenuRadioItem key={m.id} value={m.id} disabled={cantSearch} className={ITEM}>
                <ModelDot color={modelColor(m)} hollow={!usable} />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[12.5px]">{labelOf(m, providers)}</span>
                  <span className={NOTE}>
                    {providerName(findProvider(providers, m.provider), m)}
                    {cantSearch ? (
                      " · can’t search"
                    ) : usable ? null : (
                      <span className="text-[color:var(--color-amber)]!"> · needs an API key</span>
                    )}
                  </span>
                </span>
              </DropdownMenuRadioItem>
            );
          })}
          {search ? (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuRadioItem value={OFF} className={ITEM}>
                <ModelDot color="var(--color-ink-dim)" hollow />
                <span className="min-w-0 flex-1">
                  <span className="block text-[12.5px]">Off</span>
                  <span className={NOTE}>The glossary skips the lookup</span>
                </span>
              </DropdownMenuRadioItem>
            </>
          ) : null}
        </DropdownMenuRadioGroup>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          className={cn(ITEM, "text-[12.5px] text-primary focus:text-primary")}
          onSelect={() => {
            addPicked.current = true;
          }}
        >
          <Plus weight="bold" className="size-3.5" />
          Add a model…
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
