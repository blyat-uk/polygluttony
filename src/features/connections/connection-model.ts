import type { Connection } from "@/types/generated/Connection";
import type { ConnectionsConfig } from "@/types/generated/ConnectionsConfig";
import type { ModelCapabilities } from "@/types/generated/ModelCapabilities";
import type { Provider } from "@/types/generated/Provider";
import type { ProviderInfo } from "@/types/generated/ProviderInfo";
import type { ThinkingLevel } from "@/types/generated/ThinkingLevel";
import type { TuningDefaults } from "@/types/generated/TuningDefaults";

/** Pure helpers for the Connections page: labels, usability, new drafts. */

/** Which job a connection does on this page. */
export type Role = "translation" | "web-search";

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

/** Mirrors `store::connection_is_usable`: a key, or a model on this computer. */
export function isUsable(conn: Connection): boolean {
  return (
    conn.api_key.trim() !== "" ||
    conn.provider === "ollama" ||
    conn.provider === "lmstudio" ||
    isLoopbackUrl(conn.base_url)
  );
}

/** The name a person recognises: the provider label, or the host for a custom endpoint. */
export function providerName(info: ProviderInfo | undefined, conn: Connection): string {
  if (conn.provider === "custom") return hostOf(conn.base_url) || "Custom endpoint";
  return info?.label ?? conn.provider;
}

/** A fresh draft for `info`: its address and recommended model, plus a saved key if any.
 *  Thinking and tuning stay null so the backend/engine defaults apply. */
export function newConnection(
  info: ProviderInfo,
  providerKeys: Record<string, string>,
): Connection {
  return {
    provider: info.provider,
    // Custom has no fixed format: "openai" is a placeholder until Connect detects it.
    driver: info.driver ?? "openai",
    base_url: info.base_url,
    api_key: providerKeys[info.provider] ?? "",
    model: info.default_model,
    thinking: null,
    max_tokens: null,
    batch_dialogue_limit: null,
    timeout: null,
    connect_timeout: null,
    concurrency: null,
  };
}

/** Keep `level` if the model offers it; otherwise fall back to the model's default.
 *  `null` (model default, nothing sent) is always kept. */
export function normaliseThinking(
  level: ThinkingLevel | null,
  caps: ModelCapabilities,
): ThinkingLevel | null {
  if (caps.thinking_levels.length === 0 || level === null) return null;
  return caps.thinking_levels.includes(level) ? level : caps.default_thinking;
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

/** Saved tuning values, with nulls filled from the provider's defaults. */
export function effectiveTuning(conn: Connection, defaults: TuningDefaults): TuningDefaults {
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

/** What the store needs to know: the translation model's label, and whether Translate can run. */
export function translationStatus(
  cfg: ConnectionsConfig,
  providers: ProviderInfo[] | undefined,
): { label: string | null; usable: boolean } {
  const t = cfg.translation;
  if (!t) return { label: null, usable: false };
  return { label: modelLabel(findProvider(providers, t.provider), t.model), usable: isUsable(t) };
}
