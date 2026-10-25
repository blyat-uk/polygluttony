import { Play, Stop } from "@phosphor-icons/react";
import { Link } from "@tanstack/react-router";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { LogPanel, LogToggleButton } from "@/components/log-drawer";
import type { LogLine } from "@/stores/translation-store";
import type { Blocked } from "./footer-copy";

export function EpisodesFooter({
  running,
  stopping,
  label,
  blocked,
  estimate,
  glossaryTick,
  note = null,
  runningNote = null,
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
  /** "Build a glossary first": the run reads the ticked episodes for the glossary,
   *  then translates. Null when there's nothing for the glossary to read. */
  glossaryTick: {
    label: string;
    checked: boolean;
    onChange: (checked: boolean) => void;
    /** The button's label with the tick the other way. */
    otherLabel: string;
  } | null;
  /** A quiet word about the run, e.g. that it goes without the glossary. */
  note?: string | null;
  /** The same, while it runs: what Stop does now. */
  runningNote?: string | null;
  logs: Pick<LogLine, "at" | "level" | "message" | "file">[];
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
            <Play weight="fill" />
            {/* As wide as the label for the tick's other state, so the tick
                doesn't slide out from under the pointer when it's clicked. */}
            <span className="grid justify-items-center">
              <span className="col-start-1 row-start-1">{label}</span>
              {glossaryTick ? (
                <span aria-hidden className="invisible col-start-1 row-start-1">
                  {glossaryTick.otherLabel}
                </span>
              ) : null}
            </span>
          </Button>
        )}

        {glossaryTick && !running ? (
          <label className="flex shrink-0 cursor-pointer items-center gap-2 text-[12.5px] text-foreground select-none">
            <Checkbox
              checked={glossaryTick.checked}
              onCheckedChange={(v) => glossaryTick.onChange(v === true)}
            />
            {glossaryTick.label}
          </label>
        ) : null}
        {running ? null : <FooterHint blocked={blocked} estimate={estimate} />}
        {note && !running ? <span className="text-[11.5px] text-muted-foreground">{note}</span> : null}
        {runningNote && running ? (
          <span className="text-[11.5px] text-muted-foreground">{runningNote}</span>
        ) : null}

        <span className="flex-1" />

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
