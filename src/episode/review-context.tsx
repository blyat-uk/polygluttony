import { createContext, useContext } from "react";
import type { VerifyIssue } from "@/types/generated/VerifyIssue";

/** What a line row can do in review. Null in the context while the window
 *  can't review (no folder); `enabled` is false while it mustn't (a run). */
export type ReviewActions = {
  enabled: boolean;
  /** The line whose translation is open for editing. */
  editingId: number | null;
  /** True between asking for an action and the window seeing it happen. */
  busy: boolean;
  select: (id: number) => void;
  startEdit: (id: number) => void;
  cancelEdit: () => void;
  /** Resolves true once saved; the editor stays open on false. */
  saveEdit: (id: number, text: string) => Promise<boolean>;
  retranslate: (ids: number[]) => void;
  looksFine: (id: number, issues: VerifyIssue[]) => void;
  /** `term` from a name issue; without one, the text selected in the window. */
  fixInGlossary: (term: string | null) => void;
};

export const ReviewContext = createContext<ReviewActions | null>(null);

export function useReview(): ReviewActions | null {
  return useContext(ReviewContext);
}
