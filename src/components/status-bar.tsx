import type { ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { CaretRight } from "@phosphor-icons/react";
import type { FileResult } from "@/types/generated/FileResult";
import type { FileStateKind } from "@/types/generated/FileStateKind";
import type { Language } from "@/types/generated/Language";
import type { WorldType } from "@/types/generated/WorldType";
import { ipc } from "@/lib/ipc";
import { folderTitle } from "@/lib/folder-title";
import { cn, plural } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";
import { useGlossaryRun } from "@/stores/glossary-store";
import { useTranslationRun, type FileRow } from "@/stores/translation-store";
import { tallyRun } from "@/features/episodes/run-summary";

const WORLD_LABEL: Record<WorldType, string> = {
  xianxia: "Xianxia",
  wuxia: "Wuxia",
  historical: "Historical",
  modern: "Modern",
};

const FINISHED: readonly FileStateKind[] = ["done", "warning", "failed", "stopped"];
const WORKING: readonly FileStateKind[] = ["translating", "retranslating", "cleanup", "verifying"];

/** "Translating 3/19 · batch 2 of 3": the episode being worked on, of how many, and the
 *  batch it's on. */
export function translatingLabel(files: Record<string, FileRow>): string {
  const all = Object.values(files);
  const finished = all.filter((f) => FINISHED.includes(f.state)).length;
  const at = Math.min(finished + 1, all.length);
  const active = all.find((f) => WORKING.includes(f.state));
  const batch =
    active && active.totalBatches > 0 ? ` · batch ${active.batch} of ${active.totalBatches}` : "";
  return `Translating ${at}/${all.length}${batch}`;
}

/** What a glossary operation is called while it runs. */
export function glossaryLabel(op: string, done: number, total: number): string {
  switch (op) {
    case "build":
      return total > 0 ? `Building glossary · ${done}/${total}` : "Building glossary";
    case "normalize":
      return "Tidying up";
    case "lookup":
      return "Looking up names";
    case "import":
      return "Importing from translated episodes";
    default:
      return "Working on the glossary";
  }
}

/** The last run's outcome, by episode: finished clean, finished with something to look at,
 *  not finished (stopped, or never started), or failed. Only real failures are failed. */
export function runTally(files: Record<string, FileRow>, results: readonly FileResult[]) {
  const t = tallyRun(files, [...results]);
  return {
    done: t.done.length,
    needsLook: t.look.length,
    notFinished: t.unfinished.length,
    failed: t.failed.length,
  };
}

/** "Chinese" for "zh", from the language table; the code itself until that loads. */
function languageName(languages: Language[] | undefined, code: string): string {
  return languages?.find((l) => l.code === code)?.name ?? code.toUpperCase();
}

/** One clickable part of the status bar: quiet until hovered, gold ring on focus. */
function Segment({
  onClick,
  title,
  className,
  children,
}: {
  onClick: () => void;
  title: string;
  className?: string;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      title={title}
      onClick={onClick}
      className={cn(
        "inline-flex min-w-0 shrink items-center gap-1 rounded-[13px] border border-transparent px-2 py-0.5 tabular-nums outline-none transition-colors hover:bg-[color:var(--color-bg-hover)] hover:text-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50",
        className,
      )}
    >
      {children}
      <CaretRight aria-hidden weight="fill" className="size-2 shrink-0 opacity-70" />
    </button>
  );
}

/**
 * The console line along the bottom. Every part is a way in: the folder (with its world type
 * and languages) goes to its episodes, a live run or glossary job to where it's running, the
 * last run's result to the episodes, the model to Models. Gold only while something is live.
 */
export function StatusBar() {
  const navigate = useNavigate();
  const workdir = useAppStore((s) => s.workdir);
  const sourceLang = useAppStore((s) => s.sourceLang);
  const targetLang = useAppStore((s) => s.targetLang);
  const worldType = useAppStore((s) => s.worldType);
  const fileCount = useAppStore((s) => s.fileCount);
  const lineCount = useAppStore((s) => s.dialogueLineCount);
  const modelLabel = useAppStore((s) => s.translationLabel);
  const usable = useAppStore((s) => s.hasUsableConnection);
  const translating = useTranslationRun((s) => s.running);
  const runFolder = useTranslationRun((s) => s.folder);
  const files = useTranslationRun((s) => s.files);
  const results = useTranslationRun((s) => s.results);
  const gBusy = useGlossaryRun((s) => s.busy);
  const gDone = useGlossaryRun((s) => s.done);
  const gTotal = useGlossaryRun((s) => s.total);
  const { data: appInfo } = useQuery({ queryKey: ["app-info"], queryFn: ipc.appInfo });
  const { data: languages } = useQuery({
    queryKey: ["languages"],
    queryFn: ipc.listLanguages,
    staleTime: Infinity,
  });

  // Engine exclusivity means at most one of these is live at a time.
  const live = translating
    ? { to: "/episodes", label: translatingLabel(files) }
    : gBusy
      ? { to: "/glossary", label: glossaryLabel(gBusy, gDone, gTotal) }
      : null;
  // The last run's result stays until the next run — for the folder it belongs to.
  const tally = !translating && results && runFolder === workdir ? runTally(files, results) : null;

  const go = (to: string) => void navigate({ to: to as never });
  const folder = workdir
    ? [
        folderTitle(workdir),
        worldType ? WORLD_LABEL[worldType] : null,
        `${languageName(languages, sourceLang)} → ${languageName(languages, targetLang)}`,
      ]
        .filter(Boolean)
        .join(" · ")
    : null;

  return (
    <footer className="col-start-2 flex h-8 min-w-0 items-center gap-1.5 overflow-hidden border-t [contain:inline-size] border-border bg-[color:var(--color-bg-deepest)] px-2 text-[11px] text-muted-foreground [box-shadow:inset_0_1px_0_rgba(225,166,54,.10)]">
      {folder ? (
        <>
          <Segment onClick={() => go("/episodes")} title={`${workdir} — open its episodes`}>
            <span className="truncate text-foreground/90">{folder}</span>
          </Segment>
          <span className="shrink-0 px-1 tabular-nums">
            {plural(fileCount, "file")} · {plural(lineCount, "line")}
          </span>
        </>
      ) : (
        <Segment onClick={() => go("/")} title="Open a folder of subtitles">
          No folder open
        </Segment>
      )}

      <span className="min-w-2 flex-1" />

      {live ? (
        <Segment
          onClick={() => go(live.to)}
          title="Show what's running"
          className="border-[color-mix(in_oklch,var(--color-gold)_45%,transparent)] text-[color:var(--color-gold)] hover:text-[color:var(--color-gold-hi)]"
        >
          <span
            aria-hidden
            className="size-1.5 shrink-0 rounded-full bg-current [animation:sig-ping_1.5s_ease-out_infinite]"
          />
          <span className="truncate">{live.label}</span>
        </Segment>
      ) : tally && tally.done + tally.needsLook + tally.notFinished + tally.failed > 0 ? (
        <Segment onClick={() => go("/episodes")} title="The last run's episodes" className="border-border">
          {tally.done > 0 ? (
            <span className="whitespace-nowrap text-[color:var(--color-jade)]">✓ {tally.done} done</span>
          ) : null}
          {tally.needsLook > 0 ? (
            <span className="whitespace-nowrap text-[color:var(--color-amber)]">
              {tally.done > 0 ? <span className="text-muted-foreground">· </span> : null}⚠{" "}
              {tally.needsLook} {tally.needsLook === 1 ? "needs" : "need"} a look
            </span>
          ) : null}
          {tally.notFinished > 0 ? (
            <span className="whitespace-nowrap text-foreground/80">
              {tally.done + tally.needsLook > 0 ? <span className="text-muted-foreground">· </span> : null}■{" "}
              {tally.notFinished} not finished
            </span>
          ) : null}
          {tally.failed > 0 ? (
            <span className="whitespace-nowrap text-[color:var(--color-coral)]">
              {tally.done + tally.needsLook + tally.notFinished > 0 ? (
                <span className="text-muted-foreground">· </span>
              ) : null}
              ✗ {tally.failed} failed
            </span>
          ) : null}
        </Segment>
      ) : null}

      <Segment
        onClick={() => go("/connections")}
        title={!modelLabel ? "Connect a model on the Models page" : "Your models"}
        className={cn(
          "border-border",
          (!modelLabel || !usable) &&
            "border-[color-mix(in_oklch,var(--color-amber)_45%,transparent)] text-[color:var(--color-amber)]",
        )}
      >
        {!modelLabel || !usable ? (
          <span aria-hidden className="size-1.5 shrink-0 rounded-full border border-current" />
        ) : null}
        <span className="truncate">
          {!modelLabel ? "No model" : !usable ? "Needs a key" : modelLabel}
        </span>
      </Segment>
      <span className="shrink-0 px-1 tabular-nums opacity-60">{appInfo?.version ?? "…"}</span>
    </footer>
  );
}
