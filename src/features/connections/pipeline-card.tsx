import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { ArrowCounterClockwise, Check } from "@phosphor-icons/react";
import type { ConnectionsConfig } from "@/types/generated/ConnectionsConfig";
import type { ProviderInfo } from "@/types/generated/ProviderInfo";
import type { Step } from "@/types/generated/Step";
import type { StepSettings } from "@/types/generated/StepSettings";
import type { ThinkingLevel } from "@/types/generated/ThinkingLevel";
import { findProvider, type Tuning } from "./connection-model";
import type { LibraryCaps } from "./per-step-model";
import { StepModelMenu } from "./step-model-menu";
import { StepRow } from "./step-row";
import { PHASE_LABEL, STEP_ORDER, STEP_INFO, modelById, stepSettings, type Phase } from "./steps";
import { UseOneModelDialog } from "./use-one-model-dialog";

const PHASES: readonly Phase[] = ["glossary", "translation"];

/** What goes into the pipeline, what the glossary phase hands on, and what comes out. */
const MARKER_BEFORE: Record<Phase, string> = { glossary: ".ass subtitles", translation: "glossary" };

/**
 * The per-step editor as the workflow it is: a vertical line from your subtitles, through the
 * glossary steps, to translated lines. Each step's dot takes its model's colour, so who does
 * what reads at a glance. Every change saves at once.
 */
export function PipelineCard({
  config,
  providers,
  caps,
  saving,
  saveFailed,
  onPickModel,
  onThinking,
  onTuning,
  onAddModel,
  onEditModel,
}: {
  config: ConnectionsConfig;
  providers: ProviderInfo[];
  caps: LibraryCaps;
  /** A step change is on its way to the backend. */
  saving: boolean;
  saveFailed: boolean;
  onPickModel: (step: Step, modelId: string | null) => void;
  onThinking: (step: Step, level: ThinkingLevel | null) => void;
  onTuning: (step: Step, patch: Partial<Tuning>) => Promise<unknown>;
  /** "+ Add a model…" in a step's menu. */
  onAddModel: (step: Step, trigger: HTMLElement) => void;
  /** "Add it" on a step whose model has no key. */
  onEditModel: (modelId: string, trigger: HTMLElement) => void;
}) {
  const uid = useId();
  const [oneModel, setOneModel] = useState(false);
  const feedback = useSaveFeedback(saving, saveFailed);
  const steps = config.steps;
  if (!steps) return null;

  const row = (step: Step) => {
    const settings: StepSettings | null = stepSettings(steps, step);
    const model = modelById(config, settings?.model);
    return (
      <StepRow
        key={step}
        step={step}
        settings={settings}
        model={model}
        info={model ? findProvider(providers, model.provider) : undefined}
        caps={model ? caps.byId.get(model.id) : undefined}
        menu={
          <StepModelMenu
            step={step}
            value={model ? model.id : null}
            models={config.models}
            providers={providers}
            caps={caps}
            onPick={(id) => onPickModel(step, id)}
            onAdd={(trigger) => onAddModel(step, trigger)}
          />
        }
        onThinking={(level) => onThinking(step, level)}
        onTuning={(patch) => onTuning(step, patch)}
        onAddKey={(trigger) => {
          if (model) onEditModel(model.id, trigger);
        }}
      />
    );
  };

  return (
    <section
      aria-labelledby={`${uid}-title`}
      className="rounded-2xl border border-[color-mix(in_oklch,var(--color-gold)_32%,var(--border))] bg-card px-5 pt-4 pb-3 [box-shadow:inset_0_0_0_1px_rgba(236,223,198,.04)]"
    >
      <h2 id={`${uid}-title`} className="sr-only">
        Which model does each step
      </h2>
      {/* The line runs from the first marker to the last; dots and markers sit on it. */}
      <div className="relative pl-7 before:absolute before:top-2.5 before:bottom-2.5 before:left-[9px] before:w-px before:bg-input">
        {PHASES.map((phase) => (
          <div key={phase}>
            <Marker>{MARKER_BEFORE[phase]}</Marker>
            <h3
              id={`${uid}-${phase}`}
              className="mt-2.5 mb-0.5 px-2.5 text-[9.5px] font-semibold tracking-[0.12em] text-[color:var(--color-ink-dim)] uppercase"
            >
              {PHASE_LABEL[phase]}
            </h3>
            <ul aria-labelledby={`${uid}-${phase}`} className="flex flex-col gap-0.5">
              {STEP_ORDER.filter((s) => STEP_INFO[s].phase === phase).map(row)}
            </ul>
          </div>
        ))}
        <Marker>translated lines</Marker>
      </div>

      <div className="mt-3 flex min-h-8 flex-wrap items-center gap-x-3 gap-y-1 border-t border-border pt-2">
        <button
          type="button"
          onClick={() => setOneModel(true)}
          className="-mx-1.5 inline-flex items-center gap-1.5 rounded-md px-1.5 py-1 text-[12px] text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50"
        >
          <ArrowCounterClockwise weight="bold" className="size-3.5" />
          Use one model for everything
        </button>
        <SaveFeedback state={feedback} />
      </div>

      <UseOneModelDialog
        open={oneModel}
        onOpenChange={setOneModel}
        config={config}
        providers={providers}
        caps={caps}
      />
    </section>
  );
}

/** `.ass subtitles` · `glossary` · `translated lines`: a square on the line, mono words. */
function Marker({ children }: { children: ReactNode }) {
  return (
    <div className="relative py-0.5 font-mono text-[11px] text-muted-foreground">
      <span
        aria-hidden
        className="absolute top-1/2 left-[-22px] size-[7px] -translate-y-1/2 rounded-[2px] border border-muted-foreground bg-card"
      />
      {children}
    </div>
  );
}

type Feedback = "saving" | "saved" | null;

/** "Saving…" while a change is in flight, then "Saved" for a moment. */
function useSaveFeedback(pending: boolean, failed: boolean): Feedback {
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

function SaveFeedback({ state }: { state: Feedback }) {
  return (
    <span aria-live="polite" className="ml-auto text-[11px] text-[color:var(--color-ink-dim)]">
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
