import { memo, useRef, type ReactNode } from "react";
import {
  ArrowRight,
  Check,
  DotsThree,
  Globe,
  PencilSimple,
  Trash,
} from "@phosphor-icons/react";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";
import { CATEGORIES, CATEGORY_LABEL, type Category, type TermRow } from "./model";

/** Checkbox · source · translation · category · ··· */
export const GRID_COLS =
  "grid grid-cols-[40px_minmax(0,0.85fr)_minmax(0,1.15fr)_158px_44px] items-center gap-x-1";

export const ROW_HEIGHT = 40;

export type EditField = "source" | "target";

export interface RowActions {
  onSelect: (key: string, on: boolean) => void;
  onStartEdit: (key: string, field: EditField) => void;
  /** False when the edit was refused (the editor stays open with the text). */
  onCommitEdit: (row: TermRow, field: EditField, value: string, viaEnter: boolean) => boolean;
  onCancelEdit: () => void;
  onMove: (row: TermRow, to: Category) => void;
  onDelete: (row: TermRow) => void;
  onMarkReviewed: (row: TermRow) => void;
  /** Absent when no model can search the web. */
  onLookup?: (row: TermRow) => void;
}

export const TermRowView = memo(function TermRowView({
  row,
  selected,
  editing,
  editError = null,
  flash,
  disabled,
  actions,
}: {
  row: TermRow;
  selected: boolean;
  editing: EditField | null;
  /** Why the last try at saving the open edit was refused. */
  editError?: string | null;
  flash: boolean;
  disabled: boolean;
  actions: RowActions;
}) {
  return (
    <div
      role="row"
      data-key={row.key}
      className={cn(
        GRID_COLS,
        "group h-full border-b border-border/60 px-3 transition-colors duration-700",
        selected && "bg-[color:color-mix(in_oklch,var(--color-gold)_6%,transparent)]",
        flash && "bg-[color:color-mix(in_oklch,var(--color-gold)_16%,transparent)] duration-150",
        !selected && !flash && "hover:bg-[color:var(--color-bg-hover)]/60",
      )}
    >
      <div className="flex justify-center">
        <Checkbox
          checked={selected}
          aria-label={`Select ${row.source}`}
          onCheckedChange={(v) => actions.onSelect(row.key, v === true)}
        />
      </div>
      <EditableCell
        value={row.source}
        editing={editing === "source"}
        error={editing === "source" ? editError : null}
        disabled={disabled}
        label={`Edit ${row.source}`}
        className="text-[13.5px] text-[color:var(--color-ink-emphasis)]"
        after={row.isNew ? <NewTag /> : null}
        onStart={() => actions.onStartEdit(row.key, "source")}
        onCommit={(v, enter) => actions.onCommitEdit(row, "source", v, enter)}
        onCancel={actions.onCancelEdit}
      />
      <EditableCell
        value={row.target}
        editing={editing === "target"}
        error={editing === "target" ? editError : null}
        disabled={disabled}
        label={`Edit the translation of ${row.source}`}
        className="text-[13px] text-foreground"
        onStart={() => actions.onStartEdit(row.key, "target")}
        onCommit={(v, enter) => actions.onCommitEdit(row, "target", v, enter)}
        onCancel={actions.onCancelEdit}
      />
      <CategorySelect
        value={row.category}
        disabled={disabled}
        onChange={(c) => actions.onMove(row, c)}
      />
      <RowMenu row={row} disabled={disabled} actions={actions} />
    </div>
  );
});

export function NewTag() {
  return (
    <span className="shrink-0 rounded-[13px] border border-[color:var(--color-success)]/45 px-1.5 py-px text-[9px] font-semibold tracking-[0.12em] text-[color:var(--color-success)]">
      NEW
    </span>
  );
}

/** A cell that edits on a single click. Enter saves (and the table moves to the
 *  next row), Esc cancels, leaving the field saves. A refused edit keeps the
 *  field open with what was typed, and says why under it. */
function EditableCell({
  value,
  editing,
  error,
  disabled,
  label,
  className,
  after,
  onStart,
  onCommit,
  onCancel,
}: {
  value: string;
  editing: boolean;
  error: string | null;
  disabled: boolean;
  label: string;
  className?: string;
  after?: ReactNode;
  onStart: () => void;
  onCommit: (value: string, viaEnter: boolean) => boolean;
  onCancel: () => void;
}) {
  if (editing) {
    return <InlineInput initial={value} error={error} onCommit={onCommit} onCancel={onCancel} />;
  }
  return (
    <button
      type="button"
      aria-label={label}
      disabled={disabled}
      onClick={onStart}
      className={cn(
        "group/cell flex h-8 min-w-0 items-center gap-2 rounded-lg px-2 text-left outline-none",
        "hover:bg-[color:var(--color-bg-hover)] focus-visible:ring-[3px] focus-visible:ring-ring/50",
        "disabled:pointer-events-none",
        className,
      )}
    >
      <span className="min-w-0 truncate">{value}</span>
      {after}
      <PencilSimple className="ml-auto size-3.5 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover/cell:opacity-100 group-focus-visible/cell:opacity-100" />
    </button>
  );
}

function InlineInput({
  initial,
  error,
  onCommit,
  onCancel,
}: {
  initial: string;
  error: string | null;
  onCommit: (value: string, viaEnter: boolean) => boolean;
  onCancel: () => void;
}) {
  // Enter/Esc unmount the input, which fires blur: `done` keeps the blur from
  // committing a second time (or bringing back a cancelled edit). A refused
  // Enter leaves the input open, so it isn't done.
  const done = useRef(false);
  return (
    <div className="relative min-w-0">
      <Input
        autoFocus
        defaultValue={initial}
        aria-invalid={error ? true : undefined}
        onFocus={(e) => e.currentTarget.select()}
        className="h-8 min-w-0 rounded-lg px-2 text-[13px] md:text-[13px]"
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            done.current = true;
            if (!onCommit(e.currentTarget.value, true)) done.current = false;
          } else if (e.key === "Escape") {
            // Ours: the side panel must not close on this Esc.
            e.preventDefault();
            done.current = true;
            onCancel();
          }
        }}
        onBlur={(e) => {
          if (!done.current) onCommit(e.currentTarget.value, false);
        }}
      />
      {error ? (
        <p
          role="alert"
          className="absolute top-full left-0 z-10 mt-1 w-max max-w-[300px] rounded-md border border-[color:var(--color-danger)]/40 bg-[color:var(--popover)] px-2 py-1 text-[11.5px] text-[color:var(--color-danger)] shadow-[0_6px_18px_-8px_rgba(0,0,0,.8)]"
        >
          {error}
        </p>
      ) : null}
    </div>
  );
}

export function CategorySelect({
  value,
  onChange,
  disabled,
  className,
}: {
  value: Category;
  onChange: (c: Category) => void;
  disabled?: boolean;
  className?: string;
}) {
  return (
    <Select value={value} onValueChange={(v) => onChange(v as Category)} disabled={disabled}>
      <SelectTrigger
        size="sm"
        aria-label="Category"
        className={cn("h-7 w-[150px] px-2.5 text-[12px] data-[size=sm]:h-7", className)}
      >
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {CATEGORIES.map((c) => (
          <SelectItem key={c} value={c} className="text-[13px]">
            {CATEGORY_LABEL[c]}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

function RowMenu({
  row,
  disabled,
  actions,
}: {
  row: TermRow;
  disabled: boolean;
  actions: RowActions;
}) {
  // "Edit source" opens an input in the row. Open it only once the menu has
  // closed: while open, the menu traps focus and would blur the input at once.
  const editAfterClose = useRef(false);
  return (
    <div className="flex justify-center">
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={`More for ${row.source}`}
            disabled={disabled}
            className="text-muted-foreground opacity-60 group-hover:opacity-100 data-[state=open]:opacity-100"
          >
            <DotsThree weight="bold" className="size-4" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="end"
          className="w-56"
          onCloseAutoFocus={(e) => {
            if (!editAfterClose.current) return;
            editAfterClose.current = false;
            e.preventDefault();
            actions.onStartEdit(row.key, "source");
          }}
        >
          <DropdownMenuItem
            onSelect={() => {
              editAfterClose.current = true;
            }}
          >
            <PencilSimple /> Edit source
          </DropdownMenuItem>
          <DropdownMenuSub>
            <DropdownMenuSubTrigger>
              <ArrowRight /> Move to
            </DropdownMenuSubTrigger>
            <DropdownMenuSubContent>
              {CATEGORIES.filter((c) => c !== row.category).map((c) => (
                <DropdownMenuItem key={c} onSelect={() => actions.onMove(row, c)}>
                  {CATEGORY_LABEL[c]}
                </DropdownMenuItem>
              ))}
            </DropdownMenuSubContent>
          </DropdownMenuSub>
          {actions.onLookup ? (
            <DropdownMenuItem onSelect={() => actions.onLookup?.(row)}>
              <Globe /> Look up this name online
            </DropdownMenuItem>
          ) : null}
          {row.isNew ? (
            <DropdownMenuItem onSelect={() => actions.onMarkReviewed(row)}>
              <Check /> Mark reviewed
            </DropdownMenuItem>
          ) : null}
          <DropdownMenuSeparator />
          <DropdownMenuItem variant="destructive" onSelect={() => actions.onDelete(row)}>
            <Trash /> Delete
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}
