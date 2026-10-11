import type { ThinkingLevel } from "@/types/generated/ThinkingLevel";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";

const LABEL: Record<ThinkingLevel, string> = {
  off: "Off",
  low: "Low",
  medium: "Medium",
  high: "High",
};

/**
 * Segmented Thinking control with exactly the levels the model offers.
 * Clicking the selected level clears it (null = the model's own default).
 */
export function ThinkingControl({
  levels,
  value,
  onChange,
  labelledBy,
}: {
  levels: ThinkingLevel[];
  value: ThinkingLevel | null;
  onChange: (level: ThinkingLevel | null) => void;
  labelledBy: string;
}) {
  return (
    <ToggleGroup
      type="single"
      size="sm"
      spacing={0}
      value={value ?? ""}
      onValueChange={(v) => onChange(v === "" ? null : (v as ThinkingLevel))}
      aria-labelledby={labelledBy}
      className="shrink-0 rounded-[14px] border border-input bg-[color:var(--color-obsidian)] p-0.5"
    >
      {levels.map((level) => (
        <ToggleGroupItem
          key={level}
          value={level}
          className="h-7 min-w-0 rounded-[11px]! px-3 text-[12px] font-normal text-muted-foreground hover:bg-transparent hover:text-foreground data-[state=on]:bg-[color:var(--color-raised)] data-[state=on]:text-primary data-[state=on]:[box-shadow:inset_0_0_0_1px_color-mix(in_oklch,var(--color-gold)_40%,transparent)]"
        >
          {LABEL[level]}
        </ToggleGroupItem>
      ))}
    </ToggleGroup>
  );
}

/** One line under/next to the control, so the choice is explained in words. */
export function thinkingHelp(value: ThinkingLevel | null): string {
  if (value === null) return "Not set — the model decides. Deeper = better on tricky dialogue, slower.";
  if (value === "off") return "No extra reasoning. Fastest.";
  return "Deeper = better on tricky dialogue, slower.";
}
