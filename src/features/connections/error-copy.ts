import type { ErrorKind } from "@/types/generated/ErrorKind";
import type { Model } from "@/types/generated/Model";
import type { ProviderInfo } from "@/types/generated/ProviderInfo";
import type { TestResult } from "@/types/generated/TestResult";
import { hostOf, providerKind } from "./connection-model";

/** The one place the app words a provider's failure: a failed Connect or Test here, and
 *  (through `errorKindCopy`) a run or glossary step that a provider stopped. The provider's
 *  raw text goes behind "Details" (`problemDetails`), never in the sentence. */

/** The form fields a test failure can point at. */
export type FieldKey = "key" | "model" | "address";

/** Plain-language copy for a failed test, and which field it belongs under
 *  (`null` = a card-level line, when the form has no such field). */
export interface ProblemCopy {
  field: FieldKey | null;
  text: string;
}

export function problemCopy(
  result: TestResult,
  info: ProviderInfo,
  conn: Pick<Model, "base_url">,
  /** The model is already saved (the card's Test): no "Save anyway" to offer. */
  { saved = false }: { saved?: boolean } = {},
): ProblemCopy {
  const kind = providerKind(info);
  const custom = kind === "custom";
  const who = custom ? "The server" : info.label;
  const host = hostOf(conn.base_url);
  const hasKeyField = kind !== "local";
  const hasAddressField = kind !== "cloud";
  const orSave = saved || !result.can_save_anyway ? "" : ", or Save anyway";

  switch (result.problem) {
    case "key": {
      const hint = info.key_hint ? ` (${info.key_hint})` : "";
      return {
        field: hasKeyField ? "key" : null,
        text: `${who} rejected this key. Check you copied all of it${hint}.`,
      };
    }
    case "model":
      return {
        field: "model",
        text: `${who} doesn't recognise this model. Pick one from the list, or check the spelling.`,
      };
    case "address":
      return {
        field: hasAddressField ? "address" : null,
        text:
          kind === "local"
            ? `Can't reach ${info.label} at ${host} — is it running?`
            : `Couldn't reach ${host}. Check your internet connection or proxy.`,
      };
    case "billing":
      return {
        field: null,
        text: custom
          ? "The server says this account is out of credit. Add credit to the account, then Try again."
          : `${who} says this account is out of credit. Add credit in your ${who} account, then Try again.`,
      };
    case "rate_limit":
      return {
        field: null,
        text: `${who} is rate-limiting this key. Wait a minute and Try again${orSave}.`,
      };
    case "server":
      return {
        field: null,
        text: `${custom ? "The server" : `${who}’s servers`} had a problem. Try again shortly${orSave}.`,
      };
    default:
      // The provider's own words are all there is; keep them out of the sentence.
      return {
        field: null,
        text: result.detail
          ? `${who} sent back an error. Details has what it said.`
          : result.message.trim() || "Something went wrong.",
      };
  }
}

/** The provider's raw words for "Details", when they add something to the sentence shown. */
export function problemDetails(result: TestResult, shown: string): string | null {
  const raw = result.detail?.trim();
  return raw && raw !== shown.trim() ? raw : null;
}

/** A failure that never reached the test (the command itself rejected). */
export function commandFailure(conn: Pick<Model, "model">, e: unknown): TestResult {
  return {
    ok: false,
    model: conn.model,
    detected_driver: null,
    message: String(e),
    problem: "other",
    can_save_anyway: false,
    detail: null,
  };
}

/**
 * The same words for a step a provider stopped mid-run (a run's `ErrorKind`), so a run's
 * banner and the Models page say one thing. `who` is the provider's name ("Anthropic").
 * Null for `other`: show the run's own message.
 */
export function errorKindCopy(kind: ErrorKind, who: string): string | null {
  switch (kind) {
    case "auth":
      return `${who} rejected the API key. Check it on the Models page.`;
    case "billing":
      return `${who} says this account is out of credit. Add credit in your ${who} account, then Try again.`;
    case "rate_limit":
      return `${who} is rate-limiting this key. Wait a minute and Try again.`;
    case "model":
      return `${who} doesn't recognise this model. Pick another on the Models page.`;
    case "network":
      return `Couldn't reach ${who}. Check your internet connection, then Try again.`;
    case "server":
      return `${who}’s servers had a problem. Try again shortly.`;
    default:
      return null;
  }
}
