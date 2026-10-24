import { useId, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { DownloadSimple, UploadSimple } from "@phosphor-icons/react";
import { toast } from "sonner";
import type { ConnectionsConfig } from "@/types/generated/ConnectionsConfig";
import type { Model } from "@/types/generated/Model";
import type { ProviderInfo } from "@/types/generated/ProviderInfo";
import type { Steps } from "@/types/generated/Steps";
import { PageHeader } from "@/components/page-header";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { findProvider, isUsable, modelLabel, providerName } from "./connection-model";
import { ConnectionForm, type FormStage } from "./connection-form";
import { FlowStrip, type WebSearchNode } from "./flow-strip";
import { OneModelCard } from "./one-model-card";
import { PerStepView } from "./per-step-view";
import { CardEyebrow, RoleCard, StatusDot } from "./role-card";
import { useSetupTransfer } from "./setup-transfer";
import { modelById } from "./steps";
import {
  capabilitiesQuery,
  useCapabilities,
  useConnectionMutations,
  useConnectionsConfig,
  useProviders,
} from "./use-connections";

/** Nothing set up → connect a model (or import a setup); then one model for every step, or a
 *  model per step. The header carries a whole setup to and from another computer. */
export function ConnectionsPage() {
  const config = useConnectionsConfig();
  const providers = useProviders();
  const transfer = useSetupTransfer();
  const steps = config.data?.steps ?? null;
  const main = config.data && steps ? modelById(config.data, steps.translate.model) : undefined;
  const caps = useCapabilities(main?.provider, main?.model);
  // Wait for the model's capabilities too, so Thinking / Web search don't pop in after paint.
  const capsSettled = !main || caps.data !== undefined || caps.isError;
  // Where focus lands when the page swaps setup for the card (or back), so it isn't dropped.
  const [focus, setFocus] = useState<"edit" | "setup" | "steps" | null>(null);
  // Back from the per-step view: the control that did it is gone, so focus the card's Edit.
  const perStep = config.data?.per_step;
  const [prevPerStep, setPrevPerStep] = useState(perStep);
  if (perStep !== prevPerStep) {
    setPrevPerStep(perStep);
    // (Removing the last model also lands here, on setup.)
    if (prevPerStep === true && perStep === false) setFocus(steps ? "edit" : "setup");
    // Into it: the footer link that did it is gone too, so focus the pipeline.
    if (prevPerStep === false && perStep === true) setFocus("steps");
  }
  // Setup → the card (a model connected, or a setup imported): the control that did it is
  // gone with the setup card, so focus what replaced it.
  const setup = config.data ? !steps || !main : undefined;
  const [prevSetup, setPrevSetup] = useState(setup);
  if (setup !== prevSetup) {
    setPrevSetup(setup);
    if (prevSetup === true && setup === false) setFocus(perStep ? "steps" : "edit");
  }

  const error = config.error ?? providers.error;
  return (
    <div className="flex min-h-full flex-col">
      <PageHeader
        title="Connections"
        description="The AI models that build your glossary and translate your subtitles."
        actions={
          <div className="flex shrink-0 items-center gap-2">
            {/* aria-disabled, not disabled: a disabled button drops focus to the page. */}
            <Button
              size="sm"
              variant="secondary"
              aria-disabled={transfer.busy || undefined}
              className="aria-disabled:pointer-events-none aria-disabled:opacity-50"
              onClick={() => void transfer.startImport()}
            >
              <UploadSimple className="size-4" /> Import setup…
            </Button>
            {/* Nothing to carry over until a model is set up. */}
            {config.data && config.data.models.length > 0 ? (
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    size="sm"
                    variant="secondary"
                    aria-disabled={transfer.busy || undefined}
                    className="aria-disabled:pointer-events-none aria-disabled:opacity-50"
                    onClick={() => void transfer.exportSetup()}
                  >
                    <DownloadSimple className="size-4" /> Export setup…
                  </Button>
                </TooltipTrigger>
                <TooltipContent side="bottom">Includes your API keys in plain text.</TooltipContent>
              </Tooltip>
            ) : null}
          </div>
        }
      />
      <div className="px-5 py-5">
        <div className="mx-auto flex w-full max-w-[700px] flex-col gap-3">
          {config.data && providers.data && capsSettled ? (
            !steps || !main ? (
              <FirstRunCard
                config={config.data}
                providers={providers.data}
                autoFocus={focus === "setup"}
                importing={transfer.busy}
                onImport={() => void transfer.startImport()}
              />
            ) : config.data.per_step ? (
              <PerStepView
                config={config.data}
                providers={providers.data}
                autoFocus={focus === "steps"}
              />
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
      {transfer.dialog}
    </div>
  );
}

/** First run: the connection form in the card it turns into, and a way to bring a setup over
 *  instead. */
function FirstRunCard({
  config,
  providers,
  autoFocus,
  importing,
  onImport,
}: {
  config: ConnectionsConfig;
  providers: ProviderInfo[];
  /** Focus the first provider tile (the last model was just removed). */
  autoFocus: boolean;
  /** An export or import is in flight. */
  importing: boolean;
  /** "Import your setup": the header's Import… flow. */
  onImport: () => void;
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
          const name = providerName(findProvider(providers, model.provider), model);
          if (tested) toast.success(`Connected to ${name}`);
          else toast.success(`Saved without confirming — ${name} didn’t answer the test.`);
        }}
      />
      {stage === "pick" ? (
        <div className="mt-4 border-t border-border pt-3 text-[12px] leading-5">
          <span className="text-muted-foreground">Coming from another computer?</span>{" "}
          <button
            type="button"
            aria-disabled={importing || undefined}
            onClick={onImport}
            className="rounded-md font-medium text-primary outline-none hover:underline focus-visible:ring-[3px] focus-visible:ring-ring/50 aria-disabled:pointer-events-none aria-disabled:opacity-60"
          >
            Import your setup
          </button>
        </div>
      ) : null}
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
