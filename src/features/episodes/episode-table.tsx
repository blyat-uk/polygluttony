import { Fragment, useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { toast } from "sonner";
import {
  ArrowClockwise,
  AppWindow,
  CaretDown,
  CaretRight,
  DotsThree,
  Eye,
  FolderOpen,
} from "@phosphor-icons/react";
import type { SourceFile } from "@/types/generated/SourceFile";
import { ipc } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { StateChip } from "@/components/state-chip";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { IssuePanel } from "@/features/translate/file-cells";
import type { EpisodeStatus } from "./episode-status";
import { leftoverNote } from "./episode-status";
import { collapseQuietRuns } from "./collapse-rows";
import { episodeName } from "./folder-labels";
import { toggleOne } from "./selection";

/** Open an episode's window (or focus it), optionally on one line. */
export function openEpisode(file: string, folder: string, line?: number) {
  ipc.openEpisodeWindow(file, { folder, line }).catch((err: unknown) => {
    toast.error(`Couldn't open ${episodeName(file)}`, {
      description: err instanceof Error ? err.message : String(err),
    });
  });
}

/** Show the episode's output in the file manager, or the source when there's none yet. */
function showInFolder(file: SourceFile) {
  revealItemInDir(file.output_path ?? file.path).catch((err: unknown) => {
    toast.error("Couldn't show it in the folder", {
      description: err instanceof Error ? err.message : String(err),
    });
  });
}

const DETAIL_TONE: Record<EpisodeStatus["detailTone"], string> = {
  muted: "text-muted-foreground",
  alert: "text-[color:var(--color-alert)]",
  danger: "text-[color:var(--color-danger)]",
};

export function EpisodeTable({
  folder,
  files,
  selected,
  onSelect,
  statusOf,
  watchable,
  canTranslate,
  onTranslateOne,
}: {
  folder: string;
  files: SourceFile[];
  selected: string[];
  onSelect: (next: string[]) => void;
  statusOf: (name: string) => EpisodeStatus;
  /** Files of the running run: they get an always-visible Watch button. */
  watchable: Set<string>;
  canTranslate: boolean;
  onTranslateOne: (name: string) => void;
}) {
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [showAll, setShowAll] = useState(false);
  const ticked = new Set(selected);
  const allTicked = files.length > 0 && files.every((f) => ticked.has(f.name));

  const toggleExpand = (name: string) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });

  const quiet = (f: SourceFile) =>
    !showAll &&
    !ticked.has(f.name) &&
    !watchable.has(f.name) &&
    !f.both_variants &&
    statusOf(f.name).kind === "done";
  const items = collapseQuietRuns(files, quiet, (f) => f.name);

  return (
    <div className="overflow-hidden rounded-xl border border-border">
      <table className="w-full table-fixed text-sm">
        <thead>
          <tr className="border-b border-border bg-[color:var(--color-raised)] text-[11px] tracking-wide text-muted-foreground uppercase">
            <th className="w-10 py-2 pl-4 text-left">
              <Checkbox
                aria-label="Select every episode"
                checked={allTicked}
                onCheckedChange={() => onSelect(allTicked ? [] : files.map((f) => f.name))}
              />
            </th>
            <th className="px-3 py-2 text-left font-medium">Episode</th>
            <th className="w-20 px-3 py-2 text-right font-medium">Lines</th>
            <th className="w-[42%] px-3 py-2 text-left font-medium">Status</th>
            <th className="w-36 py-2 pr-3" />
          </tr>
        </thead>
        <tbody>
          {items.map((item) => {
            if (item.type === "gap") {
              return (
                <tr key={item.key} className="border-b border-border">
                  <td colSpan={5} className="py-1 text-center">
                    <button
                      type="button"
                      onClick={() => setShowAll(true)}
                      className="rounded-md px-2 py-0.5 text-[11px] text-muted-foreground outline-none hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50"
                    >
                      · · · {item.count} more done · · ·
                    </button>
                  </td>
                </tr>
              );
            }
            const f = item.row;
            const status = statusOf(f.name);
            const isTicked = ticked.has(f.name);
            const name = episodeName(f.name);
            const expandable = status.issues.length > 0;
            const isOpen = expandable && expanded.has(f.name);
            const leftover = leftoverNote(f, name);
            return (
              <Fragment key={f.name}>
                <tr
                  onClick={() => onSelect(toggleOne(selected, f.name, files))}
                  className={cn(
                    "cursor-pointer border-b border-border last:border-0 hover:bg-[color:var(--color-bg-hover)]",
                    isTicked && "bg-[color-mix(in_oklch,var(--color-gold)_7%,transparent)]",
                  )}
                >
                  <td className="py-2 pl-4 align-middle">
                    <Checkbox
                      aria-label={`Select ${name}`}
                      checked={isTicked}
                      onClick={(e) => e.stopPropagation()}
                      onCheckedChange={() => onSelect(toggleOne(selected, f.name, files))}
                    />
                  </td>
                  <td className="min-w-0 px-3 py-2">
                    <span className="block truncate text-[13px]" title={f.name}>
                      {name}
                    </span>
                    {leftover ? (
                      <span className="block truncate text-[11px] text-[color:var(--color-alert)]">
                        {leftover}
                      </span>
                    ) : null}
                  </td>
                  <td className="px-3 py-2 text-right text-[12px] text-muted-foreground tabular-nums">
                    {f.dialogue_count.toLocaleString()}
                  </td>
                  <td className="min-w-0 px-3 py-2" onClick={(e) => e.stopPropagation()}>
                    <div className="flex min-w-0 items-center gap-2">
                      {expandable ? (
                        <button
                          type="button"
                          aria-expanded={isOpen}
                          aria-label={isOpen ? "Hide the issues" : "Show the issues"}
                          onClick={() => toggleExpand(f.name)}
                          className="inline-flex shrink-0 items-center gap-1 rounded-[13px] outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
                        >
                          <StateChip
                            label={status.label}
                            color={status.color}
                            live={status.live}
                            className="hover:border-[color:var(--color-alert)]/60"
                          />
                          {isOpen ? (
                            <CaretDown className="size-3 text-muted-foreground" />
                          ) : (
                            <CaretRight className="size-3 text-muted-foreground" />
                          )}
                        </button>
                      ) : (
                        <StateChip label={status.label} color={status.color} live={status.live} />
                      )}
                      {status.detail ? (
                        <span
                          className={cn(
                            "min-w-0 truncate text-[12px] tabular-nums",
                            DETAIL_TONE[status.detailTone],
                          )}
                          title={status.detail}
                        >
                          {status.detail}
                        </span>
                      ) : null}
                    </div>
                  </td>
                  <td className="py-2 pr-3" onClick={(e) => e.stopPropagation()}>
                    <div className="flex items-center justify-end gap-1">
                      {watchable.has(f.name) ? (
                        <Button variant="outline" size="xs" onClick={() => openEpisode(f.name, folder)}>
                          <Eye /> Watch
                        </Button>
                      ) : status.kind === "needs_look" ? (
                        <Button variant="outline" size="xs" onClick={() => openEpisode(f.name, folder)}>
                          Review
                        </Button>
                      ) : null}
                      <RowMenu
                        file={f}
                        name={name}
                        canTranslate={canTranslate}
                        onTranslate={() => onTranslateOne(f.name)}
                        onOpen={() => openEpisode(f.name, folder)}
                      />
                    </div>
                  </td>
                </tr>
                {isOpen ? (
                  <tr className="border-b border-border last:border-0">
                    <td colSpan={5} className="p-0">
                      <IssuePanel
                        issues={status.issues}
                        onLine={(line) => openEpisode(f.name, folder, line)}
                      />
                    </td>
                  </tr>
                ) : null}
              </Fragment>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function RowMenu({
  file,
  name,
  canTranslate,
  onTranslate,
  onOpen,
}: {
  file: SourceFile;
  name: string;
  canTranslate: boolean;
  onTranslate: () => void;
  onOpen: () => void;
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon-xs" aria-label={`More for ${name}`}>
          <DotsThree weight="bold" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-56">
        <DropdownMenuItem className="py-1.5 focus:bg-muted" onSelect={() => showInFolder(file)}>
          <FolderOpen />
          Show in folder
        </DropdownMenuItem>
        <DropdownMenuItem
          className="py-1.5 focus:bg-muted"
          disabled={!canTranslate}
          onSelect={onTranslate}
        >
          <ArrowClockwise />
          {file.translation === "none" ? "Translate this episode" : "Translate again"}
        </DropdownMenuItem>
        <DropdownMenuItem className="py-1.5 focus:bg-muted" onSelect={onOpen}>
          <AppWindow />
          Open in episode window
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
