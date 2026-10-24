import type { ConnectionsConfig } from "@/types/generated/ConnectionsConfig";
import type { Model } from "@/types/generated/Model";
import type { Step } from "@/types/generated/Step";
import type { StepSettings } from "@/types/generated/StepSettings";
import type { Steps } from "@/types/generated/Steps";
import type { TuningField } from "./connection-model";

/** The five LLM steps: names, order, and which Tuning fields each one actually uses. */

/** Workflow order — the order the page lists them in. */
export const STEP_ORDER: readonly Step[] = ["extract", "normalize", "web-search", "translate", "verify"];

/** The steps that share one model in the one-model view (web search is opt-in on top). */
export const TEXT_STEPS: readonly Step[] = ["extract", "normalize", "translate", "verify"];

export type Phase = "glossary" | "translation";

export const STEP_INFO: Record<
  Step,
  { name: string; purpose: string; phase: Phase; tuning: readonly TuningField[] }
> = {
  extract: {
    name: "Find names",
    purpose: "Finds names and terms in your subtitles",
    phase: "glossary",
    tuning: ["batch_dialogue_limit", "concurrency", "timeout", "max_tokens"],
  },
  normalize: {
    name: "Tidy up",
    purpose: "Fixes spellings and drops generic terms",
    phase: "glossary",
    tuning: ["concurrency", "timeout", "max_tokens"],
  },
  "web-search": {
    name: "Look up names online",
    purpose: "Finds the names fans already use",
    phase: "glossary",
    // One request per glossary build.
    tuning: ["timeout", "max_tokens"],
  },
  translate: {
    name: "Translate",
    purpose: "Every line, plus clean-up passes",
    phase: "translation",
    tuning: ["batch_dialogue_limit", "concurrency", "timeout", "max_tokens"],
  },
  verify: {
    name: "Verify",
    purpose: "Checks each episode and redoes lines that slipped",
    phase: "translation",
    // Verifies files as Translate finishes them, so it runs as many at once as Translate.
    tuning: ["timeout", "max_tokens"],
  },
};

export const PHASE_LABEL: Record<Phase, string> = { glossary: "Glossary", translation: "Translation" };

/** `Steps` field for a step id ("web-search" → `web_search`). */
export const STEP_KEY: Record<Step, keyof Steps> = {
  extract: "extract",
  normalize: "normalize",
  "web-search": "web_search",
  translate: "translate",
  verify: "verify",
};

export function stepSettings(steps: Steps, step: Step): StepSettings | null {
  return steps[STEP_KEY[step]];
}

/** A copy of `steps` with one step replaced (`null` only makes sense for web search). */
export function withStep(steps: Steps, step: Step, settings: StepSettings | null): Steps {
  return { ...steps, [STEP_KEY[step]]: settings } as Steps;
}

export function modelById(cfg: ConnectionsConfig, id: string | undefined): Model | undefined {
  return id === undefined ? undefined : cfg.models.find((m) => m.id === id);
}

/** The steps (in workflow order) that run on `modelId`. */
export function stepsUsing(cfg: ConnectionsConfig, modelId: string): Step[] {
  const steps = cfg.steps;
  if (!steps) return [];
  return STEP_ORDER.filter((s) => stepSettings(steps, s)?.model === modelId);
}

/** "A", "A and B", "A, B and C". */
export function listNames(names: readonly string[]): string {
  if (names.length <= 1) return names[0] ?? "";
  return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
}

/** "Find names and Translate", "Find names, Tidy up and Verify". */
export function stepNames(steps: Step[]): string {
  return listNames(steps.map((s) => STEP_INFO[s].name));
}

/** Fresh settings for `modelId`: the model's own Thinking default and provider tuning. */
export function defaultStepSettings(modelId: string): StepSettings {
  return {
    model: modelId,
    thinking: null,
    max_tokens: null,
    batch_dialogue_limit: null,
    timeout: null,
    concurrency: null,
  };
}

/** Model colours, by `Model.color` (fixed when the model joins the library). Warm neutral
 *  tints only: a model's dot must never read as a state (gold live, aqua verifying, jade
 *  done, amber needs a look, coral failed, muted waiting), nor as violet, rose or sky. */
const MODEL_PALETTE = [
  "#d9b98a", // sand
  "#b08a5a", // bronze
  "#e8d7b5", // parchment
  "#a8998a", // stone
  "#7f6c5c", // umber
  "#cfc6b8", // ash
] as const;

export function modelColor(model: Pick<Model, "color"> | undefined): string {
  if (!model) return "var(--color-ink-dim)";
  return MODEL_PALETTE[model.color % MODEL_PALETTE.length];
}
