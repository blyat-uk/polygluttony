import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ArrowsLeftRight, Plus } from "@phosphor-icons/react";
import { toast } from "sonner";
import type { ConnectionsConfig } from "@/types/generated/ConnectionsConfig";
import type { Model } from "@/types/generated/Model";
import type { ModelCapabilities } from "@/types/generated/ModelCapabilities";
import type { ProviderInfo } from "@/types/generated/ProviderInfo";
import type { StepSettings } from "@/types/generated/StepSettings";
import type { Steps } from "@/types/generated/Steps";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";
import {
  findProvider,
  hostOf,
  isUsable,
  modelLabel,
  providerKind,
  providerName,
  removalForgetsKey,
} from "./connection-model";
import { ConnectionForm, type FormStage } from "./connection-form";
import { ModelPicker } from "./model-picker";
import { ModelStatusDot, TestButton, TestOutcome } from "./model-test";
import { ProviderMark } from "./provider-tiles";
import {
  CardEyebrow,
  OptionRow,
  RoleCard,
  SaveFeedback,
  StatusDot,
  useSaveFeedback,
} from "./role-card";
import { modelById, stepsUsing } from "./steps";
import { modelStatus, useModelTest } from "./test-status";
import { ThinkingControl, thinkingHelp } from "./thinking-control";
import { TuningSection } from "./tuning-section";
import { connectionsKey, useConnectionMutations, useLiveModels } from "./use-connections";

/** What the card shows: its summary, the model's edit form, or the web-search form inline. */
type Mode = "summary" | "edit" | "search-add" | "search-edit";

const LINK = "h-auto px-0 py-0.5 text-[12px]";

/**
 * The one-model view: one model does every step. Model, Thinking, Web search and Tuning save
 * as soon as they change; Thinking and Tuning go to every text step at once.
 */
export function OneModelCard({
  config,
  steps,
  model,
  providers,
  caps,
  focusEdit = false,
  onRemoving,
}: {
  config: ConnectionsConfig;
  steps: Steps;
  /** The model the text steps share (Translate's). */
  model: Model;
  providers: ProviderInfo[];
  /** Its capabilities; undefined only when they couldn't be loaded. */
  caps: ModelCapabilities | undefined;
  /** Focus Edit: the model was just connected, or took over from a removed one. */
  focusEdit?: boolean;
  /** As this model's removal starts; `remaining`: another model will do every step. */
  onRemoving?: (remaining: boolean) => void;
}) {
  const qc = useQueryClient();
  const titleId = useId();
  const thinkingId = useId();
  const searchId = useId();
  const searchHelpId = useId();
  const m = useConnectionMutations();
  const [mode, setMode] = useState<Mode>("summary");
  const [searchStage, setSearchStage] = useState<FormStage>("pick");
  const [confirm, setConfirm] = useState<"model" | "search" | null>(null);
  // The web-search model the Remove dialog is about; kept while the dialog animates out.
  const [searchTarget, setSearchTarget] = useState<Model | null>(null);
  // Where focus goes once the card is back on its summary, so it isn't dropped.
  const [focusTo, setFocusTo] = useState<"edit" | "search" | null>(null);
  const editRef = useRef<HTMLButtonElement>(null);
  const searchRef = useRef<HTMLDivElement>(null);
  // Live models are fetched the first time the menu opens, then stay cached.
  const [menuOpened, setMenuOpened] = useState(false);
  const live = useLiveModels(model, menuOpened);
  const tested = useModelTest(model);
  const feedback = useSaveFeedback(
    m.updateModel.isPending || m.setSharedSteps.isPending || m.setStep.isPending,
    m.updateModel.isError || m.setSharedSteps.isError || m.setStep.isError,
  );

  useEffect(() => {
    if (focusEdit) setFocusTo("edit");
  }, [focusEdit]);

  useEffect(() => {
    if (mode !== "summary" || !focusTo) return;
    // Wait until this render shows the latest saved state: a row that is about to be
    // replaced would drop focus again.
    if (qc.getQueryData<ConnectionsConfig>(connectionsKey) !== config) return;
    if (focusTo === "edit") editRef.current?.focus();
    else searchRef.current?.querySelector<HTMLElement>("button:not(:disabled)")?.focus();
    setFocusTo(null);
  }, [focusTo, mode, config, qc]);

  const info = findProvider(providers, model.provider);
  const kind = info ? providerKind(info) : "custom";
  const label = modelLabel(info, model.model);
  const shared = steps.translate;
  const levels = caps?.thinking_levels ?? [];

  const ws = steps.web_search;
  const searchModel = ws && ws.model !== model.id ? modelById(config, ws.model) : undefined;
  const searchInfo = searchModel ? findProvider(providers, searchModel.provider) : undefined;
  const searchLabel = searchModel ? modelLabel(searchInfo, searchModel.model) : "";

  // Removing this model: every step moves to the first one left (the backend's rule).
  const successor = config.models.find((x) => x.id !== model.id);
  const forgetsKey = removalForgetsKey(model, config.models, config.provider_keys);
  // Removing the web-search model takes it out of the library only when nothing else uses it.
  const targetInfo = searchTarget ? findProvider(providers, searchTarget.provider) : undefined;
  const targetForgetsKey =
    !!searchTarget &&
    stepsUsing(config, searchTarget.id).every((s) => s === "web-search") &&
    removalForgetsKey(searchTarget, config.models, config.provider_keys);

  // Build on the latest cached settings, not this render's: an earlier save may still be
  // settling, and it mustn't be undone.
  const latestShared = (): StepSettings =>
    qc.getQueryData<ConnectionsConfig>(connectionsKey)?.steps?.translate ?? shared;

  const changeModel = (id: string) => {
    const latest = qc
      .getQueryData<ConnectionsConfig>(connectionsKey)
      ?.models.find((x) => x.id === model.id);
    if (!latest || latest.model === id) return;
    // The backend re-fits each step's Thinking to the new model.
    m.updateModel.mutate({ ...latest, model: id });
  };

  const backToSummary = (focus: "edit" | "search") => {
    setMode("summary");
    setFocusTo(focus);
  };

  const removeModel = async () => {
    // Before the call: the page may swap this card out as soon as the new state lands.
    onRemoving?.(!!successor);
    try {
      await m.removeModel.mutateAsync(model.id);
      toast.success(`${label} removed`);
    } catch (e) {
      toast.error(`Couldn’t remove ${label}: ${String(e)}`);
    }
  };

  const confirmSearchRemoval = () => {
    if (!searchModel) return;
    setSearchTarget(searchModel);
    setConfirm("search");
  };

  const removeSearchModel = async () => {
    if (!searchTarget) return;
    const target = searchTarget;
    const name = modelLabel(targetInfo, target.model);
    let next: ConnectionsConfig;
    try {
      next = await m.setStep.mutateAsync({ step: "web-search", settings: null });
    } catch {
      return; // The mutation already rolled back and said why.
    }
    backToSummary("search");
    if (stepsUsing(next, target.id).length === 0) {
      try {
        await m.removeModel.mutateAsync(target.id);
      } catch (e) {
        toast.error(`Look up names online is off, but ${name} couldn’t be removed: ${String(e)}`);
        return;
      }
    }
    toast.success(`${name} removed. Look up names online is off.`);
  };

  const status = <ModelStatusDot status={modelStatus(model, tested.test)} />;

  // ── web search row ─────────────────────────────────────────────────────────
  const openSearchForm = (next: "search-add" | "search-edit") => {
    // Start on the stage the form opens on, so its label doesn't flash.
    setSearchStage(next === "search-edit" && searchModel ? "fields" : "pick");
    setMode(next);
  };
  const differentModel = (text: string, srText: string, icon = false) => (
    <Button
      variant="link"
      size="sm"
      className={LINK}
      aria-label={srText}
      onClick={() => openSearchForm("search-add")}
    >
      {icon ? <Plus weight="bold" /> : null}
      {text}
    </Button>
  );
  const searchHelp = (
    <span id={searchHelpId}>Finds the names fans already use, for the glossary.</span>
  );

  let searchRow: ReactNode;
  if (mode === "search-add" || mode === "search-edit") {
    const editing = mode === "search-edit" && searchModel ? searchModel : null;
    searchRow = (
      <div className="my-1.5 rounded-xl border border-border px-4 pt-1 pb-4">
        {searchStage === "fields" ? (
          <h3 className="pt-3 text-[11.5px] font-semibold text-[color:var(--color-ink-emphasis)]">
            {editing ? "Model for looking up names online" : "A different model to look up names online"}
          </h3>
        ) : null}
        <ConnectionForm
          key={mode}
          role="web-search"
          providers={providers}
          providerKeys={config.provider_keys}
          initial={editing}
          autoFocus
          onStageChange={setSearchStage}
          onSave={async (next, tested) => {
            await m.saveModel.mutateAsync({ model: next, assign: "web-search" });
            backToSummary("search");
            const nextInfo = findProvider(providers, next.provider);
            toast.success(
              tested
                ? `Look up names online uses ${modelLabel(nextInfo, next.model)} now`
                : `Saved without confirming — ${providerName(nextInfo, next)} didn’t answer the test.`,
            );
          }}
          onCancel={() => backToSummary("search")}
          onRemove={editing ? confirmSearchRemoval : undefined}
        />
      </div>
    );
  } else if (searchModel) {
    // Web search runs on another library model.
    searchRow = (
      <OptionRow
        labelId={searchId}
        label="Look up names online"
        end={
          <>
            <Button
              variant="outline"
              size="sm"
              aria-label={`Edit ${searchLabel}`}
              onClick={() => openSearchForm("search-edit")}
            >
              Edit
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="text-muted-foreground"
              aria-label={`Remove ${searchLabel}`}
              disabled={m.setStep.isPending || m.removeModel.isPending}
              onClick={confirmSearchRemoval}
            >
              Remove
            </Button>
          </>
        }
      >
        <span className="flex min-w-0 flex-1 basis-48 items-center gap-2">
          <ProviderMark info={searchInfo} provider={searchModel.provider} size="sm" />
          <span className="min-w-0 truncate text-[12.5px]">
            {searchLabel}{" "}
            <span className="text-muted-foreground">· {providerName(searchInfo, searchModel)}</span>
          </span>
          {!isUsable(searchModel) ? (
            <span className="shrink-0 whitespace-nowrap">
              <StatusDot tone="amber" hollow>
                needs a key
              </StatusDot>
            </span>
          ) : null}
        </span>
      </OptionRow>
    );
  } else if (ws || caps?.web_search) {
    // This model searches (switch on), or could (switch off).
    searchRow = (
      <OptionRow
        labelId={searchId}
        label="Look up names online"
        help={searchHelp}
        end={differentModel("Use a different model", "Use a different model to look up names online")}
      >
        <Switch
          aria-labelledby={searchId}
          aria-describedby={searchHelpId}
          checked={!!ws}
          onCheckedChange={(on) =>
            m.setStep.mutate({
              step: "web-search",
              settings: on ? { ...latestShared(), model: model.id } : null,
            })
          }
        />
      </OptionRow>
    );
  } else if (caps) {
    searchRow = (
      <OptionRow
        labelId={searchId}
        label="Look up names online"
        end={differentModel("Add a model that can", "Add a model that can look up names online", true)}
      >
        <span className="min-w-0 flex-1 basis-48 text-[12px]">{label} can’t search the web.</span>
      </OptionRow>
    );
  } else {
    // Capabilities couldn't be loaded: don't guess, offer the other route.
    searchRow = (
      <OptionRow
        labelId={searchId}
        label="Look up names online"
        help={searchHelp}
        end={differentModel("Use a different model", "Use a different model to look up names online")}
      />
    );
  }

  return (
    <RoleCard variant="primary" labelledBy={titleId}>
      <CardEyebrow
        title="Your model"
        note="· does every step"
        id={titleId}
        aside={
          <>
            {mode !== "edit" ? <SaveFeedback state={feedback} /> : null}
            {status}
          </>
        }
      />

      {mode === "edit" ? (
        <ConnectionForm
          key="edit"
          role="model"
          providers={providers}
          providerKeys={config.provider_keys}
          initial={model}
          onSave={async (next, tested) => {
            await m.saveModel.mutateAsync({ model: next });
            backToSummary("edit");
            const name = providerName(findProvider(providers, next.provider), next);
            toast.success(
              tested
                ? `Connected to ${name}`
                : `Saved without confirming — ${name} didn’t answer the test.`,
            );
          }}
          onCancel={() => backToSummary("edit")}
          onRemove={() => setConfirm("model")}
        />
      ) : (
        <>
          <div className="mt-3 flex items-center gap-3">
            <ProviderMark info={info} provider={model.provider} size="lg" />
            <div className="min-w-0 flex-1">
              <div className="flex min-w-0 items-baseline gap-2">
                <span
                  className={cn(
                    "truncate text-[14.5px] font-semibold text-[color:var(--color-ink-emphasis)]",
                    // "Ollama" stays whole; its address gives way.
                    kind === "local" && "shrink-0",
                  )}
                >
                  {providerName(info, model)}
                </span>
                {kind === "local" ? (
                  <span className="truncate font-mono text-[11px] text-[color:var(--color-ink-dim)]">
                    {hostOf(model.base_url)}
                  </span>
                ) : kind === "custom" ? (
                  <span className="shrink-0 text-[11px] text-[color:var(--color-ink-dim)]">
                    custom endpoint
                  </span>
                ) : null}
              </div>
              <div className="mt-1 flex min-w-0">
                <ModelPicker
                  variant="inline"
                  ariaLabel="Model"
                  value={model.model}
                  onChange={changeModel}
                  curated={info?.models ?? []}
                  live={live.data}
                  liveLoading={live.isFetching}
                  sourceLabel={kind === "custom" ? "this server" : (info?.label ?? "the provider")}
                  onOpenChange={(open) => {
                    if (open) setMenuOpened(true);
                  }}
                />
              </div>
            </div>
            <div className="flex shrink-0 items-center gap-1.5">
              {isUsable(model) ? (
                <TestButton test={tested.test} onTest={() => void tested.run()} label={label} />
              ) : null}
              <Button
                ref={editRef}
                variant="outline"
                size="sm"
                aria-label={`Edit ${label}`}
                onClick={() => setMode("edit")}
              >
                Edit
              </Button>
              <Button
                variant="ghost"
                size="sm"
                className="text-muted-foreground"
                aria-label={`Remove ${label}`}
                disabled={m.removeModel.isPending}
                onClick={() => setConfirm("model")}
              >
                Remove
              </Button>
            </div>
          </div>

          <TestOutcome test={tested.test} model={model} info={info} className="mt-2.5 ml-[50px]" />

          <div className="mt-3.5 mb-2 h-px bg-border" />

          {levels.length > 0 ? (
            <OptionRow labelId={thinkingId} label="Thinking" help={thinkingHelp(shared.thinking)}>
              <ThinkingControl
                labelledBy={thinkingId}
                levels={levels}
                value={shared.thinking}
                clearable={caps?.default_thinking === null}
                onChange={(thinking) => m.setSharedSteps.mutate({ ...latestShared(), thinking })}
              />
            </OptionRow>
          ) : null}
          <div ref={searchRef}>{searchRow}</div>
          {info ? (
            <TuningSection
              value={shared}
              defaults={info.tuning}
              onSave={async (patch) => {
                try {
                  await m.setSharedSteps.mutateAsync({ ...latestShared(), ...patch });
                  toast.success("Tuning saved");
                } catch {
                  // The mutation already rolled back and reported the error.
                }
              }}
            />
          ) : null}

          <div className="mt-3 border-t border-border pt-3">
            <button
              type="button"
              disabled={m.showEveryStep.isPending}
              onClick={() => m.showEveryStep.mutate()}
              className="group rounded-md text-left text-[12px] leading-5 outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:opacity-60"
            >
              <span className="font-medium text-primary group-hover:underline">
                <ArrowsLeftRight weight="bold" className="mr-1.5 inline size-3.5 -translate-y-px" />
                Use different models for different steps
              </span>{" "}
              <span className="text-muted-foreground">
                · e.g. a stronger model to tidy up, a cheaper one to verify
              </span>
            </button>
          </div>
        </>
      )}

      <ConfirmRemove
        open={confirm === "model"}
        onClose={() => setConfirm(null)}
        title={`Remove ${label}?`}
        onConfirm={() => void removeModel()}
      >
        {successor
          ? `Every step will use ${modelLabel(findProvider(providers, successor.provider), successor.model)} instead.`
          : "You’ll be back to setup, and can’t build a glossary or translate until you connect a model again."}
        {forgetsKey ? ` Your saved ${providerName(info, model)} API key will be forgotten too.` : null}
      </ConfirmRemove>
      <ConfirmRemove
        open={confirm === "search" && !!searchTarget}
        onClose={() => setConfirm(null)}
        title={`Remove ${searchTarget ? modelLabel(targetInfo, searchTarget.model) : "this model"}?`}
        onConfirm={() => void removeSearchModel()}
      >
        Look up names online will turn off.
        {searchTarget && targetForgetsKey
          ? ` Your saved ${providerName(targetInfo, searchTarget)} API key will be forgotten too.`
          : null}
      </ConfirmRemove>
    </RoleCard>
  );
}

function ConfirmRemove({
  open,
  onClose,
  title,
  onConfirm,
  children,
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  onConfirm: () => void;
  children: ReactNode;
}) {
  return (
    <AlertDialog open={open} onOpenChange={(next) => (next ? undefined : onClose())}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{title}</AlertDialogTitle>
          <AlertDialogDescription>{children}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Keep it</AlertDialogCancel>
          <AlertDialogAction variant="destructive" onClick={onConfirm}>
            Remove
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
