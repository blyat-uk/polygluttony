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
    name: "Extract names",
    purpose: "Finds names and terms in your subtitles",
    phase: "glossary",
    tuning: ["batch_dialogue_limit", "concurrency", "timeout", "max_tokens"],
  },
  normalize: {
    name: "Normalize",
    purpose: "Merges duplicates, fixes spellings",
    phase: "glossary",
    tuning: ["concurrency", "timeout", "max_tokens"],
  },
  "web-search": {
    name: "Web search",
    purpose: "Looks up names fans already use",
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
    purpose: "Checks each file, flags lines to redo",
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

/** "Extract names and Translate", "Extract names, Normalize and Verify". */
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

/** Model colours, by `Model.color` (fixed when the model joins the library). */
const MODEL_PALETTE = [
  "var(--color-gold)",
  "#c9a2ff", // violet
  "var(--color-aqua)",
  "var(--color-jade)",
  "#f2a0c4", // rose
  "#8fb4ff", // sky
] as const;

export function modelColor(model: Pick<Model, "color"> | undefined): string {
  if (!model) return "var(--color-ink-dim)";
  return MODEL_PALETTE[model.color % MODEL_PALETTE.length];
}
