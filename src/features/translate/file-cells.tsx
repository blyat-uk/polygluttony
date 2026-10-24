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

/** The engine emits "drift" and the synthesized "untranslated"; the other keys are pre-styled for future issue types. */
const TAG_CLS: Record<string, string> = {
  drift: "border-[color:var(--color-state-verify)]/40 bg-[color:var(--color-state-verify)]/15 text-[color:var(--color-state-verify)]",
  glossary: "border-primary/40 bg-primary/15 text-primary",
  merged: "border-[color:var(--color-alert)]/40 bg-[color:var(--color-alert)]/15 text-[color:var(--color-alert)]",
  dropped: "border-[color:var(--color-alert)]/40 bg-[color:var(--color-alert)]/15 text-[color:var(--color-alert)]",
  untranslated: "border-[color:var(--color-alert)]/40 bg-[color:var(--color-alert)]/15 text-[color:var(--color-alert)]",
};

/** Amber-edged fold-out listing each issue with its source → translation evidence. */
export function IssuePanel({ issues }: { issues: VerifyIssue[] }) {
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
                TAG_CLS[it.issue_type] ?? TAG_CLS.drift,
              )}
            >
              {it.issue_type}
            </span>
            {it.line_id > 0 && (
              <span className="text-[11.5px] tabular-nums text-muted-foreground">
                line {it.line_id}
              </span>
            )}
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
