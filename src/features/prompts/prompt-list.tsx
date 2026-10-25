import type { PromptGroup } from "@/types/generated/PromptGroup";
import type { PromptId } from "@/types/generated/PromptId";
import type { PromptMeta } from "@/types/generated/PromptMeta";
import { cn } from "@/lib/utils";

const GROUP_LABELS: Record<PromptGroup, string> = {
  translation: "Translation",
  tones: "Tones",
  glossary: "Glossary",
  verify: "Verify",
};
const GROUP_ORDER: PromptGroup[] = ["translation", "tones", "glossary", "verify"];

interface Props {
  prompts: PromptMeta[] | undefined;
  selected: PromptId | null;
  onSelect: (id: PromptId) => void;
}

export function PromptList({ prompts, selected, onSelect }: Props) {
  return (
    <div className="w-56 shrink-0 overflow-y-auto border-r border-border py-2">
      {GROUP_ORDER.map((group) => {
        const items = prompts?.filter((p) => p.group === group) ?? [];
        if (!items.length) return null;
        return (
          <div key={group} className="mb-2">
            <div className="px-4 py-1 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
              {GROUP_LABELS[group]}
            </div>
            {items.map((p) => (
              <button
                key={p.id}
                type="button"
                onClick={() => onSelect(p.id)}
                aria-current={selected === p.id ? "true" : undefined}
                className={cn(
                  "flex w-full items-center justify-between px-4 py-1.5 text-left text-[12.5px]",
                  selected === p.id
                    ? "bg-[color:var(--popover)] text-primary"
                    : "hover:bg-[color:var(--color-bg-hover)]",
                )}
              >
                <span className="truncate">{p.name}</span>
                {p.modified ? <EditedMark className="ml-2" /> : null}
              </button>
            ))}
          </div>
        );
      })}
    </div>
  );
}

/** A prompt you've changed from its default: gold dot and the word, so it reads twice. */
export function EditedMark({ className }: { className?: string }) {
  return (
    <span
      title="Changed from the default"
      className={cn(
        "inline-flex shrink-0 items-center gap-1 text-[10.5px] font-medium text-primary",
        className,
      )}
    >
      <span aria-hidden className="size-1.5 rounded-full bg-current" />
      edited
    </span>
  );
}
