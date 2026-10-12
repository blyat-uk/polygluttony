import { CheckCircle } from "@phosphor-icons/react";
import type { SourceFile } from "@/types/generated/SourceFile";
import { Checkbox } from "@/components/ui/checkbox";
import { plural } from "@/lib/utils";

/**
 * Source-file list with per-file selection. `selected` is the explicit list of
 * selected file names (empty = none selected); a freshly opened folder starts
 * with all files selected. The header checkbox toggles all ↔ none.
 */
export function FileList({
  files,
  selected,
  onChange,
}: {
  files: SourceFile[];
  selected: string[];
  onChange: (sel: string[]) => void;
}) {
  const allSelected = files.length > 0 && files.every((f) => selected.includes(f.name));
  const someSelected = selected.length > 0 && !allSelected;

  const toggle = (name: string) => {
    onChange(
      selected.includes(name) ? selected.filter((n) => n !== name) : [...selected, name],
    );
  };
  const toggleAll = () => onChange(allSelected ? [] : files.map((f) => f.name));

  return (
    <div className="mt-4 rounded-md border border-border">
      <div className="flex items-center gap-2 border-b border-border px-3 py-2 text-[11px] text-muted-foreground">
        <Checkbox
          checked={allSelected ? true : someSelected ? "indeterminate" : false}
          onCheckedChange={toggleAll}
        />
        <span>
          {plural(files.length, "file")} · {selected.length} selected
        </span>
      </div>
      <ul className="max-h-64 overflow-auto" aria-label="Source files">
        {files.map((f) => (
          <li key={f.path} className="flex items-center gap-2 px-3 py-1.5 text-[12px]">
            <Checkbox
              checked={selected.includes(f.name)}
              onCheckedChange={() => toggle(f.name)}
            />
            <span className="flex-1 truncate">{f.name}</span>
            <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground">
              {plural(f.dialogue_count, "line")}
            </span>
            {f.has_translation ? (
              <span className="inline-flex shrink-0 items-center gap-0.5 text-[10px] text-[color:var(--color-success)]">
                <CheckCircle weight="fill" className="size-3" />
                translated
              </span>
            ) : null}
          </li>
        ))}
      </ul>
    </div>
  );
}
