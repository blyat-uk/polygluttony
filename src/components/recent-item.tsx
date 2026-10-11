import { Folder } from "@phosphor-icons/react";
import type { RecentFolder } from "@/types/generated/RecentFolder";
import { formatRelativeTime } from "@/lib/relative-time";
import { folderTitle } from "@/lib/folder-title";

export function RecentItem({
  recent,
  onOpen,
  onRemove,
}: {
  recent: RecentFolder;
  onOpen: () => void;
  onRemove: () => void;
}) {
  // Many recents are a show's "translate" or "Season N" folder; name them after the show.
  const name = folderTitle(recent.path);
  return (
    <div className="group flex items-center gap-3 rounded-md border border-border bg-[color:var(--card)] px-3 py-2 hover:bg-[color:var(--color-bg-hover)]">
      <button
        type="button"
        onClick={onOpen}
        className="flex min-w-0 flex-1 items-center gap-3 text-left"
      >
        <Folder weight="fill" className="size-5 shrink-0 text-primary" />
        <span className="min-w-0">
          <span className="block truncate text-[13px] font-medium text-foreground">{name}</span>
          <span className="block truncate font-mono text-[11px] text-muted-foreground">
            {recent.path}
          </span>
        </span>
      </button>
      <span className="shrink-0 text-[11px] tabular-nums text-muted-foreground">
        {recent.file_count} files · {formatRelativeTime(Number(recent.last_opened))}
      </span>
      <button
        type="button"
        onClick={onRemove}
        className="shrink-0 text-[11px] text-muted-foreground opacity-0 transition group-hover:opacity-100 hover:text-[color:var(--color-danger)]"
      >
        Remove
      </button>
    </div>
  );
}
