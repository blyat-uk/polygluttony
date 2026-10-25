import { CheckCircle, XCircle } from "@phosphor-icons/react";
import type { Model } from "@/types/generated/Model";
import type { ProviderInfo } from "@/types/generated/ProviderInfo";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { cn } from "@/lib/utils";
import { problemCopy, problemDetails } from "./error-copy";
import { Details } from "./form-parts";
import { StatusDot } from "./role-card";
import { formatLatency, type ModelStatus, type ModelTest } from "./test-status";

/** Colour + dot + words for a library model: Needs an API key · Ready · Testing… ·
 *  Connected · Test failed. "Connected" only once a test passed this session. */
export function ModelStatusDot({ status }: { status: ModelStatus }) {
  switch (status) {
    case "needs-key":
      return (
        <StatusDot tone="amber" hollow>
          Needs an API key
        </StatusDot>
      );
    case "connected":
      return (
        <StatusDot tone="jade" glow>
          Connected
        </StatusDot>
      );
    case "failed":
      return <StatusDot tone="coral">Test failed</StatusDot>;
    case "testing":
      return <StatusDot tone="muted">Testing…</StatusDot>;
    default:
      return <StatusDot tone="muted">Ready</StatusDot>;
  }
}

/** "Test": one tiny request to the saved model. Nothing is saved. */
export function TestButton({
  test,
  onTest,
  label,
  disabled = false,
}: {
  test: ModelTest;
  onTest: () => void;
  /** The model's name, for screen readers ("Test Claude Sonnet 5.5"). */
  label: string;
  disabled?: boolean;
}) {
  const busy = test.state === "testing";
  return (
    <Button
      variant="ghost"
      size="sm"
      aria-label={`Test ${label}`}
      aria-busy={busy || undefined}
      disabled={busy || disabled}
      onClick={onTest}
    >
      {busy ? <Spinner className="size-3.5" /> : null}
      {busy ? "Testing…" : "Test"}
    </Button>
  );
}

/** The last test's answer, inline: "✓ Works · 1.4 s", or what went wrong and what to do,
 *  with the provider's own words behind Details. Nothing before the first test. */
export function TestOutcome({
  test,
  model,
  info,
  className,
}: {
  test: ModelTest;
  model: Model;
  info: ProviderInfo | undefined;
  className?: string;
}) {
  if (test.state === "works") {
    return (
      <p
        aria-live="polite"
        className={cn(
          "flex items-center gap-1.5 text-[11.5px] leading-4 text-[color:var(--color-jade)] tabular-nums",
          className,
        )}
      >
        <CheckCircle weight="fill" className="size-3.5 shrink-0" />
        Works · {formatLatency(test.seconds)}
      </p>
    );
  }
  if (test.state !== "failed") return null;
  const text = info
    ? problemCopy(test.result, info, model, { saved: true }).text
    : test.result.message;
  const details = problemDetails(test.result, text);
  return (
    <div aria-live="polite" className={className}>
      <p className="flex items-start gap-1.5 text-[11.5px] leading-4 text-[color:var(--color-coral)] select-text">
        <XCircle weight="fill" className="mt-px size-3.5 shrink-0" />
        <span className="min-w-0">{text}</span>
      </p>
      {details ? <Details text={details} className="mt-1 pl-5" /> : null}
    </div>
  );
}
