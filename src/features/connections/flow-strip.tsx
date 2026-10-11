import type { ReactNode } from "react";
import { CaretRight } from "@phosphor-icons/react";
import { cn } from "@/lib/utils";
import { RoleBadge } from "./role-card";

export type TranslationNode =
  | { state: "connected"; label: string }
  | { state: "needs-key" }
  | { state: "none" };

export type WebSearchNode =
  | { state: "same" }
  | { state: "separate"; label: string }
  | { state: "off" };

type Tone = "gold" | "amber" | "aqua" | "off";

const NODE: Record<Tone, string> = {
  gold: "border-[color-mix(in_oklch,var(--color-gold)_55%,var(--border))] text-[color:var(--color-gold-hi)]",
  amber:
    "border-dashed border-[color-mix(in_oklch,var(--color-amber)_55%,var(--border))] text-[color:var(--color-amber)]",
  aqua: "border-[color-mix(in_oklch,var(--color-aqua)_45%,transparent)] text-[color:var(--color-aqua)]",
  off: "border-dashed border-input text-[color:var(--color-ink-dim)]",
};

/** A role node: number, dot (hollow when not working), and words. Truncates long model ids. */
function Node({ n, tone, children }: { n: 1 | 2; tone: Tone; children: ReactNode }) {
  const live = tone === "gold" || tone === "aqua";
  return (
    <span
      className={cn(
        "inline-flex max-w-full min-w-0 items-center gap-1.5 justify-self-start rounded-[10px] border bg-card py-1 pr-2.5 pl-1.5",
        NODE[tone],
      )}
    >
      <RoleBadge n={n} className="size-4 text-[9px]" />
      <span
        aria-hidden
        className={cn(
          "size-1.5 shrink-0 rounded-full",
          live ? "bg-current" : "border border-current",
        )}
      />
      <span className="truncate">{children}</span>
    </span>
  );
}

function Arrow({ className }: { className?: string }) {
  return (
    <span
      aria-hidden
      className={cn("flex items-center text-[color:var(--color-ink-dim)]", className)}
    >
      <span className="h-px w-4 bg-current" />
      <CaretRight weight="fill" className="-ml-1 size-2.5" />
    </span>
  );
}

function Output({ active, children }: { active: boolean; children: ReactNode }) {
  return (
    <span
      className={cn(
        "whitespace-nowrap",
        active ? "text-muted-foreground" : "text-[color:var(--color-ink-dim)]",
      )}
    >
      {children}
    </span>
  );
}

/**
 * What each role feeds, as a live readout:
 * `.ass subtitles ─▶ ① Translation ─▶ translated lines / ② Web search ─▶ glossary names`.
 */
export function FlowStrip({
  translation,
  webSearch,
}: {
  translation: TranslationNode;
  webSearch: WebSearchNode;
}) {
  return (
    <figure
      aria-label="What each connection does"
      className="grid grid-cols-[auto_auto_minmax(0,max-content)_auto_auto] items-center justify-center gap-x-2.5 gap-y-1.5 rounded-[14px] border border-border bg-[color:var(--color-obsidian)] px-4 py-3 text-[11.5px]"
    >
      <span className="row-span-2 font-mono text-[11px] whitespace-nowrap text-muted-foreground">
        .ass subtitles
      </span>
      <Arrow className="row-span-2" />

      {translation.state === "connected" ? (
        <Node n={1} tone="gold">
          Translation · {translation.label}
        </Node>
      ) : translation.state === "needs-key" ? (
        <Node n={1} tone="amber">
          Translation · needs a key
        </Node>
      ) : (
        <Node n={1} tone="amber">
          Translation · not set up
        </Node>
      )}
      <Arrow />
      <Output active={translation.state === "connected"}>translated lines</Output>

      {webSearch.state === "same" ? (
        <Node n={2} tone="aqua">
          Web search · same model
        </Node>
      ) : webSearch.state === "separate" ? (
        <Node n={2} tone="aqua">
          Web search · {webSearch.label}
        </Node>
      ) : (
        <Node n={2} tone="off">
          Web search · off
        </Node>
      )}
      <Arrow />
      <Output active={webSearch.state !== "off"}>glossary names</Output>
    </figure>
  );
}
