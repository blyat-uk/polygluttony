import { useId, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import type { ConnectionsConfig } from "@/types/generated/ConnectionsConfig";
import type { Model } from "@/types/generated/Model";
import type { ProviderInfo } from "@/types/generated/ProviderInfo";
import type { Steps } from "@/types/generated/Steps";
import { PageHeader } from "@/components/page-header";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { findProvider, isUsable, modelLabel, providerName } from "./connection-model";
import { ConnectionForm, type FormStage } from "./connection-form";
import { FlowStrip, type WebSearchNode } from "./flow-strip";
import { OneModelCard } from "./one-model-card";
import { PerStepView } from "./per-step-view";
import { CardEyebrow, RoleCard, StatusDot } from "./role-card";
import { modelById } from "./steps";
import {
  capabilitiesQuery,
  useCapabilities,
  useConnectionMutations,
  useConnectionsConfig,
  useProviders,
} from "./use-connections";

/** Nothing set up → connect a model; then one model for every step, or a model per step. */
export function ConnectionsPage() {
  const config = useConnectionsConfig();
  const providers = useProviders();
  const steps = config.data?.steps ?? null;
  const main = config.data && steps ? modelById(config.data, steps.translate.model) : undefined;
  const caps = useCapabilities(main?.provider, main?.model);
  // Wait for the model's capabilities too, so Thinking / Web search don't pop in after paint.
  const capsSettled = !main || caps.data !== undefined || caps.isError;
  // Where focus lands when the page swaps setup for the card (or back), so it isn't dropped.
  const [focus, setFocus] = useState<"edit" | "setup" | null>(null);
  // Back from the per-step view: the control that did it is gone, so focus the card's Edit.
  const perStep = config.data?.per_step;
  const [prevPerStep, setPrevPerStep] = useState(perStep);
  if (perStep !== prevPerStep) {
    setPrevPerStep(perStep);
    if (prevPerStep === true && perStep === false) setFocus("edit");
  }

  const error = config.error ?? providers.error;
  return (
    <div className="flex min-h-full flex-col">
      <PageHeader
        title="Connections"
        description="The AI models that build your glossary and translate your subtitles."
      />
      <div className="px-5 py-5">
        <div className="mx-auto flex w-full max-w-[700px] flex-col gap-3">
          {config.data && providers.data && capsSettled ? (
            !steps || !main ? (
              <FirstRunCard
                config={config.data}
                providers={providers.data}
                autoFocus={focus === "setup"}
                onConnected={() => setFocus("edit")}
              />
            ) : config.data.per_step ? (
              <PerStepView config={config.data} providers={providers.data} />
            ) : (
              <>
                <FlowStrip
                  usable={isUsable(main)}
                  webSearch={webSearchNode(config.data, steps, main, providers.data)}
                />
                <OneModelCard
                  key={main.id}
                  config={config.data}
                  steps={steps}
                  model={main}
                  providers={providers.data}
                  caps={caps.data}
                  focusEdit={focus === "edit"}
                  onRemoving={(remaining) => setFocus(remaining ? "edit" : "setup")}
                />
              </>
            )
          ) : error ? (
            <div className="flex items-center gap-3 rounded-2xl border border-border bg-card px-5 py-4 text-[12.5px]">
              <span className="min-w-0 flex-1 text-[color:var(--color-coral)]">
                Couldn’t load your connections: {String(error)}
              </span>
              <Button
                size="sm"
                variant="outline"
                onClick={() => {
                  void config.refetch();
                  void providers.refetch();
                }}
              >
                Retry
              </Button>
            </div>
          ) : (
            <div className="flex items-center gap-2 py-6 text-[12px] text-muted-foreground">
              <Spinner className="size-3.5" /> Loading connections…
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

/** First run: the connection form in the card it turns into. */
function FirstRunCard({
  config,
  providers,
  autoFocus,
  onConnected,
}: {
  config: ConnectionsConfig;
  providers: ProviderInfo[];
  /** Focus the first provider tile (the last model was just removed). */
  autoFocus: boolean;
  onConnected: () => void;
}) {
  const qc = useQueryClient();
  const titleId = useId();
  const m = useConnectionMutations();
  const [stage, setStage] = useState<FormStage>("pick");
  return (
    <RoleCard variant="primary" labelledBy={titleId}>
      <CardEyebrow
        title="Your model"
        id={titleId}
        aside={
          <StatusDot tone="amber" hollow>
            {stage === "fields" ? "Not connected yet" : "Not set up"}
          </StatusDot>
        }
      />
      <ConnectionForm
        role="model"
        heading="Connect an AI model"
        lead="It builds your glossary and translates your subtitles. You’ll need an API key (local models don’t need one)."
        providers={providers}
        providerKeys={config.provider_keys}
        initial={null}
        autoFocus={autoFocus}
        onStageChange={setStage}
        onSave={async (model, tested) => {
          // Load what the model supports first, so the card it turns into paints whole.
          await qc.prefetchQuery(capabilitiesQuery(model.provider, model.model));
          await m.saveModel.mutateAsync({ model });
          onConnected();
          const name = providerName(findProvider(providers, model.provider), model);
          if (tested) toast.success(`Connected to ${name}`);
          else toast.success(`Saved without confirming — ${name} didn’t answer the test.`);
        }}
      />
    </RoleCard>
  );
}

function webSearchNode(
  cfg: ConnectionsConfig,
  steps: Steps,
  main: Model,
  providers: ProviderInfo[],
): WebSearchNode {
  const ws = steps.web_search;
  const model = ws ? modelById(cfg, ws.model) : undefined;
  if (!model) return { state: "off" };
  if (!isUsable(model)) return { state: "needs-key" };
  if (model.id === main.id) return { state: "same" };
  return { state: "separate", label: modelLabel(findProvider(providers, model.provider), model.model) };
}
