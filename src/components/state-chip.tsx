import { cn } from "@/lib/utils";

const STATE_COLOR: Record<string, string> = {
  idle: "var(--muted-foreground)",
  pending: "var(--muted-foreground)",
  translating: "var(--color-gold)",
  retranslating: "var(--color-amber)",
  cleanup: "var(--color-state-cleanup)",
  verifying: "var(--color-state-verify)",
  done: "var(--color-success)",
  warning: "var(--color-alert)",
  failed: "var(--color-danger)",
  stopped: "var(--muted-foreground)",
};
const LIVE = new Set(["translating", "retranslating", "cleanup", "verifying"]);

/** Dot + label: state reads as colour, glyph and text. Pass `color`/`live` to
 *  show a state outside the engine's own (e.g. an episode's "New"). */
export function StateChip({
  state = "idle",
  label,
  color,
  live,
  className,
}: {
  state?: string;
  label?: string;
  color?: string;
  live?: boolean;
  className?: string;
}) {
  const dot = color ?? STATE_COLOR[state] ?? STATE_COLOR.idle;
  const pulse = live ?? LIVE.has(state);
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-[13px] border border-border px-2.5 py-0.5 text-[11px] whitespace-nowrap",
        className,
      )}
    >
      <span
        className={`size-2 shrink-0 rounded-full${pulse ? " [animation:sig-ping_1.5s_ease-out_infinite]" : ""}`}
        style={{ background: dot }}
      />
      {label ?? state[0].toUpperCase() + state.slice(1)}
    </span>
  );
}
