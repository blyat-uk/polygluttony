import { useEffect } from "react";
import { useNavigate } from "@tanstack/react-router";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { Translate } from "@phosphor-icons/react";
import { Button } from "@/components/ui/button";
import { RecentItem } from "@/components/recent-item";
import { useAppStore } from "@/stores/app-store";
import { useRecents, useRecentMutations } from "./use-recents";
import { useOpenFolder } from "@/features/project/use-project";

export function WelcomePage() {
  const navigate = useNavigate();
  const hasConnection = useAppStore((s) => s.hasUsableConnection);
  const { data: recents } = useRecents();
  const recentM = useRecentMutations();
  const open = useOpenFolder();

  const pick = async () => {
    const path = await openDialog({ directory: true, multiple: false });
    if (typeof path === "string") open.mutate(path);
  };

  // Folder drag-and-drop onto the window → open the first dropped path.
  useEffect(() => {
    const unlisten = getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === "drop" && event.payload.paths.length > 0) {
        open.mutate(event.payload.paths[0]);
      }
    });
    return () => {
      void unlisten.then((u) => u());
    };
  }, [open.mutate]);

  const emptyResult = open.data?.files.length === 0;

  return (
    <div className="flex h-full flex-col items-center justify-center gap-6 p-10">
      <div className="flex flex-col items-center gap-1 text-center">
        <Translate weight="fill" className="size-10 text-primary" />
        <h1 className="text-[22px] font-semibold text-foreground">Polygluttony</h1>
        <p className="text-sm text-muted-foreground">
          LLM-powered subtitle translation for donghua &amp; anime.
        </p>
      </div>

      {!hasConnection ? (
        <div className="w-full max-w-xl space-y-2">
          <Step n={1} title="Connect an AI provider" hint="Anthropic, OpenAI, Google, or a model on your computer (Ollama, LM Studio).">
            <Button variant="secondary" onClick={() => navigate({ to: "/connections" })}>
              Connect
            </Button>
          </Step>
          <Step n={2} title="Open a folder of subtitles" hint="Point at a folder of .ass files to begin.">
            <Button onClick={pick}>Open folder</Button>
          </Step>
        </div>
      ) : (
        <div className="w-full max-w-xl space-y-3">
          <div className="flex items-center justify-between">
            <h2 className="text-[13px] font-semibold text-muted-foreground">Recent folders</h2>
            <Button size="sm" onClick={pick}>
              Open folder
            </Button>
          </div>
          {recents && recents.length > 0 ? (
            <div className="space-y-1.5">
              {recents.map((r) => (
                <RecentItem
                  key={r.path}
                  recent={r}
                  onOpen={() => open.mutate(r.path)}
                  onRemove={() => recentM.remove.mutate(r.path)}
                />
              ))}
              <button
                type="button"
                onClick={() => {
                  if (confirm("Clear all recent folders?")) recentM.clear.mutate();
                }}
                className="text-[11px] text-muted-foreground hover:text-foreground"
              >
                Clear all
              </button>
            </div>
          ) : (
            <p className="text-center text-[12px] text-muted-foreground">No recent folders yet.</p>
          )}
          <p className="text-center text-[11px] text-muted-foreground">
            ↓ or drag a folder onto the window
          </p>
        </div>
      )}
      {emptyResult ? (
        <p className="text-center text-[12px] text-[color:var(--color-alert)]">
          No subtitle files found here.
        </p>
      ) : null}
    </div>
  );
}

function Step({
  n,
  title,
  hint,
  children,
}: {
  n: number;
  title: string;
  hint: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-center gap-3 rounded-md border border-border bg-[color:var(--card)] px-4 py-3">
      <span className="flex size-6 shrink-0 items-center justify-center rounded-full bg-[color:var(--popover)] text-[12px] text-primary">
        {n}
      </span>
      <div className="min-w-0 flex-1">
        <div className="text-[13px] font-medium text-foreground">{title}</div>
        <div className="text-[11px] text-muted-foreground">{hint}</div>
      </div>
      {children}
    </div>
  );
}
