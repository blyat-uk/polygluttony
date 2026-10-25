// Which terms Tidy up / Look up names online works on.

import type { GlossaryDoc } from "@/types/generated/GlossaryDoc";
import type { GlossaryScope } from "@/types/generated/GlossaryScope";
import type { TermRef } from "@/types/generated/TermRef";
import type { ScopeChoice } from "./ui-store";

export function toGlossaryScope(choice: ScopeChoice, selected: TermRef[]): GlossaryScope {
  switch (choice.kind) {
    case "all":
      return { kind: "all" };
    case "selected":
      return { kind: "terms", terms: selected };
    case "category":
      return { kind: "category", category: choice.category };
    case "one":
      return { kind: "terms", terms: [choice.ref] };
  }
}

export function scopeCount(choice: ScopeChoice, doc: GlossaryDoc, selected: number): number {
  switch (choice.kind) {
    case "all":
      return doc.count;
    case "selected":
      return selected;
    case "category":
      return Object.keys(doc.terms[choice.category] ?? {}).length;
    case "one":
      return doc.terms[choice.ref.category]?.[choice.ref.source] !== undefined ? 1 : 0;
  }
}
