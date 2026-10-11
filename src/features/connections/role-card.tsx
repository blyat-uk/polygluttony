import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

/** Card chrome shared by the two role slots: frame, numbered eyebrow, status dot. */

type CardVariant = "primary" | "solid" | "dashed";

const FRAME: Record<CardVariant, string> = {
  // The required slot: a gold-tinted hairline, no lift (Tone-Plus-Bloom).
  primary:
    "border-[color-mix(in_oklch,var(--color-gold)_32%,var(--border))] bg-card p-5 [box-shadow:inset_0_0_0_1px_rgba(236,223,198,.04)]",
  solid: "border-border bg-card p-5",
  // Not configured: dashed and see-through, so it reads as an empty slot.
  dashed: "border-dashed border-input bg-transparent px-5 py-4",
};

export function RoleCard({
  variant,
  labelledBy,
  children,
}: {
  variant: CardVariant;
  labelledBy: string;
  children: ReactNode;
}) {
  return (
    <section aria-labelledby={labelledBy} className={cn("rounded-2xl border", FRAME[variant])}>
      {children}
    </section>
  );
}

/** ① / ② — the role number, echoed in the flow strip. */
export function RoleBadge({ n, className }: { n: 1 | 2; className?: string }) {
  return (
    <span
      aria-hidden
      className={cn(
        "grid size-[18px] shrink-0 place-items-center rounded-full bg-[color:var(--color-raised)] text-[10px] font-semibold tracking-normal text-primary",
        className,
      )}
    >
      {n}
    </span>
  );
}

export function CardEyebrow({
  n,
  title,
  id,
  aside,
}: {
  n: 1 | 2;
  title: string;
  id: string;
  aside?: ReactNode;
}) {
  return (
    <div className="flex min-h-5 items-center gap-2">
      <RoleBadge n={n} />
      <h2
        id={id}
        className="text-[10.5px] font-semibold uppercase tracking-[0.1em] text-muted-foreground"
      >
        {title}
      </h2>
      {aside ? <div className="ml-auto flex items-center gap-3">{aside}</div> : null}
    </div>
  );
}

export type StatusTone = "jade" | "amber" | "aqua" | "muted";

const TONE: Record<StatusTone, string> = {
  jade: "text-[color:var(--color-jade)]",
  amber: "text-[color:var(--color-amber)]",
  aqua: "text-[color:var(--color-aqua)]",
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

/** A quiet word on the eyebrow's right ("optional", "off"). */
export function EyebrowNote({ children }: { children: ReactNode }) {
  return <span className="text-[11px] text-[color:var(--color-ink-dim)]">{children}</span>;
}
