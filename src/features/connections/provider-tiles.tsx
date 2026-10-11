import { useEffect, useRef } from "react";
import { Desktop, MagnifyingGlass, Plugs } from "@phosphor-icons/react";
import type { Provider } from "@/types/generated/Provider";
import type { ProviderInfo } from "@/types/generated/ProviderInfo";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

const MARK_SIZE = {
  sm: "size-7 rounded-lg text-[12px] [&_svg]:size-3.5",
  md: "size-[30px] rounded-[9px] text-[12.5px] [&_svg]:size-4",
  lg: "size-[38px] rounded-[10px] text-[15px] [&_svg]:size-5",
} as const;

/** A neutral monogram tile (no brand marks): A / O / G for cloud providers,
 *  a desktop for models on this computer, a plug for any other endpoint. */
export function ProviderMark({
  info,
  provider,
  size = "md",
}: {
  info: ProviderInfo | undefined;
  provider: Provider;
  size?: keyof typeof MARK_SIZE;
}) {
  const local = info?.local ?? (provider === "ollama" || provider === "lmstudio");
  return (
    <span
      aria-hidden
      className={cn(
        "grid shrink-0 place-items-center border border-input bg-[color:var(--color-raised)] font-bold text-[color:var(--color-gold-hi)]",
        MARK_SIZE[size],
      )}
    >
      {local ? (
        <Desktop weight="duotone" />
      ) : provider === "custom" ? (
        <Plugs weight="duotone" />
      ) : (
        (info?.label ?? provider).charAt(0).toUpperCase()
      )}
    </span>
  );
}

function CapChip({ tone, children }: { tone: "aqua" | "jade" | "muted"; children: React.ReactNode }) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-lg border px-1.5 py-px text-[10px] leading-4",
        tone === "aqua" &&
          "border-[color-mix(in_oklch,var(--color-aqua)_35%,transparent)] text-[color:var(--color-aqua)]",
        tone === "jade" &&
          "border-[color-mix(in_oklch,var(--color-jade)_35%,transparent)] text-[color:var(--color-jade)]",
        tone === "muted" && "border-input text-muted-foreground",
      )}
    >
      {children}
    </span>
  );
}

/** What a provider can do, as small chips under its name. */
function ProviderCaps({ info }: { info: ProviderInfo }) {
  if (!info.can_search && !info.can_think && !info.local) return null;
  return (
    <span className="mt-1.5 flex flex-wrap gap-1">
      {info.can_search ? (
        <CapChip tone="aqua">
          <MagnifyingGlass weight="bold" className="size-2.5" /> web search
        </CapChip>
      ) : null}
      {info.can_think ? <CapChip tone="muted">thinking</CapChip> : null}
      {info.local ? <CapChip tone="jade">free · private</CapChip> : null}
    </span>
  );
}

/** The provider picker: one real button per provider. `focus` moves keyboard focus to that
 *  tile on mount (used after "Change", so focus isn't dropped). */
export function ProviderTiles({
  providers,
  onPick,
  focus,
}: {
  providers: ProviderInfo[];
  onPick: (info: ProviderInfo) => void;
  focus?: Provider;
}) {
  const focusRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    focusRef.current?.focus();
  }, []);

  return (
    <div className="grid grid-cols-[repeat(auto-fill,minmax(186px,1fr))] gap-2">
      {providers.map((info) => (
        <button
          key={info.provider}
          ref={info.provider === focus ? focusRef : undefined}
          type="button"
          onClick={() => onPick(info)}
          className="flex items-start gap-2.5 rounded-[14px] border border-input bg-[color:var(--color-obsidian)] px-3 py-2.5 text-left outline-none transition-colors hover:border-[color:var(--color-gold)] hover:bg-[color:var(--color-bg-hover)] focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 active:translate-y-px"
        >
          <ProviderMark info={info} provider={info.provider} />
          <span className="min-w-0">
            <span className="block text-[12.5px] font-semibold text-[color:var(--color-ink-emphasis)]">
              {info.label}
            </span>
            <span className="block text-[11px] leading-snug text-muted-foreground">
              {info.tagline}
            </span>
            <ProviderCaps info={info} />
          </span>
        </button>
      ))}
    </div>
  );
}

/** After picking: the tiles collapse to this one row, with a way back. */
export function ChosenProvider({
  info,
  onChange,
  disabled = false,
}: {
  info: ProviderInfo;
  onChange: () => void;
  disabled?: boolean;
}) {
  return (
    <div className="flex items-center gap-2.5 rounded-[14px] border border-input bg-[color:var(--color-obsidian)] py-2 pr-2 pl-2.5">
      <ProviderMark info={info} provider={info.provider} />
      <div className="min-w-0">
        <div className="text-[12.5px] font-semibold text-[color:var(--color-ink-emphasis)]">
          {info.label}
        </div>
        <div className="truncate text-[11px] text-muted-foreground">{info.tagline}</div>
      </div>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="ml-auto text-primary"
        aria-label={`Change provider (now ${info.label})`}
        disabled={disabled}
        onClick={onChange}
      >
        Change
      </Button>
    </div>
  );
}
