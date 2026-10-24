import { useQueries } from "@tanstack/react-query";
import type { ConnectionsConfig } from "@/types/generated/ConnectionsConfig";
import type { Model } from "@/types/generated/Model";
import type { ModelCapabilities } from "@/types/generated/ModelCapabilities";
import type { ProviderInfo } from "@/types/generated/ProviderInfo";
import type { Step } from "@/types/generated/Step";
import { findProvider, modelLabel, providerName, removalForgetsKey } from "./connection-model";
import { listNames, STEP_ORDER, stepSettings, stepsUsing, TEXT_STEPS } from "./steps";
import { capabilitiesQuery } from "./use-connections";

/** Per-step view helpers: library capabilities, and what a removal or "use one model" will
 *  do — mirroring the backend's command rules, so the confirm dialogs can say it in words. */

export type LibraryCaps = {
  /** By `Model.id`; undefined while that model's answer loads. */
  byId: ReadonlyMap<string, ModelCapabilities | undefined>;
  /** Every model has answered (or failed). */
  settled: boolean;
};

/** What each library model supports, in one batch of cached queries. */
export function useLibraryCapabilities(models: readonly Model[]): LibraryCaps {
  return useQueries({
    queries: models.map((m) => capabilitiesQuery(m.provider, m.model)),
    combine: (results) => ({
      byId: new Map(models.map((m, i) => [m.id, results[i]?.data])),
      settled: results.every((r) => r.data !== undefined || r.isError),
    }),
  });
}

/** Unknown capabilities count as "can't search": the backend checks again anyway. */
export function canSearch(caps: LibraryCaps, model: Model): boolean {
  return caps.byId.get(model.id)?.web_search ?? false;
}

export function labelOf(model: Model, providers: ProviderInfo[]): string {
  return modelLabel(findProvider(providers, model.provider), model.model);
}

export { listNames };

export type RemovalOutcome =
  | { kind: "last" }
  | {
      kind: "moves";
      /** First remaining model: where the removed model's steps go. */
      fallback: Model;
      /** Steps moving to `fallback`, in workflow order (web search too when it lands there). */
      moved: Step[];
      /** After the move every step that's on runs on `fallback`. */
      everyStep: boolean;
      /** Web search was on the removed model and lands on another model, or turns off. */
      webSearch: { to: Model } | "off" | null;
    };

/** Mirrors `remove_model`: steps on it → the first remaining model; web search on it → the
 *  first remaining model that can search, else off. */
export function removalOutcome(
  cfg: ConnectionsConfig,
  modelId: string,
  caps: LibraryCaps,
): RemovalOutcome {
  const remaining = cfg.models.filter((m) => m.id !== modelId);
  const fallback = remaining[0];
  if (!fallback) return { kind: "last" };

  const steps = cfg.steps;
  const moved: Step[] = TEXT_STEPS.filter((s) => steps && stepSettings(steps, s)?.model === modelId);
  let webSearch: { to: Model } | "off" | null = null;
  if (steps?.web_search?.model === modelId) {
    const to = remaining.find((m) => canSearch(caps, m));
    if (!to) webSearch = "off";
    else if (to.id === fallback.id) moved.push("web-search");
    else webSearch = { to };
  }
  moved.sort((a, b) => STEP_ORDER.indexOf(a) - STEP_ORDER.indexOf(b));

  // Every step that stays on ends up on the fallback model.
  const everyStep =
    !!steps &&
    moved.length > 0 &&
    webSearch === null &&
    STEP_ORDER.every((s) => {
      const settings = stepSettings(steps, s);
      return settings === null || settings.model === modelId || settings.model === fallback.id;
    });
  return { kind: "moves", fallback, moved, everyStep, webSearch };
}

/** Provider names whose saved key goes when `leaving` leave the library and `staying` stay. */
export function forgottenKeys(
  leaving: readonly Model[],
  staying: readonly Model[],
  cfg: ConnectionsConfig,
  providers: ProviderInfo[],
): string[] {
  const names: string[] = [];
  for (const m of leaving) {
    if (!removalForgetsKey(m, staying, cfg.provider_keys)) continue;
    const name = providerName(findProvider(providers, m.provider), m);
    if (!names.includes(name)) names.push(name);
  }
  return names;
}

/** "Your saved Anthropic API key will be forgotten too." — or null when no key goes. */
export function forgottenKeysSentence(names: readonly string[]): string | null {
  if (names.length === 0) return null;
  return names.length === 1
    ? `Your saved ${names[0]} API key will be forgotten too.`
    : `Your saved ${listNames(names)} API keys will be forgotten too.`;
}

export type OneModelOutcome = {
  /** A text step already runs on the chosen model, so its settings carry over. */
  keepsSettings: boolean;
  /** Web search stays on this other model (the chosen one can't search). */
  webSearchStaysOn: Model | null;
  /** Models no step will use any more. They stay in the library, keys and all. */
  idle: Model[];
};

/** Mirrors `use_one_model`: only the steps move; no model or key leaves. */
export function oneModelOutcome(
  cfg: ConnectionsConfig,
  chosen: Model,
  caps: LibraryCaps,
): OneModelOutcome {
  const ws = cfg.steps?.web_search ?? null;
  const stays =
    ws && ws.model !== chosen.id && !canSearch(caps, chosen)
      ? (cfg.models.find((m) => m.id === ws.model) ?? null)
      : null;
  const used = stays ? [chosen, stays] : [chosen];
  return {
    keepsSettings: stepsUsing(cfg, chosen.id).some((s) => s !== "web-search"),
    webSearchStaysOn: stays,
    idle: cfg.models.filter((m) => !used.some((k) => k.id === m.id)),
  };
}
