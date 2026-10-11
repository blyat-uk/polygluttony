import { invoke } from "@tauri-apps/api/core";
import { listen, type EventCallback, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppInfo } from "@/types/generated/AppInfo";
import type { BuildMode } from "@/types/generated/BuildMode";
import type { Connection } from "@/types/generated/Connection";
import type { ConnectionsConfig } from "@/types/generated/ConnectionsConfig";
import type { ModelCapabilities } from "@/types/generated/ModelCapabilities";
import type { Provider } from "@/types/generated/Provider";
import type { ProviderInfo } from "@/types/generated/ProviderInfo";
import type { TestResult } from "@/types/generated/TestResult";
import type { WebSearch } from "@/types/generated/WebSearch";
import type { FirstRunStatus } from "@/types/generated/FirstRunStatus";
import type { Language } from "@/types/generated/Language";
import type { ProjectView } from "@/types/generated/ProjectView";
import type { FolderPrefs } from "@/types/generated/FolderPrefs";
import type { RecentFolder } from "@/types/generated/RecentFolder";
import type { Tone } from "@/types/generated/Tone";
import type { GlossaryBackupInfo } from "@/types/generated/GlossaryBackupInfo";
import type { GlossaryDoc } from "@/types/generated/GlossaryDoc";
import type { NormalizeReview } from "@/types/generated/NormalizeReview";
import type { ReferenceStatus } from "@/types/generated/ReferenceStatus";
import type { ReferenceSummary } from "@/types/generated/ReferenceSummary";
import type { ReferenceTerminology } from "@/types/generated/ReferenceTerminology";
import type { WorldType } from "@/types/generated/WorldType";
import type { PromptId } from "@/types/generated/PromptId";
import type { PromptMeta } from "@/types/generated/PromptMeta";

/**
 * Typed wrappers around the Rust core's Tauri commands. The webview never talks
 * to LLMs or the filesystem directly — every backend capability is exposed here.
 */
export const ipc = {
  /** App/core metadata. Doubles as a startup health check for the IPC bridge. */
  appInfo: () => invoke<AppInfo>("app_info"),
  /** O21 — whether a usable translation connection exists (first-run gate). */
  firstRunStatus: () => invoke<FirstRunStatus>("first_run_status"),
  /** The Connections page's whole state: translation, web search, remembered keys. */
  getConnections: () => invoke<ConnectionsConfig>("get_connections"),
  /** Provider catalog (labels, key links, curated models, tuning defaults). */
  listProviders: () => invoke<ProviderInfo[]>("list_providers"),
  /** What a provider + model supports: thinking levels, web search. */
  modelCapabilities: (provider: Provider, model: string) =>
    invoke<ModelCapabilities>("model_capabilities", { provider, model }),
  /** Save the translation connection; returns the normalised page state. */
  saveTranslation: (connection: Connection) =>
    invoke<ConnectionsConfig>("save_translation", { connection }),
  /** Remove the translation connection (web search on "translation" turns off). */
  removeTranslation: () => invoke<ConnectionsConfig>("remove_translation"),
  /** Set who searches the web: off, the translation model, or a separate connection. */
  setWebSearch: (webSearch: WebSearch) =>
    invoke<ConnectionsConfig>("set_web_search", { webSearch }),
  /** O5 — send one tiny test request. Custom endpoints get their API format detected.
   *  `webSearch` also exercises the provider's web-search tool (the separate search model). */
  testConnection: (connection: Connection, webSearch?: boolean) =>
    invoke<TestResult>(
      "test_connection",
      webSearch === undefined ? { connection } : { connection, webSearch },
    ),
  /** Live model ids from the provider. Rejects when it can't be reached or the request
   *  fails; `[]` means it answered with no models. */
  listModels: (connection: Connection) => invoke<string[]>("list_models", { connection }),
  /** O6/O7/O8 — open a folder: discover files, counts, detect world/language, load prefs. */
  openFolder: (path: string) =>
    invoke<ProjectView>("open_folder", { path, now: Math.floor(Date.now() / 1000) }),
  /** O8 — the supported-language table for the selects. */
  listLanguages: () => invoke<Language[]>("list_languages"),
  /** Persist per-folder preferences (languages, world override, tone, selection). */
  saveFolderPrefs: (path: string, prefs: FolderPrefs) =>
    invoke<void>("save_folder_prefs", { path, prefs }),
  /** Persist the source/target pair as the global default (new folders + future sessions). */
  setDefaultLanguages: (source: string, target: string) =>
    invoke<void>("set_default_languages", { source, target }),
  /** Recent folders (MRU; missing folders pruned server-side). */
  listRecents: () => invoke<RecentFolder[]>("list_recents"),
  removeRecent: (path: string) => invoke<void>("remove_recent", { path }),
  clearRecents: () => invoke<void>("clear_recents"),
  /** O16 — start a translation run. */
  startTranslation: (args: {
    folder: string
    files: string[]
    tone: Tone
    sourceLang: string
    targetLang: string
  }) => invoke<void>("start_translation", args),
  /** O17 — cancel the active run. */
  cancelTranslation: () => invoke<void>("cancel_translation"),
  /** O9 — load glossary.json (null when none exists). */
  loadGlossary: (folder: string) => invoke<GlossaryDoc | null>("load_glossary", { folder }),
  /** O14 — persist the whole glossary doc (atomic write). */
  saveGlossary: (folder: string, doc: GlossaryDoc) =>
    invoke<void>("save_glossary", { folder, doc }),
  /** O10 — start a glossary build run (events on glossary://event). */
  startGlossaryBuild: (args: {
    folder: string
    mode: BuildMode
    files: string[]
    worldType: WorldType
    sourceLang: string
    targetLang: string
    normalize: boolean
    personalize: boolean
    personalizeContext: string
  }) => invoke<void>("start_glossary_build", args),
  /** Cancel the active glossary op (build / normalize / import). */
  cancelGlossaryBuild: () => invoke<void>("cancel_glossary_build"),
  /** O12 — run normalization; returns a review, NOT saved. */
  normalizeGlossary: (folder: string) =>
    invoke<NormalizeReview>("normalize_glossary", { folder }),
  /** O11 — extract reference terms from picked .ass files (cached for builds). */
  importReferenceFiles: (folder: string, paths: string[]) =>
    invoke<ReferenceSummary>("import_reference_files", { folder, paths }),
  referenceStatus: (folder: string) =>
    invoke<ReferenceStatus>("reference_status", { folder }),
  clearReference: (folder: string) => invoke<void>("clear_reference", { folder }),
  /** Cached reference terminology for the review screen (null = no cache). */
  loadReference: (folder: string) =>
    invoke<ReferenceTerminology | null>("load_reference", { folder }),
  /** Persist review-screen pruning. */
  saveReference: (folder: string, terms: ReferenceTerminology) =>
    invoke<void>("save_reference", { folder, terms }),
  /** Install a picked glossary.json into this folder; returns the term count. */
  importGlossary: (folder: string, src: string) =>
    invoke<number>("import_glossary", { folder, src }),
  /** What glossary.prev.json holds right now (null = no backup yet) — the
   *  "you are about to replace this backup" disclosure. */
  glossaryBackupStatus: (folder: string) =>
    invoke<GlossaryBackupInfo | null>("glossary_backup_status", { folder }),
  exportGlossary: (folder: string, dest: string) =>
    invoke<void>("export_glossary", { folder, dest }),
  /** O15 — open glossary.json in the OS default editor. */
  openGlossaryEditor: (folder: string) => invoke<void>("open_glossary_editor", { folder }),
  watchGlossary: (folder: string) => invoke<void>("watch_glossary", { folder }),
  unwatchGlossary: () => invoke<void>("unwatch_glossary"),
  /** Label of the model that does web search (e.g. "Claude Sonnet 5.5"), or null. */
  webSearchStatus: () => invoke<string | null>("web_search_status"),
  /** Prompts — list all prompt templates with modified flags + placeholder specs. */
  listPrompts: () => invoke<PromptMeta[]>("list_prompts"),
  /** Prompts — current effective text (override or embedded default). */
  getPrompt: (id: PromptId) => invoke<string>("get_prompt", { id }),
  /** Prompts — validate + persist a custom prompt override. */
  savePrompt: (id: PromptId, text: string) => invoke<void>("save_prompt", { id, text }),
  /** Prompts — delete the override; returns the embedded default text. */
  resetPrompt: (id: PromptId) => invoke<string>("reset_prompt", { id }),
  /** Prompts — read a picked .txt file for "Load from file…" (≤1 MB, UTF-8). */
  readPromptFile: (path: string) => invoke<string>("read_prompt_file", { path }),
};

/** Subscribe to a backend-emitted event (progress, logs, …). Returns an unlisten fn. */
export function onBackendEvent<T>(
  name: string,
  handler: EventCallback<T>,
): Promise<UnlistenFn> {
  return listen<T>(name, handler);
}
