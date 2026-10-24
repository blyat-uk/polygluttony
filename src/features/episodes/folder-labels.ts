import type { Language } from "@/types/generated/Language";
import type { ProjectView } from "@/types/generated/ProjectView";
import type { Tone } from "@/types/generated/Tone";
import type { WorldType } from "@/types/generated/WorldType";
import { plural } from "@/lib/utils";

/** Tones in product order (not alphabetical). */
export const TONES: Tone[] = ["standard", "xianxia", "wuxia", "comedic", "funny"];
export const WORLDS: WorldType[] = ["xianxia", "wuxia", "historical", "modern"];

/** "xianxia" → "Xianxia". World and tone values always show in Title Case. */
export const titleCase = (s: string) => (s ? s[0].toUpperCase() + s.slice(1) : s);

/** Tone implied by a world type. Keep in sync with `tone_for_world` in
 *  src-tauri/src/config/projects.rs. */
export const toneForWorld = (w: WorldType): Tone =>
  w === "xianxia" ? "xianxia" : w === "wuxia" ? "wuxia" : "standard";

/** The tone after a world change: it follows the world unless the user picked
 *  one by hand (the current tone isn't the old world's tone). */
export const toneAfterWorldChange = (tone: Tone, from: WorldType, to: WorldType): Tone =>
  tone === toneForWorld(from) ? toneForWorld(to) : tone;

export const languageName = (languages: Language[] | undefined, code: string) =>
  languages?.find((l) => l.code === code)?.name ?? code;

/** "Chinese → English · World: Xianxia · Tone: Xianxia · Glossary 232 terms".
 *  World only shows when the source language detects one; the glossary part
 *  only when the source language supports a glossary. */
export function settingsSummary(view: ProjectView, languages: Language[] | undefined): string {
  const { prefs } = view;
  const source = languages?.find((l) => l.code === prefs.source_lang);
  const parts = [
    `${languageName(languages, prefs.source_lang)} → ${languageName(languages, prefs.target_lang)}`,
  ];
  if (source?.supports_world_detection) {
    parts.push(`World: ${titleCase(prefs.world_override ?? view.detected_world)}`);
  }
  parts.push(`Tone: ${titleCase(prefs.tone)}`);
  if (view.supports_glossary) {
    parts.push(
      view.glossary_terms == null ? "No glossary yet" : `Glossary ${plural(view.glossary_terms, "term")}`,
    );
  }
  return parts.join(" · ");
}

/** "Veil of Shadow - 07.ass" → "Veil of Shadow - 07". */
export const episodeName = (file: string) => file.replace(/\.ass$/i, "");
