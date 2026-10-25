import type { ReactNode } from "react";
import { CaretRight } from "@phosphor-icons/react";
import { cn } from "@/lib/utils";

/** Where web search runs. `needs-key`: its model has no API key yet, so it can't search. */
export type WebSearchNode =
  | { state: "same" }
  | { state: "separate"; label: string }
  | { state: "needs-key" }
  | { state: "off" };

type Tone = "gold" | "amber" | "aqua" | "off";

const NODE: Record<Tone, string> = {
  gold: "border-[color-mix(in_oklch,var(--color-gold)_55%,var(--border))] text-[color:var(--color-gold-hi)]",
  amber:
    "border-dashed border-[color-mix(in_oklch,var(--color-amber)_55%,var(--border))] text-[color:var(--color-amber)]",
  aqua: "border-[color-mix(in_oklch,var(--color-aqua)_45%,transparent)] text-[color:var(--color-aqua)]",
  off: "border-dashed border-input text-[color:var(--color-ink-dim)]",
};

/** A step node: dot (hollow when it can't run) and words. Truncates long model ids. */
function Node({ tone, note, children }: { tone: Tone; note?: string; children: ReactNode }) {
  const live = tone === "gold" || tone === "aqua";
  return (
    <span
      className={cn(
        "inline-flex max-w-full min-w-0 items-center gap-1.5 rounded-[10px] border bg-card py-1 pr-2.5 pl-2",
        NODE[tone],
      )}
    >
      <span
        aria-hidden
        className={cn("size-1.5 shrink-0 rounded-full", live ? "bg-current" : "border border-current")}
      />
      <span className="truncate">
        {children}
        {note ? <span className="sr-only"> ({note})</span> : null}
      </span>
    </span>
  );
}

/** "→" between two steps inside a node; read out as "then". */
function Then() {
  return (
    <>
      <span aria-hidden className="text-[color:var(--color-ink-dim)]">
        {" "}→{" "}
      </span>
      <span className="sr-only"> then </span>
    </>
  );
}

/** An arrow and what it leads to, kept together so a wrapped line starts with the arrow. */
function Hop({ children }: { children: ReactNode }) {
  return (
    <span className="inline-flex max-w-full min-w-0 items-center gap-2">
      <span aria-hidden className="flex shrink-0 items-center text-[color:var(--color-ink-dim)]">
        <span className="h-px w-3.5 bg-current" />
        <CaretRight weight="fill" className="-ml-1 size-2.5" />
      </span>
      {children}
    </span>
  );
}

const PHASE = "flex max-w-full min-w-0 flex-wrap items-center justify-center gap-x-2 gap-y-2";

const WEB_SEARCH: Record<WebSearchNode["state"], { tone: Tone; text: string }> = {
  same: { tone: "aqua", text: "same model" },
  separate: { tone: "aqua", text: "" },
  "needs-key": { tone: "amber", text: "needs a key" },
  off: { tone: "off", text: "off" },
};

/**
 * The one-model view's live readout, in workflow order:
 * `.ass subtitles ─▶ Glossary · find names → tidy up ─▶ Look up names online · <state> ─▶
 *  Translate → verify ─▶ translated lines`. Wraps between hops on narrow widths.
 */
export function FlowStrip({ usable, webSearch }: { usable: boolean; webSearch: WebSearchNode }) {
  const model: Tone = usable ? "gold" : "amber";
  const keyNote = usable ? undefined : "needs an API key";
  const ws = WEB_SEARCH[webSearch.state];
  return (
    <figure
      aria-label="What your model does, step by step"
      className="flex flex-wrap items-center justify-center gap-x-2 gap-y-2 rounded-[14px] border border-border bg-[color:var(--color-obsidian)] px-4 py-3 text-[11.5px]"
    >
      {/* Two phases: when the strip has to wrap, it breaks between them first. */}
      <span className={PHASE}>
        <span className="font-mono text-[11px] whitespace-nowrap text-muted-foreground">
          .ass subtitles
        </span>
        <Hop>
          <Node tone={model} note={keyNote}>
            Glossary · find names
            <Then />
            tidy up
          </Node>
        </Hop>
        <Hop>
          <Node tone={ws.tone}>
            Look up names online · {webSearch.state === "separate" ? webSearch.label : ws.text}
          </Node>
        </Hop>
      </span>
      <span className={PHASE}>
        <Hop>
          <Node tone={model} note={keyNote}>
            Translate
            <Then />
            verify
          </Node>
        </Hop>
        <Hop>
          <span
            className={cn(
              "whitespace-nowrap",
              usable ? "text-muted-foreground" : "text-[color:var(--color-ink-dim)]",
            )}
          >
            translated lines
          </span>
        </Hop>
      </span>
    </figure>
  );
}
