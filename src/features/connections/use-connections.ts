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
import type { Connection } from "@/types/generated/Connection";
import type { ConnectionsConfig } from "@/types/generated/ConnectionsConfig";
import type { Provider } from "@/types/generated/Provider";
import type { ProviderInfo } from "@/types/generated/ProviderInfo";
import type { WebSearch } from "@/types/generated/WebSearch";
import { ipc } from "@/lib/ipc";
import { useAppStore } from "@/stores/app-store";
import { translationStatus } from "./connection-model";

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

/** Push the translation label + usability into the shell store (status bar, rail, Translate). */
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
export function useLiveModels(conn: Connection | null, enabled: boolean) {
  return useQuery({
    queryKey: ["live-models", conn?.provider, conn?.base_url, conn?.api_key] as const,
    queryFn: () => ipc.listModels(conn as Connection),
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

export function useConnectionMutations() {
  const qc = useQueryClient();
  // Every mutation returns the whole normalised state: write it straight into the cache.
  const apply = (cfg: ConnectionsConfig) => {
    qc.setQueryData(connectionsKey, cfg);
    void qc.invalidateQueries({ queryKey: webSearchStatusKey });
  };
  const rollback = (e: unknown, ctx: { prev: ConnectionsConfig | undefined } | undefined) => {
    if (ctx?.prev) qc.setQueryData(connectionsKey, ctx.prev);
    toast.error(`Couldn't save: ${String(e)}`);
  };

  return {
    /** From the setup / edit form, after a test. Errors surface in the form. */
    saveTranslation: useMutation({ mutationFn: ipc.saveTranslation, onSuccess: apply }),
    /** Inline changes on the connected card (model, thinking, tuning): optimistic. */
    updateTranslation: useMutation({
      mutationFn: ipc.saveTranslation,
      onMutate: (connection) => patchCache(qc, (cfg) => ({ ...cfg, translation: connection })),
      onError: (e, _v, ctx) => rollback(e, ctx),
      onSuccess: apply,
    }),
    removeTranslation: useMutation({ mutationFn: ipc.removeTranslation, onSuccess: apply }),
    /** Separate web-search model from the form. Errors surface in the form. */
    saveWebSearch: useMutation({ mutationFn: ipc.setWebSearch, onSuccess: apply }),
    /** The switch / Remove: optimistic. */
    updateWebSearch: useMutation({
      mutationFn: (webSearch: WebSearch) => ipc.setWebSearch(webSearch),
      onMutate: (webSearch) => patchCache(qc, (cfg) => ({ ...cfg, web_search: webSearch })),
      onError: (e, _v, ctx) => rollback(e, ctx),
      onSuccess: apply,
    }),
  };
}
