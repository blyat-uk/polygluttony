import type { Connection } from "@/types/generated/Connection";
import type { ProviderInfo } from "@/types/generated/ProviderInfo";
import type { TestResult } from "@/types/generated/TestResult";
import { hostOf, providerKind } from "./connection-model";

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
  conn: Connection,
): ProblemCopy {
  const kind = providerKind(info);
  const who = kind === "custom" ? "The server" : info.label;
  const host = hostOf(conn.base_url);
  const hasKeyField = kind !== "local";
  const hasAddressField = kind !== "cloud";

  switch (result.problem) {
    case "key": {
      const hint = info.key_hint ? ` (${info.key_hint})` : "";
      return {
        field: hasKeyField ? "key" : null,
        text: `${who} rejected this key. Check you copied all of it${hint}.`,
      };
    }
    case "model":
      return { field: "model", text: `${who} doesn't recognise this model.` };
    case "address":
      return {
        field: hasAddressField ? "address" : null,
        text:
          kind === "local"
            ? `Can't reach ${info.label} at ${host} — is it running?`
            : `Couldn't reach ${host}. Check your internet connection or proxy.`,
      };
    default:
      return { field: null, text: result.message.trim() || "Something went wrong." };
  }
}

/** A failure that never reached the test (the command itself rejected). */
export function commandFailure(conn: Connection, e: unknown): TestResult {
  return {
    ok: false,
    model: conn.model,
    detected_driver: null,
    message: String(e),
    problem: "other",
    can_save_anyway: false,
  };
}
