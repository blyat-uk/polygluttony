// Workspace UI state that must outlive a remount: the filters, the open side
// panel, and a Tidy up / Look up run with its proposed changes (a paid result
// must survive leaving the page while it runs).

import { create } from "zustand";
import type { TermRef } from "@/types/generated/TermRef";
import type { ShowFilter } from "./filters";
import type { Category } from "./model";
import type { ProposedChange } from "./proposed";

export type ImproveMode = "tidy" | "lookup";

export type ScopeChoice =
  | { kind: "all" }
  | { kind: "selected" }
  | { kind: "category"; category: Category }
  | { kind: "one"; ref: TermRef };

export interface ImproveState {
  mode: ImproveMode;
  scope: ScopeChoice;
  showName: string;
  stage: "setup" | "running" | "review";
  /** Ties a resolving run to the panel that started it. */
  runId: number;
  /** Terms the running request covers. */
  runCount: number;
  changes: ProposedChange[];
  ticked: string[];
}

export type Panel = { kind: "improve" } | { kind: "add-episodes"; preset: string[] | null } | null;

interface UiStore {
  folder: string | null;
  search: string;
  show: ShowFilter;
  category: Category | null;
  panel: Panel;
  improve: ImproveState | null;
  /** A scan is running from this page: show its new terms when it lands. */
  afterScan: boolean;
  /** Folder whose very first glossary build was started here. */
  firstBuild: string | null;
  setSearch: (search: string) => void;
  setShow: (show: ShowFilter) => void;
  setCategory: (category: Category | null) => void;
  openPanel: (panel: Panel) => void;
  closePanel: () => void;
  setImprove: (patch: Partial<ImproveState>) => void;
  /** Reset everything when the folder changes. */
  forFolder: (folder: string) => void;
}

export const useGlossaryUi = create<UiStore>((set) => ({
  folder: null,
  search: "",
  show: "all",
  category: null,
  panel: null,
  improve: null,
  afterScan: false,
  firstBuild: null,
  setSearch: (search) => set({ search }),
  setShow: (show) => set({ show }),
  setCategory: (category) => set({ category }),
  openPanel: (panel) => set({ panel }),
  closePanel: () => set({ panel: null, improve: null }),
  setImprove: (patch) => set((s) => (s.improve ? { improve: { ...s.improve, ...patch } } : {})),
  forFolder: (folder) =>
    set((s) =>
      s.folder === folder
        ? {}
        : {
            folder,
            search: "",
            show: "all",
            category: null,
            panel: null,
            improve: null,
            afterScan: false,
            firstBuild: null,
          },
    ),
}));

let runCounter = 0;
export const nextRunId = () => ++runCounter;
