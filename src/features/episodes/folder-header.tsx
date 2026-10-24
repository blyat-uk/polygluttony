import { useNavigate } from "@tanstack/react-router";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import { ArrowsClockwise, CaretDown, Folder, FolderOpen, X } from "@phosphor-icons/react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { folderTitle } from "@/lib/folder-title";
import { useAppStore } from "@/stores/app-store";
import { useOpenFolder } from "@/features/project/use-project";
import { useRecents } from "@/features/welcome/use-recents";

const ITEM = "py-1.5 focus:bg-muted";

/** Show name, the path in mono underneath, and Rescan / Switch folder ▾. */
export function FolderHeader({
  folder,
  rescanning,
  onRescan,
}: {
  folder: string;
  rescanning: boolean;
  onRescan: () => void;
}) {
  const navigate = useNavigate();
  const open = useOpenFolder();
  const { data: recents } = useRecents();
  const others = (recents ?? []).filter((r) => r.path !== folder).slice(0, 6);

  const openPath = (path: string) =>
    open.mutate(path, {
      onSuccess: (view) => {
        if (view.files.length === 0) {
          toast.error(`No subtitle files in ${folderTitle(path)}.`, { description: path });
        }
      },
    });

  const openAnother = async () => {
    const path = await openDialog({ directory: true, multiple: false });
    if (typeof path === "string") openPath(path);
  };

  const closeFolder = () => {
    useAppStore.getState().clearProject();
    void navigate({ to: "/" });
  };

  return (
    <div className="flex items-start justify-between gap-4 border-b border-border px-5 py-3">
      <div className="flex min-w-0 flex-col gap-0.5">
        <h1 className="truncate text-[1.125rem] font-semibold tracking-[-0.02em] text-foreground">
          {folderTitle(folder)}
        </h1>
        <p className="truncate font-mono text-[11.5px] text-muted-foreground" title={folder}>
          {folder}
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <Button variant="outline" size="sm" onClick={onRescan} disabled={rescanning}>
          <ArrowsClockwise className={rescanning ? "animate-spin" : undefined} />
          Rescan
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="outline" size="sm" disabled={open.isPending}>
              Switch folder
              <CaretDown className="size-3" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-80">
            {others.length > 0 ? (
              <>
                <DropdownMenuLabel className="text-[11px] text-muted-foreground">
                  Recent folders
                </DropdownMenuLabel>
                {others.map((r) => (
                  <DropdownMenuItem key={r.path} className={ITEM} onSelect={() => openPath(r.path)}>
                    <Folder weight="fill" className="text-primary" />
                    <span className="min-w-0">
                      <span className="block truncate text-[13px]">{folderTitle(r.path)}</span>
                      <span className="block truncate font-mono text-[10.5px] text-muted-foreground!">
                        {r.path}
                      </span>
                    </span>
                  </DropdownMenuItem>
                ))}
                <DropdownMenuSeparator />
              </>
            ) : null}
            <DropdownMenuItem className={ITEM} onSelect={() => void openAnother()}>
              <FolderOpen />
              Open another…
            </DropdownMenuItem>
            <DropdownMenuItem className={ITEM} onSelect={closeFolder}>
              <X />
              Close folder
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </div>
  );
}
