import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { Folder } from "@phosphor-icons/react";
import { StateChip } from "@/components/state-chip";
import { StatusChip } from "@/components/status-chip";
import { Separator } from "@/components/ui/separator";
import { ipc } from "@/lib/ipc";
import { plural } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";
import { useGlossaryRun } from "@/stores/glossary-store";
import { useTranslationRun } from "@/stores/translation-store";

export function StatusBar() {
  const workdir = useAppStore((s) => s.workdir);
  const sourceLang = useAppStore((s) => s.sourceLang);
  const targetLang = useAppStore((s) => s.targetLang);
  const worldType = useAppStore((s) => s.worldType);
  const fileCount = useAppStore((s) => s.fileCount);
  const lineCount = useAppStore((s) => s.dialogueLineCount);
  const translationLabel = useAppStore((s) => s.translationLabel);
  const translating = useTranslationRun((s) => s.running);
  const navigate = useNavigate();
  const gBusy = useGlossaryRun((s) => s.busy);
  const gDone = useGlossaryRun((s) => s.done);
  const gTotal = useGlossaryRun((s) => s.total);
  const tFiles = useTranslationRun((s) => s.files);
  const { data: appInfo } = useQuery({ queryKey: ["app-info"], queryFn: ipc.appInfo });

  const translatingCounts = (() => {
    const all = Object.values(tFiles);
    const done = all.filter((f) => ["done", "warning", "failed"].includes(f.state)).length;
    return `${done}/${plural(all.length, "file")}`;
  })();

  const activeBatch = (() => {
    const a = Object.values(tFiles).find((f) =>
      ["translating", "retranslating", "cleanup", "verifying"].includes(f.state));
    return a && a.totalBatches > 0 ? ` · batch ${a.batch}/${a.totalBatches}` : "";
  })();

  // Engine exclusivity means at most one of these is active at a time.
  const activity = gBusy
    ? {
        to: "/glossary" as const,
        label:
          gBusy === "build"
            ? `Building glossary · ${gDone}/${gTotal}`
            : gBusy === "import"
              ? `Importing reference terms · ${gDone}/${gTotal}`
              : "Normalizing glossary…",
      }
    : translating
      ? { to: "/translate" as const, label: `Translating · ${translatingCounts}${activeBatch}` }
      : null;

  return (
    <footer className="col-start-2 flex h-8 items-center gap-3 border-t border-border bg-[color:var(--color-bg-deepest)] px-3 text-[11px] text-muted-foreground [box-shadow:inset_0_1px_0_rgba(225,166,54,.10)]">
      <span className="flex min-w-0 items-center gap-1.5">
        <Folder className="size-3.5 shrink-0" />
        <span className="truncate">{workdir ?? "No folder selected"}</span>
      </span>
      <Separator orientation="vertical" className="h-4" />
      <span className="shrink-0 tabular-nums">
        {workdir ? `${plural(fileCount, "file")} · ${plural(lineCount, "line")}` : "— files · — lines"}
      </span>
      <span className="flex-1" />
      {activity ? (
        <button type="button" className="cursor-pointer" onClick={() => void navigate({ to: activity.to })}>
          <StatusChip variant="alert">⏳ {activity.label}</StatusChip>
        </button>
      ) : null}
      {worldType ? <StatusChip variant="muted">{worldType}</StatusChip> : null}
      <span className="shrink-0">
        {sourceLang}→{targetLang}
      </span>
      <StatusChip variant="accent">{translationLabel ?? "No model"}</StatusChip>
      <StateChip state={translating ? "translating" : gBusy ? "translating" : "idle"} label={gBusy && !translating ? "Busy" : undefined} />
      <span className="shrink-0 opacity-60">core {appInfo?.version ?? "…"}</span>
    </footer>
  );
}
