import { useState } from "react";
import { CaretDown, CaretUp } from "@phosphor-icons/react";
import type { FolderPrefs } from "@/types/generated/FolderPrefs";
import type { Language } from "@/types/generated/Language";
import type { ProjectView } from "@/types/generated/ProjectView";
import type { Tone } from "@/types/generated/Tone";
import type { WorldType } from "@/types/generated/WorldType";
import { SetupField } from "@/components/setup-field";
import { HelpText } from "@/components/help-text";
import { Button } from "@/components/ui/button";
import { TONES, WORLDS, settingsSummary, titleCase, toneAfterWorldChange } from "./folder-labels";

const SELECT_CLS =
  "h-9 w-full rounded-md border border-input bg-[color:var(--card)] px-2 text-sm outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:opacity-50";

/**
 * The folder's settings as one summary line, with Edit ▾ opening the language,
 * world type and tone controls in place. Starts open on a folder's first visit,
 * when the detected settings are worth checking. Tone lives only here.
 */
export function FolderSettings({
  view,
  languages,
  locked,
  onChange,
}: {
  view: ProjectView;
  languages: Language[] | undefined;
  /** A run is going in this folder; settings wait until it's done. */
  locked: boolean;
  onChange: (next: FolderPrefs) => void;
}) {
  const [open, setOpen] = useState(view.first_visit);
  const { prefs } = view;
  const sameLang = prefs.source_lang === prefs.target_lang;
  const source = languages?.find((l) => l.code === prefs.source_lang);
  const world: WorldType = prefs.world_override ?? view.detected_world;

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2 text-[12.5px]">
        <span className="text-muted-foreground">{settingsSummary(view, languages)}</span>
        <Button
          variant="ghost"
          size="xs"
          aria-expanded={open}
          onClick={() => setOpen((o) => !o)}
          className="text-foreground"
        >
          Edit
          {open ? <CaretUp /> : <CaretDown />}
        </Button>
      </div>

      {open ? (
        <div className="rounded-xl border border-border bg-[color:var(--card)] px-4 pt-3.5 pb-1">
          <div className="grid grid-cols-2 gap-x-4">
            <SetupField label="Source language" htmlFor="ep-source">
              <LangSelect
                id="ep-source"
                languages={languages}
                value={prefs.source_lang}
                disabled={locked}
                onChange={(v) => onChange({ ...prefs, source_lang: v })}
              />
            </SetupField>
            <SetupField label="Target language" htmlFor="ep-target">
              <LangSelect
                id="ep-target"
                languages={languages}
                value={prefs.target_lang}
                disabled={locked}
                onChange={(v) => onChange({ ...prefs, target_lang: v })}
              />
            </SetupField>
          </div>
          <div className="-mt-2 mb-3.5">
            {sameLang ? (
              <p className="text-[11px] text-[color:var(--color-danger)]">
                Source and target must differ.
              </p>
            ) : (
              <HelpText>Also used for new folders.</HelpText>
            )}
          </div>

          <div className="grid grid-cols-2 gap-x-4">
            {source?.supports_world_detection ? (
              <SetupField
                label="World type"
                htmlFor="ep-world"
                help={
                  <HelpText>
                    Which terms matter most, e.g. Xianxia favours cultivation terms like 金丹 →
                    “Golden Core”.
                  </HelpText>
                }
              >
                <select
                  id="ep-world"
                  className={SELECT_CLS}
                  value={world}
                  disabled={locked}
                  onChange={(e) => {
                    const next = e.target.value as WorldType;
                    onChange({
                      ...prefs,
                      world_override: next,
                      tone: toneAfterWorldChange(prefs.tone, world, next),
                    });
                  }}
                >
                  {WORLDS.map((w) => (
                    <option key={w} value={w}>
                      {titleCase(w)}
                      {!prefs.world_override && w === view.detected_world ? " (detected)" : ""}
                    </option>
                  ))}
                </select>
              </SetupField>
            ) : null}
            <SetupField
              label="Tone"
              htmlFor="ep-tone"
              help={
                <HelpText>
                  How the dialogue reads: Standard for everyday speech, Wuxia for sweeping
                  martial-arts lines.
                </HelpText>
              }
            >
              <select
                id="ep-tone"
                className={SELECT_CLS}
                value={prefs.tone}
                disabled={locked}
                onChange={(e) => onChange({ ...prefs, tone: e.target.value as Tone })}
              >
                {TONES.map((t) => (
                  <option key={t} value={t}>
                    {titleCase(t)}
                  </option>
                ))}
              </select>
            </SetupField>
          </div>
          {locked ? (
            <p className="mb-3 text-[11px] text-muted-foreground">
              You can change these when the run is done.
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function LangSelect({
  id,
  languages,
  value,
  disabled,
  onChange,
}: {
  id: string;
  languages: Language[] | undefined;
  value: string;
  disabled: boolean;
  onChange: (v: string) => void;
}) {
  return (
    <select
      id={id}
      className={SELECT_CLS}
      value={value}
      disabled={disabled || !languages}
      onChange={(e) => onChange(e.target.value)}
    >
      {(languages ?? []).map((l) => (
        <option key={l.code} value={l.code}>
          {l.name}
        </option>
      ))}
      {!languages ? <option value={value}>{value}</option> : null}
    </select>
  );
}
