import { useEffect, useId, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import type { ConnectionsConfig } from "@/types/generated/ConnectionsConfig";
import type { Model } from "@/types/generated/Model";
import type { ProviderInfo } from "@/types/generated/ProviderInfo";
import type { Step } from "@/types/generated/Step";
import type { StepSettings } from "@/types/generated/StepSettings";
import type { ThinkingLevel } from "@/types/generated/ThinkingLevel";
import { Spinner } from "@/components/ui/spinner";
import { findProvider, isUsable, modelLabel, providerName, type Tuning } from "./connection-model";
import { ConnectionForm } from "./connection-form";
import { ModelChips, ModelDot } from "./model-chips";
import { useLibraryCapabilities } from "./per-step-model";
import { PipelineCard } from "./pipeline-card";
import { RemoveModelDialog } from "./remove-model-dialog";
import { EyebrowNote, StatusDot } from "./role-card";
import {
  STEP_INFO,
  defaultStepSettings,
  modelById,
  modelColor,
  stepNames,
  stepSettings,
  stepsUsing,
} from "./steps";
import { capabilitiesQuery, connectionsKey, useConnectionMutations } from "./use-connections";

/** Which form is open under the chips: a library model's, or a new one (maybe for a step). */
type FormTarget = { kind: "edit"; id: string } | { kind: "add"; assign?: Step };

/** Where focus goes once the form closes: its trigger, or "+ Add" when that's gone. */
type FocusTarget = HTMLElement | "add";

/** The add form's question when it was opened from a step's menu. */
const ASK: Record<Step, string> = {
  extract: "Which AI should extract names?",
  normalize: "Which AI should normalize the glossary?",
  "web-search": "Which AI should search the web?",
  translate: "Which AI should translate?",
  verify: "Which AI should verify translations?",
};

/**
 * Per-step view (layout B): the model library as a slim row of chips, the form for one of
 * them right under it, then the pipeline card where each step picks its model and settings.
 */
export function PerStepView({
  config,
  providers,
  autoFocus = false,
}: {
  config: ConnectionsConfig;
  providers: ProviderInfo[];
  /** Focus the pipeline once it shows (the one-model card's switch that opened this is gone). */
  autoFocus?: boolean;
}) {
  const qc = useQueryClient();
  const m = useConnectionMutations();
  const caps = useLibraryCapabilities(config.models);
  const formId = useId();
  const [form, setForm] = useState<FormTarget | null>(null);
  const [removing, setRemoving] = useState<Model | null>(null);
  const [focusAfter, setFocusAfter] = useState<FocusTarget | null>(null);
  const returnTo = useRef<HTMLElement | null>(null);
  const addRef = useRef<HTMLButtonElement>(null);
  const pipelineRef = useRef<HTMLDivElement>(null);

  // Show the pipeline once every model's capabilities are in, so Thinking tags don't pop in
  // after paint. From then on it stays: a newly added model fills its tag in place.
  const [ready, setReady] = useState(caps.settled);
  if (caps.settled && !ready) setReady(true);

  useEffect(() => {
    if (autoFocus && ready) pipelineRef.current?.focus();
  }, [autoFocus, ready]);

  // After the form closes (and the chips re-render), put focus back where it came from.
  useEffect(() => {
    if (!focusAfter) return;
    const el = focusAfter === "add" || !focusAfter.isConnected ? addRef.current : focusAfter;
    el?.focus();
    setFocusAfter(null);
  }, [focusAfter]);

  const editing = form?.kind === "edit" ? modelById(config, form.id) : undefined;
  // An edit form whose model has gone has nothing left to edit.
  const open = form?.kind === "edit" && !editing ? null : form;

  const openForm = (target: FormTarget, trigger: HTMLElement) => {
    returnTo.current = trigger;
    setForm(target);
  };
  const closeForm = (focus: FocusTarget | null = returnTo.current) => {
    setForm(null);
    setFocusAfter(focus ?? "add");
    returnTo.current = null;
  };

  /** A step's settings as of now: the cache already holds optimistic changes this render
   *  may not have seen, so a save never undoes one made a moment before. */
  const current = (step: Step): StepSettings | null => {
    const cfg = qc.getQueryData<ConnectionsConfig>(connectionsKey) ?? config;
    return cfg.steps ? stepSettings(cfg.steps, step) : null;
  };

  const pickModel = (step: Step, id: string | null) => {
    if (id === null) {
      m.setStep.mutate({ step, settings: null });
      return;
    }
    // New model: its own Thinking default (the backend fills it in), the step's tuning kept.
    const settings = { ...(current(step) ?? defaultStepSettings(id)), model: id, thinking: null };
    m.setStep.mutate({ step, settings });
  };

  const setThinking = (step: Step, thinking: ThinkingLevel | null) => {
    const now = current(step);
    if (now) m.setStep.mutate({ step, settings: { ...now, thinking } });
  };

  const setTuning = async (step: Step, patch: Partial<Tuning>) => {
    const now = current(step);
    if (!now) return;
    try {
      await m.setStep.mutateAsync({ step, settings: { ...now, ...patch } });
      toast.success(`${STEP_INFO[step].name} tuning saved`);
    } catch {
      // The mutation already rolled back and reported the error.
    }
  };

  const save = async (model: Model, tested: boolean, target: FormTarget) => {
    const assign = target.kind === "add" ? target.assign : undefined;
    const searchedBefore = !!model.id && config.steps?.web_search?.model === model.id;
    // Throws on failure: the form stays open and shows the error.
    const cfg = await m.saveModel.mutateAsync({ model, assign });
    // Have every model's capabilities ready, so the pipeline doesn't fill in after closing.
    await Promise.all(
      cfg.models.map((x) =>
        qc.ensureQueryData(capabilitiesQuery(x.provider, x.model)).catch(() => undefined),
      ),
    );
    closeForm();

    const info = findProvider(providers, model.provider);
    const label = modelLabel(info, model.model);
    const parts = [
      !tested
        ? `${label} saved without confirming — ${providerName(info, model)} didn’t answer the test.`
        : target.kind === "edit"
          ? `${label} saved.`
          : `${label} connected.`,
    ];
    if (assign) parts.push(`${STEP_INFO[assign].name} uses it now.`);
    else if (target.kind === "add") parts.push("Pick it for any step below.");
    if (searchedBefore && cfg.steps?.web_search?.model !== model.id) {
      parts.push("Web search turned off: this model can’t search the web.");
    }
    toast.success(parts.join(" "));
  };

  return (
    <div className="flex flex-col gap-3">
      <ModelChips
        models={config.models}
        providers={providers}
        openId={open?.kind === "edit" ? open.id : null}
        adding={open?.kind === "add" && !open.assign}
        formId={formId}
        addRef={addRef}
        onEdit={(id, trigger) => openForm({ kind: "edit", id }, trigger)}
        onAdd={(trigger) => openForm({ kind: "add" }, trigger)}
        onClose={() => closeForm()}
      />

      {open ? (
        <FormCard
          key={open.kind === "edit" ? `edit-${open.id}` : `add-${open.assign ?? ""}`}
          id={formId}
          target={open}
          model={editing}
          config={config}
          providers={providers}
          onSave={(model, tested) => save(model, tested, open)}
          onCancel={() => closeForm()}
          onRemove={editing ? () => setRemoving(editing) : undefined}
        />
      ) : null}

      {ready ? (
        <div
          ref={pipelineRef}
          tabIndex={-1}
          role="group"
          aria-label="Steps"
          className="outline-none"
        >
          <PipelineCard
            config={config}
            providers={providers}
            caps={caps}
            saving={m.setStep.isPending}
            saveFailed={m.setStep.isError}
            onPickModel={pickModel}
            onThinking={setThinking}
            onTuning={setTuning}
            onAddModel={(step, trigger) => openForm({ kind: "add", assign: step }, trigger)}
            onEditModel={(id, trigger) => openForm({ kind: "edit", id }, trigger)}
          />
        </div>
      ) : (
        <div className="flex items-center gap-2 py-6 text-[12px] text-muted-foreground">
          <Spinner className="size-3.5" /> Loading your steps…
        </div>
      )}

      <RemoveModelDialog
        model={removing}
        config={config}
        providers={providers}
        onClose={() => setRemoving(null)}
        // Its chip is gone too: focus lands on "+ Add".
        onRemoved={() => closeForm("add")}
      />
    </div>
  );
}

/** The shared connection form in a card under the chips, with a header saying what it's for. */
function FormCard({
  id,
  target,
  model,
  config,
  providers,
  onSave,
  onCancel,
  onRemove,
}: {
  id: string;
  target: FormTarget;
  /** The library model being edited. */
  model: Model | undefined;
  config: ConnectionsConfig;
  providers: ProviderInfo[];
  onSave: (model: Model, tested: boolean) => Promise<void>;
  onCancel: () => void;
  onRemove?: () => void;
}) {
  const titleId = useId();
  const assign = target.kind === "add" ? target.assign : undefined;
  const used = model ? stepsUsing(config, model.id) : [];
  // Search-capable providers only (and a test that also searches) when web search is the
  // model's whole job.
  const role =
    assign === "web-search" || (used.length === 1 && used[0] === "web-search")
      ? "web-search"
      : "model";

  return (
    <section
      id={id}
      aria-labelledby={titleId}
      className="rounded-2xl border border-border bg-card p-5"
    >
      <div className="flex min-h-5 flex-wrap items-center gap-x-2 gap-y-1">
        {model ? <ModelDot color={modelColor(model)} /> : null}
        <h2
          id={titleId}
          className="text-[10.5px] font-semibold tracking-[0.1em] text-muted-foreground uppercase"
        >
          {model
            ? modelLabel(findProvider(providers, model.provider), model.model)
            : assign
              ? `Add a model for ${STEP_INFO[assign].name}`
              : "Add a model"}
        </h2>
        {model ? (
          <div className="ml-auto flex items-center gap-3">
            <EyebrowNote>
              {used.length > 0 ? `used by ${stepNames(used)}` : "no step uses it yet"}
            </EyebrowNote>
            {isUsable(model) ? (
              <StatusDot tone="jade" glow>
                Connected
              </StatusDot>
            ) : (
              <StatusDot tone="amber" hollow>
                Needs an API key
              </StatusDot>
            )}
          </div>
        ) : null}
      </div>
      <ConnectionForm
        role={role}
        providers={providers}
        providerKeys={config.provider_keys}
        initial={model ?? null}
        onSave={onSave}
        onCancel={onCancel}
        onRemove={onRemove}
        autoFocus
        heading={assign ? ASK[assign] : model ? undefined : "Which AI do you want to add?"}
        lead={
          assign === "web-search"
            ? "It looks up the names fans already use, once per glossary build. Anthropic, OpenAI and Google can search."
            : assign
              ? `Pick a provider. ${STEP_INFO[assign].name} switches to it once it’s connected.`
              : model
                ? undefined
                : "Pick a provider. Once it’s connected, choose which steps use it below."
        }
      />
    </section>
  );
}
