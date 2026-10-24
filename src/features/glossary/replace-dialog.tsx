import { useEffect, useMemo, useState } from "react";
import { ArrowRight } from "@phosphor-icons/react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useDebouncedValue } from "@/hooks/use-debounced-value";
import { cn, plural } from "@/lib/utils";
import type { TermRow } from "./model";
import { findReplaceMatches, type ReplaceOptions } from "./replace";

/** "Replace in translations…": find, replace, preview with a tick per match. */
export function ReplaceDialog({
  open,
  onOpenChange,
  allRows,
  selectedRows,
  onApply,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  allRows: TermRow[];
  /** Ticked rows; the dialog starts scoped to them when there are any. */
  selectedRows: TermRow[];
  onApply: (keys: string[], opts: ReplaceOptions) => void;
}) {
  const [find, setFind] = useState("");
  const [replace, setReplace] = useState("");
  const [matchCase, setMatchCase] = useState(false);
  const [onlySelected, setOnlySelected] = useState(true);
  const [unticked, setUnticked] = useState<Set<string>>(new Set());

  // Fresh each time it opens.
  useEffect(() => {
    if (!open) return;
    setFind("");
    setReplace("");
    setMatchCase(false);
    setOnlySelected(selectedRows.length > 0);
    setUnticked(new Set());
    // Only on open: the selection can't change while the dialog is up.
  }, [open]);

  const scoped = onlySelected && selectedRows.length > 0 ? selectedRows : allRows;
  const opts: ReplaceOptions = { find: useDebouncedValue(find, 120), replace, matchCase };
  const matches = useMemo(
    () => findReplaceMatches(scoped, opts),
    [scoped, opts.find, opts.replace, opts.matchCase],
  );
  const ticked = matches.filter((m) => !m.empty && !unticked.has(m.key));

  const toggle = (key: string, on: boolean) =>
    setUnticked((s) => {
      const next = new Set(s);
      if (on) next.delete(key);
      else next.add(key);
      return next;
    });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[85vh] flex-col gap-4 sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>Replace in translations</DialogTitle>
          <DialogDescription>
            Fixes a name across many terms at once. Nothing changes until you press Replace.
          </DialogDescription>
        </DialogHeader>

        <div className="grid grid-cols-[1fr_auto_1fr] items-center gap-2">
          <Input
            autoFocus
            value={find}
            onChange={(e) => setFind(e.target.value)}
            placeholder="Find, e.g. Jiu Ying"
            aria-label="Find"
          />
          <ArrowRight className="size-4 text-muted-foreground" />
          <Input
            value={replace}
            onChange={(e) => setReplace(e.target.value)}
            placeholder="Replace with, e.g. Jiuying"
            aria-label="Replace with"
          />
        </div>
        <div className="flex flex-wrap items-center gap-x-5 gap-y-2 text-[12.5px]">
          <label className="flex cursor-pointer items-center gap-2">
            <Checkbox checked={matchCase} onCheckedChange={(v) => setMatchCase(v === true)} />
            Match case
          </label>
          {selectedRows.length > 0 ? (
            <label className="flex cursor-pointer items-center gap-2">
              <Checkbox checked={onlySelected} onCheckedChange={(v) => setOnlySelected(v === true)} />
              Only the {plural(selectedRows.length, "selected term")}
            </label>
          ) : null}
        </div>

        <div className="min-h-24 flex-1 overflow-auto rounded-xl border border-border bg-[color:var(--card)]">
          {!opts.find ? (
            <p className="p-4 text-[12.5px] text-muted-foreground">Type what to find to see the matches.</p>
          ) : matches.length === 0 ? (
            <p className="p-4 text-[12.5px] text-muted-foreground">
              No translation contains “{opts.find}”{onlySelected && selectedRows.length ? " in the selected terms" : ""}.
            </p>
          ) : (
            <ul className="py-1">
              {matches.map((m) => (
                <li key={m.key}>
                  <label
                    className={cn(
                      "flex items-start gap-2.5 px-3 py-1.5 text-[12.5px] hover:bg-[color:var(--color-bg-hover)]",
                      m.empty ? "cursor-not-allowed opacity-60" : "cursor-pointer",
                    )}
                  >
                    <Checkbox
                      className="mt-0.5"
                      checked={!m.empty && !unticked.has(m.key)}
                      disabled={m.empty}
                      onCheckedChange={(v) => toggle(m.key, v === true)}
                    />
                    <span className="w-28 shrink-0 truncate text-[color:var(--color-ink-emphasis)]">{m.source}</span>
                    <span className="min-w-0 flex-1">
                      <span className="text-muted-foreground line-through">{m.before}</span>
                      <span className="text-muted-foreground"> → </span>
                      {m.empty ? (
                        <span className="text-[color:var(--color-danger)]">would be empty</span>
                      ) : (
                        <span className="text-foreground">{m.after}</span>
                      )}
                    </span>
                  </label>
                </li>
              ))}
            </ul>
          )}
        </div>

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            disabled={ticked.length === 0}
            onClick={() => onApply(ticked.map((m) => m.key), opts)}
          >
            Replace in {plural(ticked.length, "term")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
