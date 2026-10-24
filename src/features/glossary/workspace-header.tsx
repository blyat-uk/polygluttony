import { useEffect, useState } from "react";
import { Link } from "@tanstack/react-router";
import {
  ArrowCounterClockwise,
  ArrowsClockwise,
  Books,
  CaretDown,
  DotsThree,
  DownloadSimple,
  NotePencil,
  UploadSimple,
} from "@phosphor-icons/react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { plural } from "@/lib/utils";
import { useGlossaryDoc, retrySave } from "./doc-store";
import { useDeferredMenu } from "./use-deferred-menu";

/** "● Saved" / "Saving…" / coral "Not saved · Retry" beside the title. */
function SaveMark({ folder }: { folder: string }) {
  const status = useGlossaryDoc((s) => (s.folder === folder ? s.status : "saved"));
  const error = useGlossaryDoc((s) => s.error);
  // Saves take milliseconds; only say "Saving…" when one takes a while.
  const [slow, setSlow] = useState(false);
  useEffect(() => {
    if (status !== "saving") {
      setSlow(false);
      return;
    }
    const t = setTimeout(() => setSlow(true), 300);
    return () => clearTimeout(t);
  }, [status]);

  if (status === "error") {
    return (
      <span
        className="inline-flex items-center gap-1.5 text-[11.5px] font-medium text-[color:var(--color-danger)]"
        title={error ?? undefined}
      >
        <span className="size-1.5 rounded-full bg-[color:var(--color-danger)]" />
        Not saved ·
        <button
          type="button"
          className="underline underline-offset-2 hover:text-[color:var(--color-ink-emphasis)]"
          onClick={() => retrySave(folder)}
        >
          Retry
        </button>
      </span>
    );
  }
  if (status === "saving" && slow) {
    return (
      <span className="inline-flex items-center gap-1.5 text-[11.5px] font-medium text-muted-foreground">
        <span className="size-1.5 animate-pulse rounded-full bg-muted-foreground" />
        Saving…
      </span>
    );
  }
  return (
    <span className="inline-flex items-center gap-1.5 text-[11.5px] font-medium text-[color:var(--color-success)]">
      <span className="size-1.5 rounded-full bg-[color:var(--color-success)]" />
      Saved
    </span>
  );
}

export interface HeaderActions {
  onTidy: () => void;
  /** Absent when no model can search the web. */
  onLookup?: () => void;
  onAddEpisodes: () => void;
  onImportGlossary: () => void;
  onImportTranslated: () => void;
  onExport: () => void;
  onOpenEditor: () => void;
  onRestore: () => void;
  onRebuild: () => void;
}

export function WorkspaceHeader({
  folder,
  count,
  subtitle,
  subtitleHint,
  showNext,
  searchModel,
  canRunModel,
  busy,
  actions,
}: {
  folder: string;
  count: number;
  subtitle: string;
  /** Shown on hover over the subtitle, e.g. "Built as Modern". */
  subtitleHint?: string;
  /** The quiet "Next: Translate episodes →" after the first build. */
  showNext: boolean;
  searchModel: string | null;
  /** A model is set up for Tidy up. */
  canRunModel: boolean;
  busy: boolean;
  actions: HeaderActions;
}) {
  const improve = useDeferredMenu();
  const more = useDeferredMenu();
  return (
    <div className="flex items-start justify-between gap-4 border-b border-border px-5 py-3">
      <div className="flex min-w-0 flex-col gap-0.5">
        <h1 className="flex items-center gap-3 text-[1.125rem] font-semibold tracking-[-0.02em] text-foreground">
          <span>Glossary · {plural(count, "term")}</span>
          <SaveMark folder={folder} />
        </h1>
        <p className="truncate text-[12.5px] text-muted-foreground" title={subtitleHint}>
          {subtitle}
          {showNext ? (
            <>
              {" · "}
              <Link to={"/episodes" as never} className="text-primary hover:underline">
                Next: Translate episodes →
              </Link>
            </>
          ) : null}
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="outline" size="sm" disabled={busy}>
              Improve <CaretDown className="size-3" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-60" onCloseAutoFocus={improve.onCloseAutoFocus}>
            <DropdownMenuItem onSelect={improve.pick(actions.onTidy)} disabled={!canRunModel}>
              Tidy up…
            </DropdownMenuItem>
            {actions.onLookup ? (
              <DropdownMenuItem onSelect={improve.pick(actions.onLookup)}>
                Look up names online…
              </DropdownMenuItem>
            ) : null}
            {searchModel || !canRunModel ? <DropdownMenuSeparator /> : null}
            {!canRunModel ? (
              <DropdownMenuLabel className="py-1.5 text-[11px]">Connect a model first, in Models.</DropdownMenuLabel>
            ) : searchModel ? (
              <DropdownMenuLabel className="py-1.5 text-[11px]">Uses {searchModel}</DropdownMenuLabel>
            ) : null}
          </DropdownMenuContent>
        </DropdownMenu>
        <Button variant="outline" size="sm" disabled={busy} onClick={actions.onAddEpisodes}>
          Add new episodes…
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="outline" size="icon-sm" aria-label="More glossary actions">
              <DotsThree weight="bold" className="size-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-64" onCloseAutoFocus={more.onCloseAutoFocus}>
            <DropdownMenuItem onSelect={more.pick(actions.onImportGlossary)} disabled={busy}>
              <UploadSimple /> Import a glossary…
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={more.pick(actions.onImportTranslated)} disabled={busy}>
              <Books /> Import from translated episodes…
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={more.pick(actions.onExport)}>
              <DownloadSimple /> Export…
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={more.pick(actions.onOpenEditor)}>
              <NotePencil /> Open in your text editor
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={more.pick(actions.onRestore)} disabled={busy}>
              <ArrowCounterClockwise /> Restore an earlier version…
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={more.pick(actions.onRebuild)} disabled={busy}>
              <ArrowsClockwise /> Rebuild from scratch…
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </div>
  );
}
