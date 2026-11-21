import type { ProjectView } from "@/types/generated/ProjectView";
import { Checkbox } from "@/components/ui/checkbox";
import { HelpText } from "@/components/help-text";
import { cn, plural } from "@/lib/utils";

/** Whether the folder has anything to choose or explain: more than one style,
 *  or lines that are always kept as they are. */
export function hasLineChoices(view: ProjectView): boolean {
  return view.styles.length > 1 || view.always_kept.songs + view.always_kept.no_text > 0;
}

/** The ticked styles' names among the folder's, for the summary line. */
export function keptStyleNames(view: ProjectView): string[] {
  const kept = new Set(view.prefs.kept_styles);
  return view.styles.filter((s) => kept.has(s.name)).map((s) => s.name);
}

/** "Signs kept as they are" / "3 styles kept as they are", or null. */
export function keptSummary(view: ProjectView): string | null {
  const names = keptStyleNames(view);
  if (names.length === 0) return null;
  return names.length === 1 ? `${names[0]} kept as they are` : `${names.length} styles kept as they are`;
}

/** "56,100 song effect lines and 9,906 with no words" — what no run sends. */
function alwaysKeptLine(view: ProjectView): string | null {
  const { songs, no_text } = view.always_kept;
  const parts: string[] = [];
  if (songs > 0) parts.push(`${plural(songs, "song effect line")} (karaoke)`);
  if (no_text > 0) parts.push(`${no_text.toLocaleString()} with no words or a drawing`);
  if (parts.length === 0) return null;
  return `Always kept as they are: ${parts.join(" and ")}.`;
}

/**
 * The folder's styles, each with a tick: an unticked style's lines are written
 * as they are and never sent. Counts are lines to translate across the folder,
 * with a sign repeated frame by frame counted once.
 */
export function LinesToTranslate({
  view,
  disabled,
  onChange,
}: {
  view: ProjectView;
  disabled: boolean;
  onChange: (kept: string[]) => void;
}) {
  const kept = new Set(view.prefs.kept_styles);
  const ticked = view.styles.filter((s) => !kept.has(s.name));
  const always = alwaysKeptLine(view);

  const toggle = (name: string, on: boolean) => {
    const next = new Set(view.prefs.kept_styles);
    if (on) next.delete(name);
    else next.add(name);
    onChange([...next].sort());
  };

  return (
    <div className="mb-3.5">
      <p className="mb-1 text-[11.5px] font-semibold" id="ep-lines-label">
        Lines to translate
      </p>
      {view.styles.length > 1 ? (
        <ul
          aria-labelledby="ep-lines-label"
          className="max-h-56 overflow-auto rounded-xl border border-border py-1"
        >
          {view.styles.map((s) => {
            const on = !kept.has(s.name);
            // At least one style stays ticked: a run with nothing to send can't start.
            const last = on && ticked.length === 1;
            return (
              <li key={s.name}>
                <label
                  className={cn(
                    "flex items-center gap-2.5 px-3 py-1.5 text-[12.5px]",
                    disabled || last ? "cursor-default" : "cursor-pointer hover:bg-[color:var(--color-bg-hover)]",
                  )}
                  title={last ? "At least one style is translated." : undefined}
                >
                  <Checkbox
                    checked={on}
                    disabled={disabled || last}
                    onCheckedChange={(v) => toggle(s.name, v === true)}
                  />
                  <span
                    className={cn(
                      "w-32 shrink-0 truncate font-medium",
                      on ? "text-foreground" : "text-muted-foreground",
                    )}
                    title={s.name}
                  >
                    {s.name}
                  </span>
                  <span className="min-w-0 flex-1 truncate text-muted-foreground" title={s.sample}>
                    {s.sample}
                  </span>
                  <span
                    className={cn(
                      "shrink-0 text-right text-[11.5px] tabular-nums",
                      on ? "text-foreground/85" : "text-muted-foreground line-through",
                    )}
                    title={
                      s.all_lines > s.lines
                        ? `${plural(s.lines, "line")} to translate out of ${s.all_lines.toLocaleString()} in this style: a sign repeated frame by frame counts once, and song effects and lines with no words don’t count.`
                        : undefined
                    }
                  >
                    {s.lines.toLocaleString()}
                    {s.all_lines > s.lines ? (
                      <span className="text-muted-foreground"> of {s.all_lines.toLocaleString()}</span>
                    ) : null}
                  </span>
                </label>
              </li>
            );
          })}
        </ul>
      ) : null}
      <HelpText>
        {view.styles.length > 1 ? "Untick a style to keep its lines as they are. " : null}A sign
        shown frame by frame is translated once.{always ? ` ${always}` : null}
      </HelpText>
    </div>
  );
}
