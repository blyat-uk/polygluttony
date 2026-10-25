import { useCallback } from "react";
import { create } from "zustand";
import type { Model } from "@/types/generated/Model";
import type { TestResult } from "@/types/generated/TestResult";
import { ipc } from "@/lib/ipc";
import { isUsable } from "./connection-model";
import { commandFailure } from "./error-copy";

/** What this session knows about a model: nothing yet, a test in flight, or the last
 *  test's answer. Kept for the session only — a saved key isn't proof the model works. */
export type ModelTest =
  | { state: "untested" }
  | { state: "testing" }
  | { state: "works"; seconds: number }
  | { state: "failed"; result: TestResult };

/** How a library model reads: "Needs an API key", "Ready" (key saved, not tested this
 *  session), "Testing…", "Connected" (a test passed) or "Didn't work". */
export type ModelStatus = "needs-key" | "ready" | "testing" | "connected" | "failed";

/** Tests are about where a model is reached, so they key on that: editing the key, the
 *  model or the address makes it untested again. */
export function testKey(m: Pick<Model, "provider" | "model" | "base_url" | "api_key">): string {
  return [m.provider, m.model.trim().toLowerCase(), m.base_url.trim(), m.api_key.trim()].join("\n");
}

export function modelStatus(model: Model, test: ModelTest): ModelStatus {
  if (!isUsable(model)) return "needs-key";
  switch (test.state) {
    case "testing":
      return "testing";
    case "works":
      return "connected";
    case "failed":
      return "failed";
    default:
      return "ready";
  }
}

/** 1.43 → "1.4 s", 0.21 → "0.2 s", 12.6 → "13 s". */
export function formatLatency(seconds: number): string {
  return seconds < 10 ? `${seconds.toFixed(1)} s` : `${Math.round(seconds)} s`;
}

const UNTESTED: ModelTest = { state: "untested" };

const useTests = create<{
  byKey: Record<string, ModelTest>;
  set: (key: string, test: ModelTest) => void;
}>((set) => ({
  byKey: {},
  set: (key, test) => set((s) => ({ byKey: { ...s.byKey, [key]: test } })),
}));

/** A test passed elsewhere (the setup form's Connect): remember it for the session. */
export function recordWorks(model: Pick<Model, "provider" | "model" | "base_url" | "api_key">, seconds: number) {
  useTests.getState().set(testKey(model), { state: "works", seconds });
}

/** This session's test of `model`, and `run` to test it again (nothing is saved). */
export function useModelTest(model: Model | undefined): {
  test: ModelTest;
  run: () => Promise<void>;
} {
  const key = model ? testKey(model) : "";
  const test = useTests((s) => (key ? (s.byKey[key] ?? UNTESTED) : UNTESTED));
  const run = useCallback(async () => {
    if (!model) return;
    const { set, byKey } = useTests.getState();
    if (byKey[key]?.state === "testing") return;
    set(key, { state: "testing" });
    const started = performance.now();
    let result: TestResult;
    try {
      result = await ipc.testConnection(model);
    } catch (e) {
      result = commandFailure(model, e);
    }
    const seconds = (performance.now() - started) / 1000;
    set(key, result.ok ? { state: "works", seconds } : { state: "failed", result });
  }, [model, key]);
  return { test, run };
}
