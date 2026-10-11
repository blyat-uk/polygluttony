import { create } from "zustand";
import type { WorldType } from "@/types/generated/WorldType";
import type { Tone } from "@/types/generated/Tone";

interface ProjectState {
  workdir: string;
  sourceLang: string;
  targetLang: string;
  worldType: WorldType;
  tone: Tone;
  fileCount: number;
  dialogueLineCount: number;
  hasUntranslated: boolean;
  glossaryTerms: number | null;
}

interface AppState {
  workdir: string | null;
  sourceLang: string;
  targetLang: string;
  worldType: WorldType | null;
  tone: Tone;
  fileCount: number;
  dialogueLineCount: number;
  hasUntranslated: boolean;
  glossaryTerms: number | null;
  /** Display label of the translation model (e.g. "Claude Sonnet 5.5"), null when none. */
  translationLabel: string | null;
  /** A usable translation connection exists — gates Translate. */
  hasUsableConnection: boolean;
  setWorkdir: (dir: string | null) => void;
  setLanguages: (source: string, target: string) => void;
  setProject: (p: ProjectState) => void;
  clearProject: () => void;
  setTranslationStatus: (label: string | null, usable: boolean) => void;
  setGlossaryTerms: (n: number | null) => void;
}

export const useAppStore = create<AppState>((set) => ({
  workdir: null,
  sourceLang: "zh",
  targetLang: "en",
  worldType: null,
  tone: "standard",
  fileCount: 0,
  dialogueLineCount: 0,
  hasUntranslated: false,
  glossaryTerms: null,
  translationLabel: null,
  hasUsableConnection: false,
  setWorkdir: (workdir) => set({ workdir }),
  setLanguages: (sourceLang, targetLang) => set({ sourceLang, targetLang }),
  setProject: (p) => set({ ...p }),
  clearProject: () =>
    set({
      workdir: null,
      worldType: null,
      fileCount: 0,
      dialogueLineCount: 0,
      hasUntranslated: false,
      glossaryTerms: null,
    }),
  setTranslationStatus: (translationLabel, hasUsableConnection) =>
    set({ translationLabel, hasUsableConnection }),
  setGlossaryTerms: (glossaryTerms) => set({ glossaryTerms }),
}));
