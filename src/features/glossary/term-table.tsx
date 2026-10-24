import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Plus } from "@phosphor-icons/react";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { Category, TermRow } from "./model";
import {
  CategorySelect,
  GRID_COLS,
  ROW_HEIGHT,
  TermRowView,
  type EditField,
  type RowActions,
} from "./term-row";

export interface AddDraft {
  source: string;
  target: string;
  category: Category;
}

/** Scroll to a row and light it up; `n` re-fires the same key. */
export interface Highlight {
  key: string;
  n: number;
  /** Longer and stronger: "Fix in glossary" landing on a term. */
  strong?: boolean;
}

export function TermTable({
  rows,
  selected,
  onSelectRows,
  sourceLabel,
  targetLabel,
  disabled,
  draft,
  onDraftChange,
  onAdd,
  addSourceRef,
  highlight,
  bottomPad = 0,
  actions,
  empty,
}: {
  /** The visible (filtered) rows. */
  rows: TermRow[];
  selected: Set<string>;
  onSelectRows: (keys: string[], on: boolean) => void;
  sourceLabel: string;
  targetLabel: string;
  disabled: boolean;
  draft: AddDraft;
  onDraftChange: (d: AddDraft) => void;
  /** True when the term went in (the row clears and focus goes back to source). */
  onAdd: () => boolean;
  addSourceRef: React.RefObject<HTMLInputElement | null>;
  highlight: Highlight | null;
  /** Room under the last row (the bulk bar floats there). */
  bottomPad?: number;
  actions: Omit<RowActions, "onSelect" | "onStartEdit" | "onCommitEdit" | "onCancelEdit"> & {
    /** Null when the edit went in; otherwise why it was refused. */
    onCommitEdit: (row: TermRow, field: EditField, value: string) => string | null;
  };
  /** Shown instead of rows when there are none. */
  empty: ReactNode;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  /** The open edit; `error` says why its last save was refused. */
  const [editing, setEditing] = useState<{ key: string; field: EditField; error?: string } | null>(
    null,
  );
  const [flash, setFlash] = useState<string | null>(null);

  // Header columns line up with the body whether or not a scrollbar is drawn.
  const [gutter, setGutter] = useState(0);
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const measure = () => setGutter(el.offsetWidth - el.clientWidth);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    getItemKey: (i) => rows[i]?.key ?? i,
    overscan: 12,
    paddingEnd: bottomPad,
  });

  const indexOf = useMemo(() => {
    const m = new Map<string, number>();
    rows.forEach((r, i) => m.set(r.key, i));
    return m;
  }, [rows]);

  // An edit whose row went away (filtered out, deleted, undone) ends.
  useEffect(() => {
    if (editing && !indexOf.has(editing.key)) setEditing(null);
  }, [editing, indexOf]);

  // Scroll to and flash a row (new term, "Fix in glossary"). The row can land
  // a render after the request (the cache update and the request aren't one
  // batch), so wait for it briefly instead of giving up on the first miss.
  const handled = useRef(0);
  const requestedAt = useRef(0);
  useEffect(() => {
    if (highlight) requestedAt.current = Date.now();
  }, [highlight]);
  useEffect(() => {
    if (!highlight || handled.current === highlight.n) return;
    const i = indexOf.get(highlight.key);
    if (i === undefined) {
      if (Date.now() - requestedAt.current > 1500) handled.current = highlight.n;
      return;
    }
    handled.current = highlight.n;
    virtualizer.scrollToIndex(i, { align: highlight.strong ? "center" : "auto" });
    setFlash(highlight.key);
  }, [highlight, indexOf, virtualizer]);
  useEffect(() => {
    if (!flash) return;
    const t = setTimeout(() => setFlash(null), highlight?.strong ? 2600 : 1300);
    return () => clearTimeout(t);
  }, [flash, highlight?.n]);

  // Stable callbacks for the memoised rows.
  const rowsRef = useRef(rows);
  rowsRef.current = rows;
  const outer = useRef(actions);
  outer.current = actions;
  const onSelectRowsRef = useRef(onSelectRows);
  onSelectRowsRef.current = onSelectRows;
  const rowActions = useMemo<RowActions>(
    () => ({
      onSelect: (key, on) => onSelectRowsRef.current([key], on),
      onStartEdit: (key, field) => setEditing({ key, field }),
      onCancelEdit: () => setEditing(null),
      onCommitEdit: (row, field, value, viaEnter) => {
        const list = rowsRef.current;
        const next = viaEnter ? list[list.findIndex((r) => r.key === row.key) + 1] : undefined;
        const error = outer.current.onCommitEdit(row, field, value);
        if (error) {
          // Refused (a duplicate, an empty term): the field stays open with
          // what was typed, and says why.
          setEditing({ key: row.key, field, error });
          return false;
        }
        if (next) {
          setEditing({ key: next.key, field });
          const i = list.indexOf(next);
          virtualizer.scrollToIndex(i, { align: "auto" });
        } else {
          setEditing(null);
        }
        return true;
      },
      onMove: (row, to) => outer.current.onMove(row, to),
      onDelete: (row) => outer.current.onDelete(row),
      onMarkReviewed: (row) => outer.current.onMarkReviewed(row),
      onLookup: (row) => outer.current.onLookup?.(row),
    }),
    [virtualizer],
  );
  // Without a lookup model the row menu leaves the item out.
  const actionsForRows = useMemo<RowActions>(
    () => (actions.onLookup ? rowActions : { ...rowActions, onLookup: undefined }),
    [rowActions, actions.onLookup ? 1 : 0],
  );

  const allSelected = rows.length > 0 && rows.every((r) => selected.has(r.key));
  const someSelected = !allSelected && rows.some((r) => selected.has(r.key));

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* Column heads */}
      <div
        className={cn(
          GRID_COLS,
          "h-9 border-b border-border px-3 text-[10.5px] font-semibold tracking-[0.12em] text-muted-foreground uppercase",
        )}
        style={{ paddingRight: 12 + gutter }}
      >
        <div className="flex justify-center">
          <Checkbox
            aria-label="Select all shown"
            checked={allSelected ? true : someSelected ? "indeterminate" : false}
            disabled={rows.length === 0}
            onCheckedChange={() => onSelectRows(rows.map((r) => r.key), !allSelected)}
          />
        </div>
        <div className="px-2">{sourceLabel}</div>
        <div className="px-2">{targetLabel}</div>
        <div className="px-1">Category</div>
        <div />
      </div>

      {/* The add row: always first, outside the scroller so it stays put. */}
      <AddRow
        draft={draft}
        onDraftChange={onDraftChange}
        onAdd={onAdd}
        sourceRef={addSourceRef}
        disabled={disabled}
        gutter={gutter}
      />

      <div ref={scrollRef} className="min-h-0 flex-1 overflow-auto" role="rowgroup">
        {rows.length === 0 ? (
          empty
        ) : (
          <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
            {virtualizer.getVirtualItems().map((v) => {
              const row = rows[v.index];
              const editError = editing?.key === row.key ? (editing.error ?? null) : null;
              return (
                <div
                  key={v.key}
                  // A refusal note hangs below the row, over the next one.
                  className={cn("absolute inset-x-0 top-0", editError && "z-10")}
                  style={{ height: ROW_HEIGHT, transform: `translateY(${v.start}px)` }}
                >
                  <TermRowView
                    row={row}
                    selected={selected.has(row.key)}
                    editing={editing?.key === row.key ? editing.field : null}
                    editError={editError}
                    flash={flash === row.key}
                    disabled={disabled}
                    actions={actionsForRows}
                  />
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}

function AddRow({
  draft,
  onDraftChange,
  onAdd,
  sourceRef,
  disabled,
  gutter,
}: {
  draft: AddDraft;
  onDraftChange: (d: AddDraft) => void;
  onAdd: () => boolean;
  sourceRef: React.RefObject<HTMLInputElement | null>;
  disabled: boolean;
  gutter: number;
}) {
  const targetRef = useRef<HTMLInputElement>(null);
  const submit = () => {
    if (onAdd()) sourceRef.current?.focus();
  };
  const clearOnEsc = (e: React.KeyboardEvent) => {
    if (e.key !== "Escape" || (!draft.source && !draft.target)) return;
    e.preventDefault();
    onDraftChange({ ...draft, source: "", target: "" });
  };
  return (
    <div
      className={cn(GRID_COLS, "h-12 border-b border-border bg-[color:var(--color-surface)] px-3")}
      style={{ paddingRight: 12 + gutter }}
    >
      <div className="flex justify-center text-primary">
        <Plus weight="bold" className="size-3.5" />
      </div>
      <Input
        ref={sourceRef}
        value={draft.source}
        disabled={disabled}
        placeholder="New term…"
        aria-label="New term"
        className="h-8 text-[13px] md:text-[13px]"
        onChange={(e) => onDraftChange({ ...draft, source: e.target.value })}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            targetRef.current?.focus();
          } else clearOnEsc(e);
        }}
      />
      <Input
        ref={targetRef}
        value={draft.target}
        disabled={disabled}
        placeholder="Translation…"
        aria-label="Translation of the new term"
        className="h-8 text-[13px] md:text-[13px]"
        onChange={(e) => onDraftChange({ ...draft, target: e.target.value })}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            submit();
          } else clearOnEsc(e);
        }}
      />
      <CategorySelect
        value={draft.category}
        disabled={disabled}
        onChange={(category) => onDraftChange({ ...draft, category })}
      />
      <div className="flex justify-center">
        <Button
          variant="outline"
          size="xs"
          disabled={disabled || !draft.source.trim() || !draft.target.trim()}
          onClick={submit}
        >
          Add
        </Button>
      </div>
    </div>
  );
}
