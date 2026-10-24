import { CaretDown, X } from "@phosphor-icons/react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { CATEGORIES, CATEGORY_LABEL, type Category } from "./model";

/** Floating bar for ticked rows. */
export function BulkBar({
  count,
  anyNew,
  disabled,
  canTidy,
  onMove,
  onReplace,
  onTidy,
  onLookup,
  onMarkReviewed,
  onDelete,
  onClear,
}: {
  count: number;
  /** Some ticked rows carry the NEW tag. */
  anyNew: boolean;
  disabled: boolean;
  canTidy: boolean;
  onMove: (to: Category) => void;
  onReplace: () => void;
  onTidy: () => void;
  /** Absent when no model can search the web. */
  onLookup?: () => void;
  onMarkReviewed: () => void;
  onDelete: () => void;
  onClear: () => void;
}) {
  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-4 z-10 flex justify-center px-4">
      <div
        role="toolbar"
        aria-label="Ticked terms"
        className="pointer-events-auto flex items-center gap-1.5 rounded-[26px] border border-[color:var(--input)] bg-popover py-1.5 pr-1.5 pl-4 animate-in fade-in-0 slide-in-from-bottom-2 duration-150"
      >
        <span className="mr-1.5 text-[12.5px] font-semibold tabular-nums text-foreground">
          {count} selected
        </span>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="outline" size="xs" disabled={disabled}>
              Move to <CaretDown className="size-3" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="center" side="top" className="w-48">
            {CATEGORIES.map((c) => (
              <DropdownMenuItem key={c} onSelect={() => onMove(c)}>
                {CATEGORY_LABEL[c]}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
        <Button variant="outline" size="xs" disabled={disabled} onClick={onReplace}>
          Replace in translations…
        </Button>
        <Button variant="outline" size="xs" disabled={disabled || !canTidy} onClick={onTidy}>
          Tidy up
        </Button>
        {onLookup ? (
          <Button variant="outline" size="xs" disabled={disabled} onClick={onLookup}>
            Look up online
          </Button>
        ) : null}
        {anyNew ? (
          <Button variant="outline" size="xs" disabled={disabled} onClick={onMarkReviewed}>
            Mark reviewed
          </Button>
        ) : null}
        <Button variant="destructive" size="xs" disabled={disabled} onClick={onDelete}>
          Delete
        </Button>
        <Button variant="ghost" size="icon-xs" aria-label="Clear selection" onClick={onClear}>
          <X className="size-3.5" />
        </Button>
      </div>
    </div>
  );
}
