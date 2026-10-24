import { useState } from "react";
import { toast } from "sonner";
import { Stop } from "@phosphor-icons/react";
import type { TermRef } from "@/types/generated/TermRef";
import { ipc } from "@/lib/ipc";
import { cn, plural } from "@/lib/utils";
import { useGlossaryRun } from "@/stores/glossary-store";
import { usePrimaryAction } from "@/hooks/use-primary-action";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { HelpText } from "@/components/help-text";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { commit, readState } from "./doc-store";
import { setShowName } from "./edits";
import { PHASE_LABEL } from "./extraction-console";
import { CATEGORIES, CATEGORY_LABEL, type Category, type GlossaryState } from "./model";
import {
  applyProposed,
  initialTicked,
  proposedChanges,
  summaryLine,
  type ProposedChange,
} from "./proposed";
import { scopeCount, toGlossaryScope } from "./scope";
import { PanelLabel, SidePanel } from "./side-panel";
import { nextRunId, useGlossaryUi, type ImproveState, type ScopeChoice } from "./ui-store";

/** Runs the user pressed Stop on: their (empty) results are thrown away. */
const stoppedRuns = new Set<number>();

/**
 * Tidy up / Look up names online on the glossary as it is: pick the terms,
 * run, then tick the proposed changes to apply. Nothing is saved until Apply,
 * and terms the user owns start unticked.
 */
export function ImprovePanel({
  folder,
  state,
  selected,
  searchModel,
}: {
  folder: string;
  state: GlossaryState;
  /** The ticked rows (for "Selected (n)"). */
  selected: TermRef[];
  searchModel: string | null;
}) {
  const improve = useGlossaryUi((s) => s.improve);
  const setImprove = useGlossaryUi((s) => s.setImprove);
  const closePanel = useGlossaryUi((s) => s.closePanel);
  const busy = useGlossaryRun((s) => s.busy);
  const [confirmClose, setConfirmClose] = useState(false);

  const count = improve ? scopeCount(improve.scope, state.doc, selected.length) : 0;
  const lookup = improve?.mode === "lookup";
  const canRun =
    !!improve &&
    improve.stage === "setup" &&
    busy === null &&
    count > 0 &&
    (!lookup || (!!searchModel && improve.showName.trim() !== ""));
  const ticked = new Set(improve?.ticked ?? []);
  const canApply = improve?.stage === "review" && ticked.size > 0;

  const run = async () => {
    if (!improve || !canRun) return;
    const mode = improve.mode;
    const showName = improve.showName.trim();
    if (mode === "lookup") {
      // Remembered for the folder; not an undo step.
      const cur = readState(folder);
      if (cur && cur.meta.show_name !== showName) {
        commit(folder, setShowName(cur, showName), { label: "Show name", undoable: false });
      }
    }
    const scope = toGlossaryScope(improve.scope, selected);
    const runId = nextRunId();
    setImprove({ stage: "running", runId, runCount: count });
    useGlossaryRun.getState().startOp(mode === "tidy" ? "normalize" : "lookup", folder);
    try {
      const review =
        mode === "tidy"
          ? await ipc.normalizeGlossary(folder, scope)
          : await ipc.lookupGlossaryNames(folder, showName, scope);
      if (useGlossaryUi.getState().improve?.runId !== runId) return;
      if (stoppedRuns.has(runId)) {
        setImprove({ stage: "setup" });
        toast.info("Stopped. Nothing was changed.");
        return;
      }
      const meta = readState(folder)?.meta ?? state.meta;
      const changes = proposedChanges(review.diff, meta);
      setImprove({ stage: "review", changes, ticked: [...initialTicked(changes)] });
    } catch (e: unknown) {
      if (useGlossaryUi.getState().improve?.runId === runId) setImprove({ stage: "setup" });
      if (!stoppedRuns.has(runId)) {
        // Look up's errors are plain sentences written to be shown as they are.
        toast.error(mode === "tidy" ? `Tidy up didn't finish. ${String(e)}` : String(e));
      }
    } finally {
      stoppedRuns.delete(runId);
      // These commands answer with their result and send no `done` event:
      // clear the op and the phase they left behind.
      useGlossaryRun.getState().endOp();
      useGlossaryRun.setState({ phase: null, phaseDetail: null });
    }
  };

  const stop = () => {
    if (!improve) return;
    stoppedRuns.add(improve.runId);
    ipc.cancelGlossaryBuild().catch((e: unknown) => toast.error(String(e)));
  };

  const apply = () => {
    if (!improve || !canApply) return;
    const cur = readState(folder);
    if (!cur) return;
    const kind = improve.mode === "tidy" ? "tidy_up" : "look_up";
    const { state: next, applied } = applyProposed(cur, improve.changes, ticked, kind, Date.now());
    closePanel();
    if (!applied) {
      toast.info("Those changes no longer apply to the glossary.");
      return;
    }
    commit(folder, next, {
      label: `Applied ${plural(applied, "change")} from ${improve.mode === "tidy" ? "Tidy up" : "Look up names online"}`,
      toast: `Applied ${plural(applied, "change")}. A copy was saved first.`,
      snapshot: kind,
    });
  };

  usePrimaryAction(
    () => void (improve?.stage === "review" ? apply() : run()),
    improve?.stage === "review" ? canApply : canRun,
  );

  if (!improve) return null;

  const unapplied = improve.stage === "review" ? improve.changes.length : 0;
  const requestClose = () => {
    if (improve.stage === "running") return;
    if (unapplied > 0) setConfirmClose(true);
    else closePanel();
  };

  const title =
    improve.stage === "review"
      ? `Proposed changes · ${improve.changes.length}`
      : lookup
        ? "Look up names online"
        : "Tidy up";

  return (
    <>
      <SidePanel
        title={title}
        onClose={requestClose}
        canClose={improve.stage !== "running"}
        footer={
          improve.stage === "setup" ? (
            <Button disabled={!canRun} onClick={() => void run()}>
              {lookup ? "Look up" : "Tidy up"} {plural(count, "term")}
            </Button>
          ) : improve.stage === "running" ? (
            <Button variant="outline" onClick={stop}>
              <Stop className="size-4" /> Stop
            </Button>
          ) : improve.changes.length === 0 ? (
            <Button variant="outline" onClick={closePanel}>
              Close
            </Button>
          ) : (
            <>
              <Button variant="ghost" onClick={requestClose}>
                Discard
              </Button>
              <Button disabled={!canApply} onClick={apply}>
                Apply {plural(ticked.size, "change")}
              </Button>
            </>
          )
        }
      >
        {improve.stage === "setup" ? (
          <SetupStage
            improve={improve}
            state={state}
            selectedCount={selected.length}
            searchModel={searchModel}
            onChange={setImprove}
          />
        ) : improve.stage === "running" ? (
          <RunningStage mode={improve.mode} count={improve.runCount} />
        ) : (
          <ReviewStage
            improve={improve}
            onTicked={(ids) => setImprove({ ticked: ids })}
          />
        )}
      </SidePanel>

      <AlertDialog open={confirmClose} onOpenChange={setConfirmClose}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Discard {plural(unapplied, "proposed change")}?</AlertDialogTitle>
            <AlertDialogDescription>
              Nothing has been saved. To get these suggestions back you would have to run{" "}
              {lookup ? "Look up names online" : "Tidy up"} again.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep reviewing</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={closePanel}>
              Discard
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

function SetupStage({
  improve,
  state,
  selectedCount,
  searchModel,
  onChange,
}: {
  improve: ImproveState;
  state: GlossaryState;
  selectedCount: number;
  searchModel: string | null;
  onChange: (patch: Partial<ImproveState>) => void;
}) {
  const lookup = improve.mode === "lookup";
  const scope = improve.scope;
  const pick = (s: ScopeChoice) => onChange({ scope: s });
  const catOf = (c: Category) => Object.keys(state.doc.terms[c] ?? {}).length;
  const [category, setCategory] = useState<Category>(
    scope.kind === "category" ? scope.category : (CATEGORIES.find((c) => catOf(c) > 0) ?? "characters"),
  );

  return (
    <div className="flex flex-col gap-5">
      <p className="text-[12.5px] text-muted-foreground">
        {lookup
          ? "Searches the web for the names fans of this show already use."
          : "Fixes inconsistent spellings and drops terms too generic to be useful."}{" "}
        You see every change before anything is saved.
      </p>

      <div>
        <PanelLabel>Which terms?</PanelLabel>
        <div role="radiogroup" aria-label="Which terms" className="flex flex-col gap-1">
          {scope.kind === "one" ? (
            <Radio checked onSelect={() => {}}>
              Just {scope.ref.source}
            </Radio>
          ) : null}
          <Radio checked={scope.kind === "all"} onSelect={() => pick({ kind: "all" })}>
            All {plural(state.doc.count, "term")}
          </Radio>
          <Radio
            checked={scope.kind === "selected"}
            disabled={selectedCount === 0}
            onSelect={() => pick({ kind: "selected" })}
          >
            Selected ({selectedCount})
          </Radio>
          <div className="flex items-center gap-2">
            <Radio
              checked={scope.kind === "category"}
              onSelect={() => pick({ kind: "category", category })}
            >
              One category
            </Radio>
            <Select
              value={category}
              onValueChange={(v) => {
                setCategory(v as Category);
                pick({ kind: "category", category: v as Category });
              }}
            >
              <SelectTrigger size="sm" className="h-7 px-2.5 text-[12px] data-[size=sm]:h-7" aria-label="Category">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {CATEGORIES.map((c) => (
                  <SelectItem key={c} value={c} className="text-[13px]">
                    {CATEGORY_LABEL[c]} <span className="text-muted-foreground tabular-nums">{catOf(c)}</span>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>
        <HelpText>A smaller set is faster and cheaper, and less likely to time out.</HelpText>
      </div>

      {lookup ? (
        <div>
          <PanelLabel>Show name</PanelLabel>
          <Input
            value={improve.showName}
            onChange={(e) => onChange({ showName: e.target.value })}
            placeholder="The show’s name, as fans write it"
            aria-label="Show name"
          />
          <HelpText>Remembered for this folder. Your subtitle files aren’t read again.</HelpText>
          {searchModel ? (
            <p className="mt-2 text-[11px] text-muted-foreground">Uses {searchModel}</p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function Radio({
  checked,
  disabled = false,
  onSelect,
  children,
}: {
  checked: boolean;
  disabled?: boolean;
  onSelect: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={checked}
      disabled={disabled}
      onClick={onSelect}
      className="flex items-center gap-2.5 rounded-lg py-1 pr-2 text-left text-[13px] text-foreground outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50"
    >
      <span
        className={cn(
          "flex size-4 shrink-0 items-center justify-center rounded-full border",
          checked ? "border-primary" : "border-[color:var(--input)]",
        )}
      >
        {checked ? <span className="size-2 rounded-full bg-primary" /> : null}
      </span>
      {children}
    </button>
  );
}

function RunningStage({ mode, count }: { mode: ImproveState["mode"]; count: number }) {
  const phase = useGlossaryRun((s) => s.phase);
  const done = useGlossaryRun((s) => s.done);
  const total = useGlossaryRun((s) => s.total);
  const detail = useGlossaryRun((s) => s.phaseDetail);
  const lastLog = useGlossaryRun((s) => s.logs[s.logs.length - 1]?.message ?? null);
  const label = phase ? PHASE_LABEL[phase] : mode === "tidy" ? "Tidying up" : "Looking up names";
  return (
    <div className="flex flex-col gap-3">
      <p className="text-[13px] text-foreground">
        {label} · {detail ?? plural(count, "term")}…
      </p>
      <div className="h-1.5 overflow-hidden rounded-full bg-[color:var(--color-raised)]">
        {total > 0 ? (
          <div
            className="h-full rounded-full bg-primary transition-[width] duration-300"
            style={{ width: `${Math.round((done / total) * 100)}%` }}
          />
        ) : (
          <div
            className="h-full w-full [animation:sig-shimmer_1.6s_linear_infinite] bg-[length:200%_100%]"
            style={{
              backgroundImage:
                "linear-gradient(90deg, transparent 0%, color-mix(in oklch, var(--color-gold) 70%, transparent) 50%, transparent 100%)",
            }}
          />
        )}
      </div>
      {total > 0 ? (
        <p className="text-[11.5px] tabular-nums text-muted-foreground">
          batch {done} of {total}
        </p>
      ) : null}
      {lastLog ? <p className="truncate text-[11.5px] text-muted-foreground">{lastLog}</p> : null}
      <p className="text-[11.5px] text-muted-foreground">
        Nothing is saved while this runs. You can keep editing; the changes are checked against
        the glossary as it is when you apply them.
      </p>
    </div>
  );
}

function ReviewStage({
  improve,
  onTicked,
}: {
  improve: ImproveState;
  onTicked: (ids: string[]) => void;
}) {
  const changes = improve.changes;
  const ticked = new Set(improve.ticked);
  const all = changes.length > 0 && changes.every((c) => ticked.has(c.id));
  const some = !all && changes.some((c) => ticked.has(c.id));
  const anyYours = changes.some((c) => c.yours);

  if (changes.length === 0) {
    return (
      <p className="text-[13px] text-muted-foreground">
        {improve.mode === "lookup"
          ? "The lookup didn’t suggest any changes for these terms."
          : "Nothing to tidy. These terms already look consistent."}
      </p>
    );
  }

  const toggle = (id: string, on: boolean) => {
    const next = new Set(ticked);
    if (on) next.add(id);
    else next.delete(id);
    onTicked([...next]);
  };

  return (
    <div className="flex flex-col gap-3">
      <p className="text-[12px] text-muted-foreground">
        From {improve.mode === "lookup" ? "the online lookup" : "Tidy up"}. Nothing is saved yet.
      </p>
      <div className="flex items-center justify-between border-b border-border pb-2">
        <label className="flex cursor-pointer items-center gap-2.5 text-[12.5px]">
          <Checkbox
            checked={all ? true : some ? "indeterminate" : false}
            onCheckedChange={() => onTicked(all ? [] : changes.map((c) => c.id))}
          />
          All
        </label>
        <span className="text-[11.5px] tabular-nums text-muted-foreground">{summaryLine(changes)}</span>
      </div>
      <ul className="-mx-2 flex flex-col">
        {changes.map((c) => (
          <li key={c.id}>
            <label className="flex cursor-pointer items-start gap-2.5 rounded-lg px-2 py-1.5 hover:bg-[color:var(--color-bg-hover)]">
              <Checkbox
                className="mt-0.5"
                checked={ticked.has(c.id)}
                onCheckedChange={(v) => toggle(c.id, v === true)}
              />
              <span className="w-24 shrink-0">
                <span className="block truncate text-[13px] text-[color:var(--color-ink-emphasis)]">{c.source}</span>
                <span className="block text-[10.5px] text-muted-foreground">
                  {CATEGORY_LABEL[c.from ?? c.category]}
                  {c.yours ? " · Yours" : ""}
                </span>
              </span>
              <ChangeText change={c} />
            </label>
          </li>
        ))}
      </ul>
      {anyYours ? (
        <HelpText>Terms marked Yours are unticked to start with: you typed or accepted them.</HelpText>
      ) : null}
    </div>
  );
}

function ChangeText({ change: c }: { change: ProposedChange }) {
  const arrow = <span className="text-muted-foreground"> → </span>;
  switch (c.kind) {
    case "changed":
      return (
        <span className="min-w-0 flex-1 text-[12.5px]">
          <span className="text-muted-foreground line-through">{c.old}</span>
          {arrow}
          <span className="text-foreground">{c.new}</span>
        </span>
      );
    case "removed":
      return (
        <span className="min-w-0 flex-1 text-[12.5px]">
          <span className="text-muted-foreground line-through">{c.old}</span>
          {arrow}
          <span className="text-[color:var(--color-danger)]">remove</span>
        </span>
      );
    case "added":
      return (
        <span className="min-w-0 flex-1 text-[12.5px] text-[color:var(--color-success)]">+ {c.new}</span>
      );
    case "moved":
      return (
        <span className="min-w-0 flex-1 text-[12.5px]">
          <span className="text-foreground">Move to {CATEGORY_LABEL[c.category]}</span>
          {c.new !== null && c.new !== c.old ? (
            <>
              {" · "}
              <span className="text-muted-foreground line-through">{c.old}</span>
              {arrow}
              <span className="text-foreground">{c.new}</span>
            </>
          ) : null}
        </span>
      );
  }
}
