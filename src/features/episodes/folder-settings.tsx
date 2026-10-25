import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { CaretDown, CaretUp } from "@phosphor-icons/react";
import type { FolderPrefs } from "@/types/generated/FolderPrefs";
import type { Language } from "@/types/generated/Language";
import type { ProjectView } from "@/types/generated/ProjectView";
import type { Tone } from "@/types/generated/Tone";
import type { WorldType } from "@/types/generated/WorldType";
import { SetupField } from "@/components/setup-field";
import { HelpText } from "@/components/help-text";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { TONES, WORLDS, settingsSummary, titleCase, toneAfterWorldChange } from "./folder-labels";

/** The app's own select: warm dark, pill-shaped, like its other inputs. */
function PillSelect({
  id,
  value,
  disabled,
  options,
  onChange,
}: {
  id: string;
  value: string;
  disabled: boolean;
  options: { value: string; label: string }[];
  onChange: (v: string) => void;
}) {
  return (
    <Select value={value} disabled={disabled} onValueChange={onChange}>
      <SelectTrigger id={id} className="w-full">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {options.map((o) => (
          <SelectItem key={o.value} value={o.value}>
            {o.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

/**
 * The folder's settings as one summary line, with Edit ▾ opening the language,
 * world type and tone controls in place. Starts open on a folder's first visit,
 * when the detected settings are worth checking. Tone lives only here.
 */
export function FolderSettings({
  view,
  languages,
  glossaryBroken = false,
  locked,
  onChange,
}: {
  view: ProjectView;
  languages: Language[] | undefined;
  /** glossary.json is there but can't be read. */
  glossaryBroken?: boolean;
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
        <span className="text-muted-foreground">
          {settingsSummary(view, languages, glossaryBroken)}
          {glossaryBroken ? (
            <>
              {" · "}
              <span className="text-[color:var(--color-alert)]">Glossary has a mistake</span>
              {" · "}
              <Link to="/glossary" className="text-primary underline-offset-2 hover:underline">
                Fix it
              </Link>
            </>
          ) : null}
        </span>
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
                    Which terms matter most, e.g. Xianxia favors cultivation terms like 金丹 →
                    “Golden Core”.
                  </HelpText>
                }
              >
                <PillSelect
                  id="ep-world"
                  value={world}
                  disabled={locked}
                  options={WORLDS.map((w) => ({
                    value: w,
                    label: `${titleCase(w)}${!prefs.world_override && w === view.detected_world ? " (detected)" : ""}`,
                  }))}
                  onChange={(v) => {
                    const next = v as WorldType;
                    onChange({
                      ...prefs,
                      world_override: next,
                      tone: toneAfterWorldChange(prefs.tone, world, next),
                    });
                  }}
                />
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
              <PillSelect
                id="ep-tone"
                value={prefs.tone}
                disabled={locked}
                options={TONES.map((t) => ({ value: t, label: titleCase(t) }))}
                onChange={(v) => onChange({ ...prefs, tone: v as Tone })}
              />
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
    <PillSelect
      id={id}
      value={value}
      disabled={disabled || !languages}
      options={languages ? languages.map((l) => ({ value: l.code, label: l.name })) : [{ value, label: value }]}
      onChange={onChange}
    />
  );
}
