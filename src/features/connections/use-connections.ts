import { useEffect } from "react";
import {
  keepPreviousData,
  queryOptions,
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";
import { toast } from "sonner";
import type { ConnectionsConfig } from "@/types/generated/ConnectionsConfig";
import type { Model } from "@/types/generated/Model";
import type { Provider } from "@/types/generated/Provider";
import type { ProviderInfo } from "@/types/generated/ProviderInfo";
import type { Step } from "@/types/generated/Step";
import type { StepSettings } from "@/types/generated/StepSettings";
import { ipc } from "@/lib/ipc";
import { useAppStore } from "@/stores/app-store";
import { translationStatus } from "./connection-model";
import { TEXT_STEPS, withStep } from "./steps";

export const connectionsKey = ["connections"] as const;
/** Glossary's "Look up names online" gates on this; every connection change refreshes it. */
export const webSearchStatusKey = ["web-search-status"] as const;

export const connectionsQuery = queryOptions({
  queryKey: connectionsKey,
  queryFn: ipc.getConnections,
});

export const providersQuery = queryOptions({
  queryKey: ["providers"] as const,
  queryFn: ipc.listProviders,
  staleTime: Infinity,
});

export function capabilitiesQuery(provider: Provider, model: string) {
  return queryOptions({
    queryKey: ["model-capabilities", provider, model] as const,
    queryFn: () => ipc.modelCapabilities(provider, model),
    staleTime: Infinity,
  });
}

/** Push the Translate model's label + usability into the shell store (status bar, rail, Translate). */
export function syncConnectionStore(cfg: ConnectionsConfig, providers: ProviderInfo[] | undefined) {
  const { label, usable } = translationStatus(cfg, providers);
  useAppStore.getState().setTranslationStatus(label, usable);
}

export function useProviders() {
  return useQuery(providersQuery);
}

/** The page state, kept in step with the shell store whenever it changes. */
export function useConnectionsConfig() {
  const query = useQuery(connectionsQuery);
  const { data: providers } = useProviders();
  useEffect(() => {
    if (query.data && providers) syncConnectionStore(query.data, providers);
  }, [query.data, providers]);
  return query;
}

/** What a provider + model supports. Keeps the previous answer while a new model loads,
 *  so the Thinking / Web search rows don't blink out and back. */
export function useCapabilities(provider: Provider | undefined, model: string | undefined) {
  const p = provider ?? "custom";
  const m = model?.trim() ?? "";
  return useQuery({
    ...capabilitiesQuery(p, m),
    enabled: !!provider && m !== "",
    placeholderData: keepPreviousData,
  });
}

/** Live model ids from the provider. Errors when it can't be reached; `[]` = it answered
 *  with no models. Cloud and custom callers ignore the error and keep the curated list. */
export function useLiveModels(conn: Model | null, enabled: boolean) {
  return useQuery({
    queryKey: ["live-models", conn?.provider, conn?.base_url, conn?.api_key] as const,
    queryFn: () => ipc.listModels(conn as Model),
    enabled: enabled && !!conn,
    staleTime: 60_000,
    retry: false,
  });
}

/** Optimistically patch the cached page state; returns the previous value for rollback. */
async function patchCache(
  qc: QueryClient,
  patch: (cfg: ConnectionsConfig) => ConnectionsConfig,
): Promise<{ prev: ConnectionsConfig | undefined }> {
  await qc.cancelQueries({ queryKey: connectionsKey });
  const prev = qc.getQueryData<ConnectionsConfig>(connectionsKey);
  if (prev) qc.setQueryData(connectionsKey, patch(prev));
  return { prev };
}

/** Mirrors `store::set_shared_steps` for the optimistic cache patch. */
function withShared(cfg: ConnectionsConfig, settings: StepSettings): ConnectionsConfig {
  const steps = cfg.steps;
  if (!steps) return cfg;
  const sharedModel = steps.translate.model;
  let next = steps;
  for (const step of TEXT_STEPS) next = withStep(next, step, settings);
  if (steps.web_search?.model === sharedModel) next = { ...next, web_search: settings };
  return { ...cfg, steps: next };
}

export function useConnectionMutations() {
  const qc = useQueryClient();
  // Every mutation returns the whole normalised state: write it straight into the cache.
  const apply = (cfg: ConnectionsConfig) => {
    qc.setQueryData(connectionsKey, cfg);
    void qc.invalidateQueries({ queryKey: webSearchStatusKey });
  };
  const rollback = (e: unknown, ctx: { prev: ConnectionsConfig | undefined } | undefined) => {
    if (ctx?.prev) qc.setQueryData(connectionsKey, ctx.prev);
    // The snapshot predates any save that landed meanwhile: re-read the real state.
    void qc.invalidateQueries({ queryKey: connectionsKey });
    toast.error(`Couldn't save: ${String(e)}`);
  };

  return {
    /** From the setup / edit form, after a test. Errors surface in the form. */
    saveModel: useMutation({
      mutationFn: ({ model, assign }: { model: Model; assign?: Step }) => ipc.saveModel(model, assign),
      onSuccess: apply,
    }),
    /** Inline change to a saved library model (the one-model card's model menu): optimistic. */
    updateModel: useMutation({
      mutationFn: (model: Model) => ipc.saveModel(model),
      onMutate: (model) =>
        patchCache(qc, (cfg) => ({
          ...cfg,
          models: cfg.models.map((m) => (m.id === model.id ? model : m)),
        })),
      onError: (e, _v, ctx) => rollback(e, ctx),
      onSuccess: apply,
    }),
    /** After a confirm dialog. */
    removeModel: useMutation({ mutationFn: ipc.removeModel, onSuccess: apply }),
    /** A step's model, Thinking or Tuning; web search on/off: optimistic. */
    setStep: useMutation({
      mutationFn: ({ step, settings }: { step: Step; settings: StepSettings | null }) =>
        ipc.setStep(step, settings),
      onMutate: ({ step, settings }) =>
        patchCache(qc, (cfg) =>
          cfg.steps ? { ...cfg, steps: withStep(cfg.steps, step, settings) } : cfg,
        ),
      onError: (e, _v, ctx) => rollback(e, ctx),
      onSuccess: apply,
    }),
    /** The one-model card's Thinking / Tuning: optimistic. */
    setSharedSteps: useMutation({
      mutationFn: ipc.setSharedSteps,
      onMutate: (settings) => patchCache(qc, (cfg) => withShared(cfg, settings)),
      onError: (e, _v, ctx) => rollback(e, ctx),
      onSuccess: apply,
    }),
    /** "Use different models for different steps": optimistic. */
    showEveryStep: useMutation({
      mutationFn: ipc.showEveryStep,
      onMutate: () => patchCache(qc, (cfg) => ({ ...cfg, per_step: true })),
      onError: (e, _v, ctx) => rollback(e, ctx),
      onSuccess: apply,
    }),
    /** "Use one model for everything", after its dialog. */
    useOneModel: useMutation({ mutationFn: ipc.useOneModel, onSuccess: apply }),
  };
}
