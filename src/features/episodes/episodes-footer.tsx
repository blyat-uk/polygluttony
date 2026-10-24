import { Play, Stop } from "@phosphor-icons/react";
import { Link } from "@tanstack/react-router";
import { Button } from "@/components/ui/button";
import { LogPanel, LogToggleButton } from "@/components/log-drawer";
import type { LogLine } from "@/stores/translation-store";
import type { Blocked } from "./footer-copy";

export function EpisodesFooter({
  running,
  stopping,
  label,
  blocked,
  estimate,
  addToGlossary,
  logs,
  logsOpen,
  onToggleLogs,
  onStart,
  onStop,
}: {
  /** A run in this folder is going: the button becomes Stop. */
  running: boolean;
  stopping: boolean;
  /** "Translate 3 episodes · 2 replace existing translations". */
  label: string;
  blocked: Blocked;
  estimate: string | null;
  addToGlossary: { label: string; onClick: () => void } | null;
  logs: LogLine[];
  logsOpen: boolean;
  onToggleLogs: () => void;
  onStart: () => void;
  onStop: () => void;
}) {
  return (
    <>
      <div className="flex items-center gap-3 border-t border-border bg-[color:var(--popover)] px-5 py-3">
        {running ? (
          <Button variant="destructive" onClick={onStop} disabled={stopping}>
            <Stop weight="fill" /> {stopping ? "Stopping…" : "Stop"}
          </Button>
        ) : (
          <Button disabled={blocked !== null} onClick={onStart}>
            <Play weight="fill" /> {label}
          </Button>
        )}

        {running ? null : <FooterHint blocked={blocked} estimate={estimate} />}

        <span className="flex-1" />

        {addToGlossary && !running ? (
          <button
            type="button"
            onClick={addToGlossary.onClick}
            className="rounded-md text-[12px] text-primary underline-offset-2 outline-none hover:underline focus-visible:ring-[3px] focus-visible:ring-ring/50"
          >
            {addToGlossary.label}
          </button>
        ) : null}
        <LogToggleButton open={logsOpen} count={logs.length} onToggle={onToggleLogs} />
      </div>
      {logsOpen ? (
        <LogPanel
          lines={logs.map((entry) => ({
            at: entry.at,
            level: entry.level,
            message: entry.message,
            meta: [
              <span key="f" className="max-w-[160px] shrink-0 truncate text-muted-foreground">
                {entry.file ?? "run"}
              </span>,
            ],
          }))}
        />
      ) : null}
    </>
  );
}

function FooterHint({ blocked, estimate }: { blocked: Blocked; estimate: string | null }) {
  switch (blocked) {
    case "other_run":
      return (
        <span className="text-[11.5px] text-[color:var(--color-alert)]">
          Another folder is translating. You can start when it's done.
        </span>
      );
    case "glossary_busy":
      return (
        <span className="text-[11.5px] text-[color:var(--color-alert)]">
          A glossary job is running. You can translate when it's done.
        </span>
      );
    case "no_model":
      return (
        <span className="text-[11.5px] text-[color:var(--color-alert)]">
          No model is ready to translate.{" "}
          <Link to="/connections" className="text-primary underline-offset-2 hover:underline">
            Set one up in Models
          </Link>
          .
        </span>
      );
    case "same_language":
      return (
        <span className="text-[11.5px] text-[color:var(--color-alert)]">
          Source and target languages are the same.
        </span>
      );
    case "nothing_ticked":
      return <span className="text-[11.5px] text-muted-foreground">Tick the episodes to translate.</span>;
    case null:
      return estimate ? (
        <span className="text-[11.5px] text-muted-foreground tabular-nums">{estimate}</span>
      ) : null;
  }
}
