import type { FileStateKind } from "@/types/generated/FileStateKind";

// Shared by the Translate table and the episode window.

export const STATE_LABEL: Record<FileStateKind, string> = {
  pending: "Waiting",
  translating: "Translating…",
  retranslating: "Retranslating…",
  cleanup: "Cleaning up…",
  verifying: "Verifying…",
  done: "Done",
  warning: "Needs a look",
  failed: "Failed",
  stopped: "Not finished",
};

export const STATE_COLOR: Record<FileStateKind, string> = {
  pending: "var(--muted-foreground)",
  translating: "var(--primary)",
  retranslating: "var(--color-alert)",
  cleanup: "var(--color-state-cleanup)",
  verifying: "var(--color-state-verify)",
  done: "var(--color-success)",
  warning: "var(--color-alert)",
  failed: "var(--color-danger)",
  stopped: "var(--muted-foreground)",
};

/** Whether the state should pulse to indicate live activity. */
const STATE_ANIMATE = new Set<FileStateKind>([
  "translating",
  "retranslating",
  "cleanup",
  "verifying",
]);

export function FileStateChip({ state }: { state: FileStateKind }) {
  const color = STATE_COLOR[state] ?? STATE_COLOR.pending;
  const animate = STATE_ANIMATE.has(state);
  return (
    <span className="inline-flex items-center gap-1.5 rounded-[13px] border border-border px-2.5 py-0.5 text-[11px] shrink-0">
      <span
        className={`size-2 rounded-full${animate ? " animate-pulse" : ""}`}
        style={{ background: color }}
      />
      {STATE_LABEL[state]}
    </span>
  );
}
