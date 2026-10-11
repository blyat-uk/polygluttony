import type { ConnectionsConfig } from "@/types/generated/ConnectionsConfig";
import type { Model } from "@/types/generated/Model";
import type { ModelCapabilities } from "@/types/generated/ModelCapabilities";
import type { Provider } from "@/types/generated/Provider";
import type { ProviderInfo } from "@/types/generated/ProviderInfo";
import type { StepSettings } from "@/types/generated/StepSettings";
import type { ThinkingLevel } from "@/types/generated/ThinkingLevel";
import type { TuningDefaults } from "@/types/generated/TuningDefaults";

/** Pure helpers for the Connections page: labels, usability, new drafts. */

/** What the setup form is connecting: any model, or one that must be able to search. */
export type Role = "model" | "web-search";

/** How much setup a provider needs: a key (cloud), an address (local), or both (custom). */
export type ProviderKind = "cloud" | "local" | "custom";

export function findProvider(
  providers: ProviderInfo[] | undefined,
  provider: Provider,
): ProviderInfo | undefined {
  return providers?.find((p) => p.provider === provider);
}

export function providerKind(info: ProviderInfo): ProviderKind {
  if (info.local) return "local";
  return info.provider === "custom" ? "custom" : "cloud";
}

/** Curated label for a model id, else the id itself (mirrors `providers::model_label`). */
export function modelLabel(info: ProviderInfo | undefined, model: string): string {
  const id = model.toLowerCase();
  return info?.models.find((m) => m.id.toLowerCase() === id)?.label ?? model;
}

/** `host[:port]` of a URL, or the input when it doesn't parse. */
export function hostOf(url: string): string {
  try {
    return new URL(url).host || url;
  } catch {
    return url;
  }
}

export function isHttpUrl(url: string): boolean {
  try {
    const u = new URL(url);
    return u.protocol === "http:" || u.protocol === "https:";
  } catch {
    return false;
  }
}

const LOOPBACK = /^(localhost|127(?:\.\d{1,3}){3}|\[::1\]|0\.0\.0\.0)$/i;

function isLoopbackUrl(url: string): boolean {
  try {
    return LOOPBACK.test(new URL(url).hostname);
  } catch {
    return false;
  }
}

/** Mirrors `store::model_is_usable`: a key, a local provider, a Custom
 *  endpoint (its key is optional), or a localhost URL. */
export function isUsable(conn: Pick<Model, "provider" | "api_key" | "base_url">): boolean {
  return (
    conn.api_key.trim() !== "" ||
    conn.provider === "ollama" ||
    conn.provider === "lmstudio" ||
    conn.provider === "custom" ||
    isLoopbackUrl(conn.base_url)
  );
}

export function isCloud(provider: Provider): boolean {
  return provider === "anthropic" || provider === "openai" || provider === "google";
}

/** Mirrors the backend's removal rule: removing `model` from the library forgets the saved
 *  key for its (cloud) provider, unless another model that stays still uses that provider. */
export function removalForgetsKey(
  model: Model,
  remaining: readonly Model[],
  providerKeys: Record<string, string>,
): boolean {
  const hasKey = !!providerKeys[model.provider] || model.api_key.trim() !== "";
  return (
    isCloud(model.provider) &&
    hasKey &&
    !remaining.some((m) => m.id !== model.id && m.provider === model.provider)
  );
}

/** The name a person recognises: the provider label, or the host for a custom endpoint. */
export function providerName(
  info: ProviderInfo | undefined,
  conn: Pick<Model, "provider" | "base_url">,
): string {
  if (conn.provider === "custom") return hostOf(conn.base_url) || "Custom endpoint";
  return info?.label ?? conn.provider;
}

/** A fresh library model for `info`: its address and recommended model, plus a saved key
 *  if any. `id` stays empty until the backend assigns one. */
export function newModel(info: ProviderInfo, providerKeys: Record<string, string>): Model {
  return {
    id: "",
    provider: info.provider,
    // Custom has no fixed format: "openai" is a placeholder until Connect detects it.
    driver: info.driver ?? "openai",
    base_url: info.base_url,
    api_key: providerKeys[info.provider] ?? "",
    model: info.default_model,
    color: 0,
    connect_timeout: null,
  };
}

/** Mirrors `providers::normalize_thinking`: keep `level` if the model offers it, otherwise
 *  the model's default — so a model with a default always has a level selected
 *  (`null` = nothing sent, the model decides). */
export function normaliseThinking(
  level: ThinkingLevel | null,
  caps: ModelCapabilities,
): ThinkingLevel | null {
  return level !== null && caps.thinking_levels.includes(level) ? level : caps.default_thinking;
}

/** `sk-ant-…3f9a` — enough to recognise a key without showing it. */
export function maskKey(key: string): string {
  const k = key.trim();
  if (k.length <= 10) return "•".repeat(Math.max(4, k.length));
  const prefix = /^[A-Za-z]+-(?:[A-Za-z]+-)?/.exec(k)?.[0] ?? k.slice(0, 4);
  return `${prefix.slice(0, 8)}…${k.slice(-4)}`;
}

export const TUNING_FIELDS = ["batch_dialogue_limit", "concurrency", "timeout", "max_tokens"] as const;
export type TuningField = (typeof TUNING_FIELDS)[number];

export type Tuning = Pick<StepSettings, TuningField>;

/** Saved tuning values, with nulls filled from the provider's defaults. */
export function effectiveTuning(conn: Tuning, defaults: TuningDefaults): TuningDefaults {
  return {
    batch_dialogue_limit: conn.batch_dialogue_limit ?? defaults.batch_dialogue_limit,
    concurrency: conn.concurrency ?? defaults.concurrency,
    timeout: conn.timeout ?? defaults.timeout,
    max_tokens: conn.max_tokens ?? defaults.max_tokens,
  };
}

/** 180 → "3 min", 90 → "90 s". */
export function formatSeconds(s: number): string {
  return s >= 120 && s % 60 === 0 ? `${s / 60} min` : `${s} s`;
}

/** What the shell needs to know: the Translate step's model label (plus how many other
 *  models the steps use, e.g. "Claude Sonnet 5.5 +3"), and whether Translate can run. */
export function translationStatus(
  cfg: ConnectionsConfig,
  providers: ProviderInfo[] | undefined,
): { label: string | null; usable: boolean } {
  const steps = cfg.steps;
  const model = steps ? cfg.models.find((m) => m.id === steps.translate.model) : undefined;
  if (!steps || !model) return { label: null, usable: false };
  const verify = cfg.models.find((m) => m.id === steps.verify.model);
  const used = new Set(
    [steps.extract, steps.normalize, steps.web_search, steps.translate, steps.verify]
      .filter((s) => s !== null)
      .map((s) => s.model),
  );
  const others = cfg.per_step ? used.size - 1 : 0;
  const label = modelLabel(findProvider(providers, model.provider), model.model);
  return {
    label: others > 0 ? `${label} +${others}` : label,
    usable: isUsable(model) && !!verify && isUsable(verify),
  };
}
