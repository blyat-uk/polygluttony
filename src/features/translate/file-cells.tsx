import type { FileStateKind } from "@/types/generated/FileStateKind";
import type { VerifyIssue } from "@/types/generated/VerifyIssue";
import { cn } from "@/lib/utils";

export type CellGlyph = "none" | "spin" | "check" | "warn" | "fail";

/**
 * Pure mapping from pipeline state to the two status cells.
 * Cleanup counts as translating; retranslation re-spins Translated and resets
 * Verified; `reachedVerify` decides which column owns a failure's ✗.
 */
export function stateToCells(
  state: FileStateKind,
  reachedVerify: boolean,
): { translated: CellGlyph; verified: CellGlyph } {
  switch (state) {
    case "pending":
      return { translated: "none", verified: "none" };
    case "translating":
    case "retranslating":
    case "cleanup":
      return { translated: "spin", verified: "none" };
    case "verifying":
      return { translated: "check", verified: "spin" };
    case "done":
      return { translated: "check", verified: "check" };
    case "warning":
      return { translated: "check", verified: "warn" };
    case "failed":
      return reachedVerify
        ? { translated: "check", verified: "fail" }
        : { translated: "fail", verified: "none" };
    case "stopped":
      return { translated: "none", verified: "none" };
  }
}

export function StatusGlyph({ kind, tone }: { kind: CellGlyph; tone: "translate" | "verify" }) {
  switch (kind) {
    case "none":
      return <span className="text-muted-foreground/40">—</span>;
    case "spin":
      return (
        <span
          className="inline-block size-[13px] animate-spin rounded-full border-2 border-border align-[-2px]"
          style={{
            borderTopColor: tone === "verify" ? "var(--color-state-verify)" : "var(--color-gold)",
          }}
        />
      );
    case "check":
      return <span className="text-[15px] font-bold text-[color:var(--color-success)]">✓</span>;
    case "warn":
      return <span className="text-[14px] font-bold text-[color:var(--color-alert)]">⚠</span>;
    case "fail":
      return <span className="text-[14px] font-bold text-[color:var(--color-danger)]">✗</span>;
  }
}

/** Issue tags. The engine's internal "drift" reads as a meaning issue: the word
 *  "drift" never reaches the screen. Unknown types fall back to the meaning style. */
const TAG_CLS: Record<string, string> = {
  name: "border-primary/45 bg-primary/12 text-primary",
  meaning: "border-[color:var(--color-aqua)]/45 bg-[color:var(--color-aqua)]/10 text-[color:var(--color-aqua)]",
  drift: "border-[color:var(--color-aqua)]/45 bg-[color:var(--color-aqua)]/10 text-[color:var(--color-aqua)]",
  glossary: "border-primary/40 bg-primary/15 text-primary",
  merged: "border-[color:var(--color-alert)]/40 bg-[color:var(--color-alert)]/15 text-[color:var(--color-alert)]",
  dropped: "border-[color:var(--color-alert)]/40 bg-[color:var(--color-alert)]/15 text-[color:var(--color-alert)]",
  untranslated: "border-[color:var(--color-alert)]/40 bg-[color:var(--color-alert)]/10 text-[color:var(--color-alert)]",
};

const TAG_LABEL: Record<string, string> = {
  name: "name",
  meaning: "meaning",
  drift: "meaning",
  glossary: "glossary",
  merged: "merged",
  dropped: "dropped",
  untranslated: "untranslated",
};

/** The tag text for an issue type ("drift" shows as "meaning"). */
export const issueTagLabel = (type: string) => TAG_LABEL[type] ?? (type === "" ? "issue" : type);

/** Amber-edged fold-out listing each issue with its source → translation evidence.
 *  With `onLine`, "line 42" is a link (the episode window opens on that line). */
export function IssuePanel({
  issues,
  onLine,
}: {
  issues: VerifyIssue[];
  onLine?: (line: number) => void;
}) {
  return (
    <div className="border-l-2 border-[color:var(--color-alert)] bg-[color:var(--color-bg-deepest)] px-4 py-1">
      {issues.map((it, i) => (
        <div
          key={`${it.line_id}-${i}`}
          className="border-b border-dashed border-border py-2 last:border-0"
        >
          <div className="flex items-baseline gap-2">
            <span
              className={cn(
                "rounded border px-1.5 py-px text-[10.5px] font-medium uppercase tracking-wide",
                TAG_CLS[it.issue_type] ?? TAG_CLS.meaning,
              )}
            >
              {issueTagLabel(it.issue_type)}
            </span>
            {it.line_id > 0 ? (
              onLine ? (
                <button
                  type="button"
                  onClick={() => onLine(it.line_id)}
                  className="rounded-sm text-[11.5px] tabular-nums text-primary underline-offset-2 outline-none hover:underline focus-visible:ring-[3px] focus-visible:ring-ring/50"
                >
                  line {it.line_id}
                </button>
              ) : (
                <span className="text-[11.5px] tabular-nums text-muted-foreground">
                  line {it.line_id}
                </span>
              )
            ) : null}
          </div>
          <p className="mt-1 text-[12.5px]">{it.description}</p>
          {it.source !== "" && (
            <div className="mt-1 grid grid-cols-[14px_1fr] gap-x-2 text-[12px]">
              <span className="text-muted-foreground/60">源</span>
              <span className="italic text-muted-foreground">{it.source}</span>
              {it.translation !== "" && (
                <>
                  <span className="text-muted-foreground/60">→</span>
                  <span>{it.translation}</span>
                </>
              )}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
