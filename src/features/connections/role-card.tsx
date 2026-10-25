import { useEffect, useRef, useState, type ReactNode } from "react";
import { Check } from "@phosphor-icons/react";
import { cn } from "@/lib/utils";

/** Card chrome for the Connections page: frame, eyebrow, status dot, option rows and the
 *  quiet "Saving… / Saved" note for inline changes. */

type CardVariant = "primary" | "solid" | "dashed";

const FRAME: Record<CardVariant, string> = {
  // The main card: a gold-tinted hairline, no lift (Tone-Plus-Bloom).
  primary:
    "border-[color-mix(in_oklch,var(--color-gold)_32%,var(--border))] bg-card p-5 [box-shadow:inset_0_0_0_1px_rgba(236,223,198,.04)]",
  solid: "border-border bg-card p-5",
  // Not configured: dashed and see-through, so it reads as an empty slot.
  dashed: "border-dashed border-input bg-transparent px-5 py-4",
};

export function RoleCard({
  variant,
  labelledBy,
  className,
  children,
}: {
  variant: CardVariant;
  labelledBy: string;
  className?: string;
  children: ReactNode;
}) {
  return (
    <section
      aria-labelledby={labelledBy}
      className={cn("rounded-2xl border", FRAME[variant], className)}
    >
      {children}
    </section>
  );
}

/** "YOUR MODEL · does every step ……… ● Connected": the card's heading, a quiet note after it,
 *  and its status on the right. */
export function CardEyebrow({
  title,
  id,
  note,
  aside,
}: {
  title: string;
  id: string;
  note?: ReactNode;
  aside?: ReactNode;
}) {
  return (
    <div className="flex min-h-5 flex-wrap items-center gap-x-2 gap-y-1">
      <h2
        id={id}
        className="text-[10.5px] font-semibold uppercase tracking-[0.1em] text-muted-foreground"
      >
        {title}
      </h2>
      {note ? <EyebrowNote>{note}</EyebrowNote> : null}
      {aside ? <div className="ml-auto flex items-center gap-3">{aside}</div> : null}
    </div>
  );
}

export type StatusTone = "jade" | "amber" | "aqua" | "coral" | "muted";

const TONE: Record<StatusTone, string> = {
  jade: "text-[color:var(--color-jade)]",
  amber: "text-[color:var(--color-amber)]",
  aqua: "text-[color:var(--color-aqua)]",
  coral: "text-[color:var(--color-coral)]",
  muted: "text-muted-foreground",
};

/** Colour + dot + words: state reads twice. A hollow dot means "not there yet". */
export function StatusDot({
  tone,
  hollow = false,
  glow = false,
  children,
}: {
  tone: StatusTone;
  hollow?: boolean;
  glow?: boolean;
  children: ReactNode;
}) {
  return (
    <span className={cn("inline-flex items-center gap-1.5 text-[11px] font-medium", TONE[tone])}>
      <span
        aria-hidden
        className={cn(
          "size-[7px] shrink-0 rounded-full",
          hollow ? "border border-current" : "bg-current",
          glow && "[box-shadow:0_0_8px_currentColor]",
        )}
      />
      {children}
    </span>
  );
}

/** A quiet word next to the eyebrow ("· does every step", "optional"). */
export function EyebrowNote({ children }: { children: ReactNode }) {
  return <span className="text-[11px] text-[color:var(--color-ink-dim)]">{children}</span>;
}

/** One labelled setting: the label column, its control, a help line that takes the rest of
 *  the row, and optional trailing actions. Wraps under the label on narrow widths. */
export function OptionRow({
  labelId,
  label,
  help,
  end,
  children,
}: {
  labelId: string;
  label: string;
  help?: ReactNode;
  /** Trailing actions (links, Edit / Remove). */
  end?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className="flex min-h-10 flex-wrap items-center gap-x-3.5 gap-y-1 py-1">
      <span id={labelId} className="w-[104px] shrink-0 text-[12px] leading-4 font-medium">
        {label}
      </span>
      {children}
      {help ? (
        <span className="min-w-0 flex-1 basis-48 text-[11.5px] leading-4 text-muted-foreground">
          {help}
        </span>
      ) : null}
      {end ? <span className="ml-auto flex shrink-0 items-center gap-1.5">{end}</span> : null}
    </div>
  );
}

export type SaveFeedbackState = "saving" | "saved" | null;

/** "Saving…" while an inline change is in flight, then "Saved" for a moment. */
export function useSaveFeedback(pending: boolean, failed: boolean): SaveFeedbackState {
  const [justSaved, setJustSaved] = useState(false);
  const wasPending = useRef(false);
  useEffect(() => {
    const finished = wasPending.current && !pending;
    wasPending.current = pending;
    if (!finished || failed) return;
    setJustSaved(true);
    const t = setTimeout(() => setJustSaved(false), 1600);
    return () => clearTimeout(t);
  }, [pending, failed]);
  return pending ? "saving" : justSaved ? "saved" : null;
}

export function SaveFeedback({ state }: { state: SaveFeedbackState }) {
  return (
    <span aria-live="polite" className="text-[11px] text-[color:var(--color-ink-dim)]">
      {state === "saving" ? (
        "Saving…"
      ) : state === "saved" ? (
        <span className="inline-flex items-center gap-1">
          <Check weight="bold" className="size-3" /> Saved
        </span>
      ) : null}
    </span>
  );
}
