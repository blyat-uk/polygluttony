import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { FileStateKind } from "@/types/generated/FileStateKind";
import { Kbd } from "./add-to-glossary";
import { termReach } from "./review";

export type Notice = { text: string; tone: "error" | "info" };

/** After a glossary change: the lines of finished episodes that use the term. */
export type TermBanner = { source: string; lines: number; episodes: number; onlyThis: boolean };

export function EpisodeFooter({
  state,
  flagged,
  blocked,
  busy,
  notice,
  banner,
  shortcut,
  onRetranslateAll,
  onMarkDone,
  onAddToGlossary,
  onRetranslateTerm,
}: {
  state: FileStateKind;
  /** Lines that need a look. */
  flagged: number;
  /** Why the actions are off, or null. */
  blocked: string | null;
  busy: boolean;
  notice: Notice | null;
  banner: TermBanner | null;
  shortcut: string;
  onRetranslateAll: () => void;
  onMarkDone: () => void;
  onAddToGlossary: (button: HTMLElement) => void;
  onRetranslateTerm: () => void;
}) {
  const off = blocked !== null || busy;
  const nothingLeft = flagged === 0;
  const needsLook = state === "warning";

  return (
    <footer className="shrink-0 border-t border-border">
      {banner ? (
        <div className="mx-4 mt-2.5 flex flex-wrap items-center gap-2.5 rounded-[8px] border border-[color:color-mix(in_oklch,var(--color-gold)_35%,transparent)] bg-[color:color-mix(in_oklch,var(--color-gold)_4%,transparent)] px-3 py-2 text-[12px]">
          <span>
            You changed <span className="text-[color:var(--color-ink-emphasis)]">{banner.source}</span> in the
            glossary.
          </span>
          <Button
            type="button"
            size="xs"
            disabled={off}
            onClick={onRetranslateTerm}
            className="h-[22px] rounded-full px-2.5 text-[11px]"
          >
            Retranslate {termReach(banner.lines, banner.episodes, banner.onlyThis)}
          </Button>
        </div>
      ) : null}

      <div className="flex items-center gap-2 px-4 pt-2 text-[11px] text-muted-foreground">
        <span>Select text in any line →</span>
        <button
          type="button"
          onMouseDown={(e) => e.preventDefault()}
          onClick={(e) => onAddToGlossary(e.currentTarget)}
          className="inline-flex h-[22px] items-center gap-1.5 rounded-full border border-[color:var(--input)] bg-[color:var(--color-raised)] px-2.5 text-foreground/85 outline-none hover:border-[color:var(--color-gold)] hover:text-[color:var(--color-gold-hi)] focus-visible:ring-[3px] focus-visible:ring-ring/50"
        >
          Add to glossary <Kbd>{shortcut}</Kbd>
        </button>
        <span>with the source and category filled in</span>
      </div>

      <div className="flex items-center gap-2.5 bg-[color:color-mix(in_oklch,var(--color-raised)_45%,transparent)] px-4 py-2.5">
        {flagged > 0 ? (
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={off}
            onClick={onRetranslateAll}
            className="h-7 rounded-full px-3 text-[12px]"
          >
            Retranslate all {flagged.toLocaleString()}
          </Button>
        ) : null}
        <span className="min-w-0 flex-1 truncate text-[11px]">
          {blocked ? (
            <span className="text-muted-foreground">{blocked}</span>
          ) : notice ? (
            <span
              className={cn(
                notice.tone === "error" ? "text-[color:var(--color-danger)]" : "text-muted-foreground",
              )}
              title={notice.text}
            >
              {notice.text}
            </span>
          ) : null}
        </span>
        {needsLook ? (
          <>
            {!nothingLeft && !blocked ? (
              <span className="text-[11px] text-muted-foreground">Mark done when nothing’s left</span>
            ) : null}
            <Button
              type="button"
              size="sm"
              // One gold button at a time: the banner's Retranslate has it while shown.
              variant={nothingLeft && !banner ? "default" : "outline"}
              disabled={off || !nothingLeft}
              onClick={onMarkDone}
              className="h-7 rounded-full px-3 text-[12px]"
            >
              Mark episode done
            </Button>
          </>
        ) : state === "done" && nothingLeft && !blocked ? (
          <span className="text-[11px] text-[color:var(--color-success)]">✓ Done</span>
        ) : null}
      </div>
    </footer>
  );
}
