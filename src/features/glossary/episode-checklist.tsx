import type { ReactNode } from "react";
import type { SourceFile } from "@/types/generated/SourceFile";
import { Checkbox } from "@/components/ui/checkbox";
import { cn, plural } from "@/lib/utils";
import { episodes } from "./episodes";

/**
 * The episodes a glossary scan reads. Its own selection, separate from the
 * Episodes page: Build glossary, Add new episodes and Rebuild from scratch
 * each start from their own defaults.
 */
export function EpisodeChecklist({
  files,
  selected,
  onChange,
  tag,
  disabled = false,
  className,
}: {
  files: SourceFile[];
  selected: string[];
  onChange: (names: string[]) => void;
  /** A muted note at the end of a row, e.g. "in glossary". */
  tag?: (file: SourceFile) => ReactNode;
  disabled?: boolean;
  className?: string;
}) {
  const picked = new Set(selected);
  const all = files.length > 0 && files.every((f) => picked.has(f.name));
  const some = !all && files.some((f) => picked.has(f.name));
  const count = files.filter((f) => picked.has(f.name)).length;

  const toggle = (name: string, on: boolean) =>
    onChange(on ? [...selected.filter((n) => n !== name), name] : selected.filter((n) => n !== name));

  return (
    <div className={cn("rounded-xl border border-border bg-[color:var(--card)]", className)}>
      <label className="flex cursor-pointer items-center gap-2.5 border-b border-border px-3 py-2 text-[11.5px] text-muted-foreground">
        <Checkbox
          checked={all ? true : some ? "indeterminate" : false}
          disabled={disabled}
          onCheckedChange={() => onChange(all ? [] : files.map((f) => f.name))}
        />
        <span className="tabular-nums">
          {episodes(files.length)} · {count} ticked
        </span>
      </label>
      <ul className="max-h-60 overflow-auto py-1" aria-label="Episodes">
        {files.map((f) => (
          <li key={f.path}>
            <label className="flex cursor-pointer items-center gap-2.5 px-3 py-1.5 text-[12.5px] hover:bg-[color:var(--color-bg-hover)]">
              <Checkbox
                checked={picked.has(f.name)}
                disabled={disabled}
                onCheckedChange={(v) => toggle(f.name, v === true)}
              />
              <span className="min-w-0 flex-1 truncate text-foreground">{f.name}</span>
              {tag ? <span className="shrink-0 text-[10.5px] text-muted-foreground">{tag(f)}</span> : null}
              <span className="w-16 shrink-0 text-right text-[10.5px] tabular-nums text-muted-foreground">
                {plural(f.dialogue_count, "line")}
              </span>
            </label>
          </li>
        ))}
      </ul>
    </div>
  );
}
