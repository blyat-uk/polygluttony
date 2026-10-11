import { useEffect, useMemo, type ReactNode } from "react";
import type { EpisodeLine } from "@/types/generated/EpisodeLine";
import type { VerifyIssue } from "@/types/generated/VerifyIssue";
import { EpisodeHeader } from "./episode-header";
import { issuesByLine, liveIndices } from "./format";
import { LineTable } from "./line-table";
import { useEpisode } from "./use-episode";

const NO_LINES: EpisodeLine[] = [];
const NO_ISSUES: VerifyIssue[] = [];

/** A quiet message in place of the table. */
function Notice({ children, tone = "muted" }: { children: ReactNode; tone?: "muted" | "faint" }) {
  return (
    <div className="flex flex-1 items-center justify-center px-8 text-center">
      <div
        className={
          tone === "faint"
            ? "text-[12px] text-muted-foreground/60"
            : "max-w-md text-[12.5px] text-muted-foreground"
        }
      >
        {children}
      </div>
    </div>
  );
}

/** The live window for one episode: header, then every line with its
 *  translation as the run produces it. */
export function EpisodeWindow({ file }: { file: string }) {
  const { view, error, landed } = useEpisode(file);

  useEffect(() => {
    document.title = `${file} — Polygluttony`;
  }, [file]);

  const snapshot = view?.kind === "live" ? view : null;
  const lines = snapshot?.lines ?? NO_LINES;
  const live = useMemo(() => liveIndices(lines), [lines]);
  const liveIds = useMemo(() => live.map((i) => lines[i].id), [live, lines]);
  const issues = useMemo(() => issuesByLine(snapshot?.issues ?? NO_ISSUES), [snapshot?.issues]);

  let body: ReactNode;
  if (view === null) {
    body = error ? (
      <Notice>
        <p className="text-foreground/85">Couldn’t load this episode.</p>
        <p className="mt-1 break-words text-[11.5px]">{error}</p>
      </Notice>
    ) : (
      <Notice tone="faint">Loading lines…</Notice>
    );
  } else if (view.kind === "not_in_run") {
    body = <Notice>This episode isn’t part of the current run.</Notice>;
  } else if (view.kind === "unreadable") {
    body = (
      <Notice>
        <p className="text-foreground/85">Couldn’t read this episode.</p>
        <p className="mt-1 break-words text-[11.5px]">{view.message}</p>
      </Notice>
    );
  } else if (lines.length === 0) {
    body = <Notice>This episode has no dialogue lines.</Notice>;
  } else {
    body = <LineTable lines={lines} live={live} issues={issues} landed={landed} />;
  }

  return (
    <div className="flex h-screen flex-col bg-background text-foreground">
      <EpisodeHeader
        file={file}
        snapshot={snapshot}
        liveIds={liveIds}
        flaggedLines={issues.size}
        refreshError={view !== null ? error : null}
      />
      <main className="relative flex min-h-0 flex-1 flex-col">{body}</main>
    </div>
  );
}
