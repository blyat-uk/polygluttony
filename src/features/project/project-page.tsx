import { useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { BookOpen, Play } from "@phosphor-icons/react";
import type { Language } from "@/types/generated/Language";
import type { FolderPrefs } from "@/types/generated/FolderPrefs";
import type { Tone } from "@/types/generated/Tone";
import type { WorldType } from "@/types/generated/WorldType";
import { ipc } from "@/lib/ipc";
import { plural } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";
import { useProject, syncProjectStore, projectKey } from "./use-project";
import { FileList } from "./file-list";
import { PageHeader } from "@/components/page-header";
import { SetupField } from "@/components/setup-field";
import { HelpText } from "@/components/help-text";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/empty-state";

const TONES: Tone[] = ["standard", "xianxia", "wuxia", "comedic", "funny"];
const WORLDS: WorldType[] = ["xianxia", "wuxia", "historical", "modern"];

// Tone implied by a world type — keep in sync with `tone_for_world` in
// src-tauri/src/config/projects.rs.
const toneForWorld = (w: WorldType): Tone =>
  w === "xianxia" ? "xianxia" : w === "wuxia" ? "wuxia" : "standard";
const SELECT_CLS =
  "h-9 w-full rounded-md border border-input bg-[color:var(--card)] px-2 text-sm";

export function ProjectPage() {
  const workdir = useAppStore((s) => s.workdir);
  const navigate = useNavigate();
  const qc = useQueryClient();
  const { data: view } = useProject(workdir ?? "");
  const { data: languages } = useQuery({
    queryKey: ["languages"],
    queryFn: ipc.listLanguages,
    staleTime: Infinity,
  });
  const [prefs, setPrefs] = useState<FolderPrefs | null>(null);

  useEffect(() => {
    if (view) setPrefs(view.prefs);
  }, [view]);

  if (!workdir) return <EmptyState title="Project" description="Open a folder first." />;
  if (!view || !prefs || !languages) return null;

  const persist = (next: FolderPrefs) => {
    setPrefs(next);
    // Save immediately (per-folder) so a tab switch / revisit sees the change.
    void ipc.saveFolderPrefs(view.folder, next);
    // Keep the cached ProjectView + the shell (status bar + rail gating) in sync.
    qc.setQueryData(projectKey(view.folder), { ...view, prefs: next });
    syncProjectStore({ ...view, prefs: next });
    // The source/target pair doubles as the global default for new folders + sessions.
    const langsChanged =
      next.source_lang !== prefs.source_lang || next.target_lang !== prefs.target_lang;
    if (langsChanged && next.source_lang !== next.target_lang) {
      void ipc.setDefaultLanguages(next.source_lang, next.target_lang);
    }
  };

  const sameLang = prefs.source_lang === prefs.target_lang;
  const sourceLang = languages.find((l) => l.code === prefs.source_lang);
  const showWorld = !!sourceLang?.supports_world_detection;
  const showGlossary = !!sourceLang?.supports_glossary;
  const effectiveWorld: WorldType = prefs.world_override ?? view.detected_world;
  const folderName = view.folder.split(/[/\\]/).pop() || view.folder;

  return (
    <div className="flex h-full flex-col">
      <PageHeader
        title={folderName}
        description="Set up this folder, then build a glossary or jump to translating."
        actions={
          <button
            type="button"
            onClick={() => navigate({ to: "/" })}
            className="text-[11px] text-primary hover:underline"
          >
            change
          </button>
        }
      />
      <div className="flex-1 overflow-auto p-5">
        <p className="mb-3 text-[12.5px] text-muted-foreground tabular-nums">
          {plural(view.files.length, "subtitle file")} · {plural(view.total_dialogue_lines, "line")}
        </p>

        <div className="grid grid-cols-2 gap-x-4">
          <SetupField
            label="Source language"
            help={<HelpText>Detected from the files; change if it&apos;s wrong.</HelpText>}
          >
            <LangSelect
              languages={languages}
              value={prefs.source_lang}
              onChange={(v) => persist({ ...prefs, source_lang: v })}
            />
          </SetupField>
          <SetupField
            label="Target language"
            help={
              sameLang ? (
                <p className="mt-1 text-[11px] text-[color:var(--color-danger)]">
                  Source and target must differ.
                </p>
              ) : undefined
            }
          >
            <LangSelect
              languages={languages}
              value={prefs.target_lang}
              onChange={(v) => persist({ ...prefs, target_lang: v })}
            />
          </SetupField>

          {showWorld ? (
            <SetupField
              label="World type"
              help={
                <HelpText>
                  Sets which terms to prioritize — e.g. xianxia favors cultivation terms like 金丹 →
                  “Golden Core”. Auto-detected; change it if the guess is off.
                </HelpText>
              }
            >
              <select
                className={SELECT_CLS}
                value={effectiveWorld}
                onChange={(e) => {
                  const nextWorld = e.target.value as WorldType;
                  // Tone follows the world type unless the user hand-picked a
                  // different tone (i.e. tone still matches the previous
                  // world's implied tone).
                  const tone =
                    prefs.tone === toneForWorld(effectiveWorld)
                      ? toneForWorld(nextWorld)
                      : prefs.tone;
                  persist({ ...prefs, world_override: nextWorld, tone });
                }}
              >
                {[...WORLDS].sort().map((w) => (
                  <option key={w} value={w}>
                    {w}
                    {!prefs.world_override && w === view.detected_world ? " (auto-detected)" : ""}
                  </option>
                ))}
              </select>
            </SetupField>
          ) : null}

          <SetupField
            label="Tone"
            help={
              <HelpText>
                How the dialogue reads — e.g. “standard” for everyday speech, “wuxia” for sweeping
                martial-arts lines.
              </HelpText>
            }
          >
            <select
              className={SELECT_CLS}
              value={prefs.tone}
              onChange={(e) => persist({ ...prefs, tone: e.target.value as Tone })}
            >
              {[...TONES].sort().map((t) => (
                <option key={t} value={t}>
                  {t}
                </option>
              ))}
            </select>
          </SetupField>
        </div>

        <FileList
          files={view.files}
          selected={prefs.selected_files}
          onChange={(sel) => persist({ ...prefs, selected_files: sel })}
        />
      </div>

      <div className="flex items-center gap-3 border-t border-border bg-[color:var(--popover)] px-5 py-3">
        <span className="text-[11px] text-muted-foreground">Next:</span>
        {showGlossary ? (
          <Button variant="secondary" onClick={() => navigate({ to: "/glossary" })}>
            <BookOpen className="size-4" /> Build a glossary
          </Button>
        ) : null}
        <Button disabled={sameLang} onClick={() => navigate({ to: "/translate" })}>
          <Play className="size-4" /> Translate now
        </Button>
      </div>
    </div>
  );
}

function LangSelect({
  languages,
  value,
  onChange,
}: {
  languages: Language[];
  value: string;
  onChange: (v: string) => void;
}) {
  return (
    <select className={SELECT_CLS} value={value} onChange={(e) => onChange(e.target.value)}>
      {languages.map((l) => (
        <option key={l.code} value={l.code}>
          {l.name}
        </option>
      ))}
    </select>
  );
}
