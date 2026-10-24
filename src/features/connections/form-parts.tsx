import type { ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ArrowSquareOut, CaretRight, WarningCircle, XCircle } from "@phosphor-icons/react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { cn } from "@/lib/utils";
import { hostOf } from "./connection-model";

/** Presentational pieces of the connection form: labelled field with a reserved message
 *  slot, tone-coded messages, inline text buttons, the key link and the Connect button. */

export type Phase = "idle" | "testing" | "saving";

export function Field({
  id,
  label,
  aside,
  message,
  children,
}: {
  id: string;
  label: string;
  aside?: ReactNode;
  message: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="flex min-w-0 flex-col">
      <div className="mb-1.5 flex min-h-4 items-baseline justify-between gap-2">
        <label htmlFor={id} className="text-[11.5px] font-semibold">
          {label}
        </label>
        {aside}
      </div>
      {children}
      {/* Reserved two-line slot: help, status and errors swap here without moving the form. */}
      <div id={`${id}-msg`} aria-live="polite" className="mt-1.5 min-h-8">
        {message}
      </div>
    </div>
  );
}

const MSG_TONE = {
  help: "text-muted-foreground",
  busy: "text-muted-foreground",
  ok: "text-[color:var(--color-jade)]",
  warn: "text-[color:var(--color-amber)]",
  error: "text-[color:var(--color-coral)]",
} as const;

export function Msg({ tone, children }: { tone: keyof typeof MSG_TONE; children: ReactNode }) {
  return (
    <p className={cn("flex items-start gap-1.5 text-[11.5px] leading-4", MSG_TONE[tone])}>
      {tone === "error" ? <XCircle weight="fill" className="mt-px size-3.5 shrink-0" /> : null}
      {tone === "warn" ? <WarningCircle weight="fill" className="mt-px size-3.5 shrink-0" /> : null}
      {tone === "busy" ? <Spinner className="mt-px size-3.5 shrink-0" /> : null}
      {tone === "ok" ? (
        <span aria-hidden className="mt-[5px] size-[7px] shrink-0 rounded-full bg-current" />
      ) : null}
      <span className="min-w-0">{children}</span>
    </p>
  );
}

export function TextButton({ onClick, children }: { onClick: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="inline-flex h-4 items-center gap-1 rounded align-bottom font-medium text-primary outline-none hover:underline focus-visible:ring-[3px] focus-visible:ring-ring/50"
    >
      {children}
    </button>
  );
}

export function KeyLink({ url }: { url: string }) {
  const host = hostOf(url).replace(/^www\./, "");
  return (
    <button
      type="button"
      onClick={() => openUrl(url).catch((e) => toast.error(`Couldn’t open ${host}: ${String(e)}`))}
      className="inline-flex min-w-0 items-center gap-1 truncate rounded text-[11px] font-medium text-primary outline-none hover:underline focus-visible:ring-[3px] focus-visible:ring-ring/50"
    >
      <span className="truncate">Get a key at {host}</span>
      <ArrowSquareOut className="size-3 shrink-0" />
    </button>
  );
}

/** Connect is its own loader: gold-deep with flowing stripes while it tests and saves. */
export function ConnectButton({
  ref,
  phase,
  retry,
}: {
  ref: React.Ref<HTMLButtonElement>;
  phase: Phase;
  retry: boolean;
}) {
  const busy = phase !== "idle";
  return (
    <Button
      ref={ref}
      type="submit"
      disabled={busy}
      aria-busy={busy || undefined}
      className={cn(
        "min-w-[8.5rem]",
        busy &&
          "disabled:opacity-100 !bg-[var(--color-gold-deep)] !text-[color:var(--color-ink-emphasis)]",
      )}
    >
      {busy ? (
        <span
          aria-hidden
          className="pointer-events-none absolute inset-0 [animation:sig-flow_1.05s_linear_infinite] [background:repeating-linear-gradient(115deg,transparent_0_9px,rgba(255,255,255,.18)_9px_11px)]"
        />
      ) : null}
      <span className="relative z-[1] inline-flex items-center gap-2">
        {busy ? <Spinner className="size-3.5" /> : null}
        {phase === "testing"
          ? "Connecting…"
          : phase === "saving"
            ? "Saving…"
            : retry
              ? "Try again"
              : "Connect"}
      </span>
    </Button>
  );
}

/** "Details ▸": the provider's own words for a failure, closed until asked for. */
export function Details({ text, className }: { text: string; className?: string }) {
  return (
    <details className={cn("group text-[11px] leading-4", className)}>
      <summary className="inline-flex cursor-pointer list-none items-center gap-1 rounded-md text-muted-foreground outline-none select-none hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 [&::-webkit-details-marker]:hidden">
        Details
        <CaretRight weight="fill" className="size-2.5 transition-transform group-open:rotate-90" />
      </summary>
      <pre className="mt-1.5 max-h-32 overflow-auto rounded-lg border border-border bg-[color:var(--color-obsidian)] px-2.5 py-2 font-mono text-[11px] whitespace-pre-wrap break-words text-muted-foreground select-text">
        {text}
      </pre>
    </details>
  );
}
