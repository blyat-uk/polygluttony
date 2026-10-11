import { useState } from "react";
import { CaretDown } from "@phosphor-icons/react";
import type { ModelOption } from "@/types/generated/ModelOption";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Spinner } from "@/components/ui/spinner";
import { cn } from "@/lib/utils";

const TRIGGER = {
  field:
    "flex h-9 w-full min-w-0 items-center justify-between gap-2 rounded-4xl border border-input bg-input/30 px-3 text-left text-sm",
  inline:
    "inline-flex h-7 max-w-full min-w-0 items-center gap-1.5 rounded-[13px] border border-input bg-[color:var(--color-raised)] px-2.5 text-[12px] hover:border-[color:var(--color-gold)]",
} as const;

function NoteTag({ children }: { children: React.ReactNode }) {
  return (
    <span className="shrink-0 rounded-[9px] border border-[color-mix(in_oklch,var(--color-gold)_45%,var(--border))] px-1.5 text-[10px] leading-4 text-primary">
      {children}
    </span>
  );
}

/**
 * Model menu: curated models (friendly label + note), then whatever the provider
 * reports live, then "Use “…”" for anything typed into the search.
 */
export function ModelPicker({
  ref,
  id,
  value,
  onChange,
  curated,
  live,
  liveLoading = false,
  sourceLabel,
  onOpenChange,
  variant = "field",
  invalid = false,
  disabled = false,
  describedBy,
  ariaLabel,
}: {
  ref?: React.Ref<HTMLButtonElement>;
  id?: string;
  value: string;
  onChange: (model: string) => void;
  curated: ModelOption[];
  live?: string[];
  liveLoading?: boolean;
  /** Who the live list comes from ("Anthropic", "this server"). */
  sourceLabel: string;
  onOpenChange?: (open: boolean) => void;
  variant?: keyof typeof TRIGGER;
  invalid?: boolean;
  disabled?: boolean;
  describedBy?: string;
  ariaLabel?: string;
}) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");

  const lower = (s: string) => s.toLowerCase();
  const current = curated.find((m) => lower(m.id) === lower(value));
  const curatedIds = new Set(curated.map((m) => lower(m.id)));
  const liveExtra = (live ?? []).filter((m) => !curatedIds.has(lower(m)));
  // A model typed earlier may be in neither list — keep it visible and selectable.
  const orphan = value && !current && !liveExtra.includes(value) ? value : null;
  const typed = search.trim();
  const typedIsNew =
    typed !== "" &&
    ![...curatedIds, ...liveExtra.map(lower), ...(orphan ? [lower(orphan)] : [])].includes(
      lower(typed),
    );

  const setOpenState = (next: boolean) => {
    setOpen(next);
    if (!next) setSearch("");
    onOpenChange?.(next);
  };
  const pick = (model: string) => {
    onChange(model);
    setOpenState(false);
  };

  return (
    <Popover open={open} onOpenChange={setOpenState}>
      <PopoverTrigger asChild>
        <button
          ref={ref}
          id={id}
          type="button"
          role="combobox"
          aria-expanded={open}
          disabled={disabled}
          aria-invalid={invalid || undefined}
          aria-describedby={describedBy}
          aria-label={ariaLabel}
          className={cn(
            TRIGGER[variant],
            "text-foreground outline-none transition-colors focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 aria-invalid:border-destructive aria-invalid:ring-[3px] aria-invalid:ring-destructive/20",
          )}
        >
          <span className="flex min-w-0 items-center gap-2">
            {current ? (
              <span className="truncate">{current.label}</span>
            ) : value ? (
              <span className="truncate font-mono text-[12px]">{value}</span>
            ) : (
              <span className="truncate text-muted-foreground">Choose a model</span>
            )}
            {variant === "field" && current?.note ? <NoteTag>{current.note}</NoteTag> : null}
          </span>
          <CaretDown className="size-3.5 shrink-0 text-muted-foreground" />
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        className={cn(
          "p-0",
          variant === "field" ? "w-[max(var(--radix-popover-trigger-width),18rem)]" : "w-80",
        )}
      >
        <Command loop>
          <CommandInput
            placeholder="Search, or type a model name…"
            value={search}
            onValueChange={setSearch}
          />
          <CommandList>
            <CommandEmpty>Type a model name above.</CommandEmpty>
            {curated.length > 0 ? (
              <CommandGroup heading="Suggested">
                {curated.map((m) => (
                  <CommandItem
                    key={m.id}
                    value={m.id}
                    keywords={[m.label, m.note ?? ""]}
                    data-checked={lower(m.id) === lower(value)}
                    onSelect={() => pick(m.id)}
                  >
                    <span className="min-w-0 flex-1">
                      <span className="flex items-center gap-2">
                        <span className="truncate">{m.label}</span>
                        {m.note ? <NoteTag>{m.note}</NoteTag> : null}
                      </span>
                      <span className="block truncate font-mono text-[10.5px] text-[color:var(--color-ink-dim)]">
                        {m.id}
                      </span>
                    </span>
                  </CommandItem>
                ))}
              </CommandGroup>
            ) : null}
            {orphan ? (
              <CommandGroup heading="Current">
                <CommandItem value={orphan} data-checked onSelect={() => pick(orphan)}>
                  <span className="truncate font-mono text-[12px]">{orphan}</span>
                </CommandItem>
              </CommandGroup>
            ) : null}
            {liveExtra.length > 0 ? (
              <CommandGroup heading={`From ${sourceLabel}`}>
                {liveExtra.map((m) => (
                  <CommandItem
                    key={m}
                    value={m}
                    data-checked={m === value}
                    onSelect={() => pick(m)}
                  >
                    <span className="truncate font-mono text-[12px]">{m}</span>
                  </CommandItem>
                ))}
              </CommandGroup>
            ) : null}
            {typedIsNew ? (
              <CommandGroup>
                <CommandItem value={`use:${typed}`} keywords={[typed]} onSelect={() => pick(typed)}>
                  <span className="min-w-0 truncate">
                    Use “<span className="font-mono text-[12px]">{typed}</span>”
                  </span>
                </CommandItem>
              </CommandGroup>
            ) : null}
          </CommandList>
          {liveLoading ? (
            <div className="flex items-center gap-2 border-t border-border px-3 py-2 text-[11.5px] text-muted-foreground">
              <Spinner className="size-3" /> Loading models from {sourceLabel}…
            </div>
          ) : null}
        </Command>
      </PopoverContent>
    </Popover>
  );
}
