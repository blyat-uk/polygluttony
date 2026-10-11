import type { ConnectionsConfig } from "@/types/generated/ConnectionsConfig";
import type { ProviderInfo } from "@/types/generated/ProviderInfo";
import { PageHeader } from "@/components/page-header";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { findProvider, isUsable, modelLabel, providerName } from "./connection-model";
import { FlowStrip, type TranslationNode, type WebSearchNode } from "./flow-strip";
import { TranslationCard } from "./translation-card";
import { WebSearchCard } from "./web-search-card";
import { useCapabilities, useConnectionsConfig, useProviders } from "./use-connections";

/** Two jobs, not a list: ① Translation (required) and ② Web search (optional). */
export function ConnectionsPage() {
  const config = useConnectionsConfig();
  const providers = useProviders();
  const t = config.data?.translation ?? null;
  const caps = useCapabilities(t?.provider, t?.model);
  const translationCaps = t ? caps.data : undefined;
  // Wait for the model's capabilities too, so Thinking / Web search don't pop in after paint.
  const capsSettled = !t || caps.data !== undefined || caps.isError;

  const error = config.error ?? providers.error;
  return (
    <div className="flex min-h-full flex-col">
      <PageHeader
        title="Connections"
        description="The AI that translates your subtitles — and, optionally, one that searches the web for established names."
      />
      <div className="px-5 py-5">
        <div className="mx-auto flex w-full max-w-[700px] flex-col gap-3">
          {config.data && providers.data && capsSettled ? (
            <>
              <FlowStrip
                translation={translationNode(config.data, providers.data)}
                webSearch={webSearchNode(config.data, providers.data)}
              />
              <TranslationCard
                config={config.data}
                providers={providers.data}
                caps={translationCaps}
              />
              <WebSearchCard
                config={config.data}
                providers={providers.data}
                translationCaps={translationCaps}
              />
            </>
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

function translationNode(cfg: ConnectionsConfig, providers: ProviderInfo[]): TranslationNode {
  const t = cfg.translation;
  if (!t) return { state: "none" };
  if (!isUsable(t)) return { state: "needs-key" };
  return { state: "connected", label: modelLabel(findProvider(providers, t.provider), t.model) };
}

function webSearchNode(cfg: ConnectionsConfig, providers: ProviderInfo[]): WebSearchNode {
  const ws = cfg.web_search;
  if (ws.mode === "translation") {
    if (!cfg.translation) return { state: "off" };
    return isUsable(cfg.translation) ? { state: "same" } : { state: "needs-translation" };
  }
  if (ws.mode === "off") return { state: "off" };
  if (!isUsable(ws.connection)) return { state: "needs-key" };
  const info = findProvider(providers, ws.connection.provider);
  return {
    state: "separate",
    label: `${providerName(info, ws.connection)} · ${modelLabel(info, ws.connection.model)}`,
  };
}
