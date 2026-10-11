import { useId, useState, type ReactNode } from "react";
import { CaretRight } from "@phosphor-icons/react";
import type { Model } from "@/types/generated/Model";
import type { ModelCapabilities } from "@/types/generated/ModelCapabilities";
import type { ProviderInfo } from "@/types/generated/ProviderInfo";
import type { Step } from "@/types/generated/Step";
import type { StepSettings } from "@/types/generated/StepSettings";
import type { ThinkingLevel } from "@/types/generated/ThinkingLevel";
import { cn } from "@/lib/utils";
import { isUsable, normaliseThinking, type Tuning } from "./connection-model";
import { STEP_INFO, modelColor } from "./steps";
import { ThinkingControl, thinkingHelp } from "./thinking-control";
import { TuningSection } from "./tuning-section";

const THINKING_TAG: Record<ThinkingLevel, string> = {
  off: "Thinking off",
  low: "Thinking low",
  medium: "Thinking medium",
  high: "Thinking high",
};

/**
 * One step on the pipeline: a dot on the line in its model's colour (hollow when off or
 * missing its key), name + purpose, a Thinking tag when the model has Thinking, and the model
 * menu. Clicking anywhere but the menu opens Thinking + Tuning in place.
 */
export function StepRow({
  step,
  settings,
  model,
  info,
  caps,
  menu,
  onThinking,
  onTuning,
  onAddKey,
}: {
  step: Step;
  /** null = off (web search only). */
  settings: StepSettings | null;
  /** The step's library model; undefined when off. */
  model: Model | undefined;
  /** The model's provider. */
  info: ProviderInfo | undefined;
  /** The model's capabilities; undefined while they load. */
  caps: ModelCapabilities | undefined;
  /** The model menu (`StepModelMenu`). */
  menu: ReactNode;
  onThinking: (level: ThinkingLevel | null) => void;
  onTuning: (patch: Partial<Tuning>) => Promise<unknown>;
  /** Open the model's edit form to add its key; focus returns to `trigger` after. */
  onAddKey: (trigger: HTMLElement) => void;
}) {
  const uid = useId();
  const [open, setOpen] = useState(false);
  const { name, purpose, tuning } = STEP_INFO[step];

  const on = !!settings && !!model;
  const needsKey = !!model && !isUsable(model);
  const levels = caps?.thinking_levels ?? [];
  // What the backend will settle on: a step that hasn't picked a level gets the model's default.
  const thinking = settings && caps ? normaliseThinking(settings.thinking, caps) : null;
  const expanded = open && on;
  const ids = { purpose: `${uid}-purpose`, panel: `${uid}-panel`, thinking: `${uid}-thinking` };

  return (
    <li>
      <div
        className={cn(
          "relative flex min-h-12 flex-wrap items-center gap-x-2.5 gap-y-1.5 rounded-[11px] px-2.5 py-2 transition-colors",
          on && "hover:bg-[color:var(--color-raised)]",
          expanded && "bg-[color:var(--color-raised)]",
        )}
      >
        {/* The step's dot on the pipeline line; the ring masks the line behind it. */}
        <span
          aria-hidden
          className={cn(
            "absolute top-1/2 left-[-23px] size-[9px] -translate-y-1/2 rounded-full [box-shadow:0_0_0_3px_var(--card)]",
            (!on || needsKey) && "border-[1.5px] bg-card",
          )}
          style={
            !on
              ? { borderColor: "var(--color-ink-dim)" }
              : needsKey
                ? { borderColor: modelColor(model) }
                : { backgroundColor: modelColor(model) }
          }
        />

        <div className="min-w-40 flex-1 basis-40">
          {on ? (
            // Its hit area stretches over the whole row; the menu and key link sit above it.
            <button
              type="button"
              aria-expanded={expanded}
              aria-controls={ids.panel}
              aria-describedby={ids.purpose}
              onClick={() => setOpen((o) => !o)}
              className="text-left text-[12.5px] font-medium text-foreground outline-none after:absolute after:inset-0 after:rounded-[11px] focus-visible:after:ring-[3px] focus-visible:after:ring-ring/50"
            >
              {name}
            </button>
          ) : (
            <span className="text-[12.5px] font-medium text-muted-foreground">{name}</span>
          )}
          <p id={ids.purpose} className="text-[11px] leading-4 text-[color:var(--color-ink-dim)]">
            {purpose}
          </p>
          {needsKey ? (
            <p className="relative z-10 mt-0.5 w-fit text-[11px] leading-4 text-[color:var(--color-amber)]">
              Needs an API key ·{" "}
              <button
                type="button"
                onClick={(e) => onAddKey(e.currentTarget)}
                className="rounded-sm font-medium underline underline-offset-2 outline-none hover:text-[color:var(--color-gold-hi)] focus-visible:ring-[3px] focus-visible:ring-ring/50"
              >
                Add it
              </button>
            </p>
          ) : null}
        </div>

        <div className="ml-auto flex shrink-0 items-center gap-2">
          {on && levels.length > 0 ? (
            <span className="rounded-[10px] border border-border px-2 py-0.5 text-[10.5px] whitespace-nowrap text-muted-foreground">
              {thinking ? THINKING_TAG[thinking] : "Thinking auto"}
            </span>
          ) : null}
          <div className="relative z-10 flex min-w-0">{menu}</div>
          <CaretRight
            aria-hidden
            weight="bold"
            className={cn(
              "size-3 shrink-0 text-[color:var(--color-ink-dim)] transition-transform",
              expanded && "rotate-90",
              !on && "invisible",
            )}
          />
        </div>
      </div>

      {expanded ? (
        <div
          id={ids.panel}
          className="mt-1 mb-1.5 ml-2.5 rounded-xl border border-border bg-[color:var(--color-obsidian)] px-3 py-2"
        >
          {levels.length > 0 ? (
            <div className="flex min-h-10 flex-wrap items-center gap-x-3 gap-y-1 py-1">
              <span id={ids.thinking} className="w-[68px] shrink-0 text-[12px] font-medium">
                Thinking
              </span>
              <ThinkingControl
                labelledBy={ids.thinking}
                levels={levels}
                value={thinking}
                clearable={caps?.default_thinking === null}
                onChange={onThinking}
              />
              <span className="min-w-0 flex-1 basis-40 text-[11.5px] leading-4 text-muted-foreground">
                {thinkingHelp(thinking)}
              </span>
            </div>
          ) : null}
          {info && settings ? (
            <TuningSection value={settings} defaults={info.tuning} fields={tuning} onSave={onTuning} />
          ) : null}
        </div>
      ) : null}
    </li>
  );
}
