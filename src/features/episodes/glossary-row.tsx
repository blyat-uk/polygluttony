import type { ReactNode } from "react";
import { useNavigate } from "@tanstack/react-router";
import { BookOpen } from "@phosphor-icons/react";
import type { GlossaryLoad } from "@/types/generated/GlossaryLoad";
import type { ProjectView } from "@/types/generated/ProjectView";
import { Button } from "@/components/ui/button";
import { cn, plural } from "@/lib/utils";
import { episodeName } from "./folder-labels";
import { untracked } from "./footer-copy";

/** The glossary step of a run going in this folder. */
export interface GlossaryBuilding {
  kind: "first" | "scan";
  /** Terms found so far. */
  found: number;
  /** Episodes the step reads. */
  files: number;
}

/**
 * The folder's glossary in one row, above the episodes: whether there is one,
 * which episodes it has read, and a way in. Folders whose language has no
 * glossary don't get one.
 */
export function GlossaryRow({
  view,
  glossary,
  unread,
  building,
}: {
  view: ProjectView;
  glossary: GlossaryLoad | undefined;
  /** Ticked episodes the glossary hasn't read. */
  unread: string[];
  building: GlossaryBuilding | null;
}) {
  const navigate = useNavigate();
  if (!view.supports_glossary) return null;
  const open = (label: string) => (
    <Button variant="outline" size="xs" onClick={() => void navigate({ to: "/glossary" })}>
      {label}
    </Button>
  );

  if (building) {
    const what =
      building.kind === "first"
        ? `being built · ${plural(building.found, "term")} so far`
        : `${termsOf(view)} · adding ${plural(building.files, "episode")}`;
    return (
      <Row title={what} action={open("Open")}>
        Translation starts when it’s done.
      </Row>
    );
  }

  if (glossary?.status === "unreadable") {
    return (
      <Row
        tone="bad"
        title={
          <span className="text-[color:var(--color-danger)]">
            {glossary.line != null ? `has a mistake on line ${glossary.line}` : "can’t be read"}
          </span>
        }
        action={open("Fix it")}
      >
        Translations won’t use it until it’s fixed.
      </Row>
    );
  }

  if (glossary?.status !== "ok") {
    return (
      <Row title="none yet" action={open("Build it on its own…")}>
        Keeps names and terms the same in every episode: 张伟 is always “Zhang Wei”, 灵气 is
        always “spiritual energy”.
      </Row>
    );
  }

  const title = `${termsOf(view)}${coverage(view, glossary)}`;
  if (unread.length > 0) {
    const who =
      unread.length === 1
        ? `${episodeName(unread[0])} isn’t in it yet`
        : `${unread.length} ticked episodes aren’t in it yet`;
    return (
      <Row tone="look" title={title} action={open("Open")}>
        <span className="text-[color:var(--color-alert)]">{who}</span>, so new names there may
        not match the rest.
      </Row>
    );
  }
  return (
    <Row title={title} action={open("Open")}>
      Used for every translation in this folder.
    </Row>
  );
}

const termsOf = (view: ProjectView) => plural(view.glossary_terms ?? 0, "term");

/** " · covers 3 of 4 episodes"; nothing for a glossary with terms from episodes
 *  it has no record of (made before PG kept track, or imported). */
function coverage(view: ProjectView, glossary: Extract<GlossaryLoad, { status: "ok" }>): string {
  if (untracked(glossary)) return "";
  const scanned = new Set(glossary.meta.scanned_files);
  const total = view.files.length;
  const read = view.files.filter((f) => scanned.has(f.name)).length;
  return read === total
    ? ` · covers ${total === 1 ? "the episode" : `all ${total} episodes`}`
    : ` · covers ${read} of ${plural(total, "episode")}`;
}

function Row({
  title,
  tone = "neutral",
  action,
  children,
}: {
  title: ReactNode;
  tone?: "neutral" | "look" | "bad";
  action: ReactNode;
  children: ReactNode;
}) {
  return (
    <div
      className={cn(
        "flex items-center gap-3.5 rounded-xl border bg-[color:var(--card)] px-4 py-3",
        tone === "look"
          ? "border-[color:var(--color-alert)]/35"
          : tone === "bad"
            ? "border-[color:var(--color-danger)]/35"
            : "border-border",
      )}
    >
      <span
        className={cn(
          "grid size-8 shrink-0 place-items-center rounded-lg bg-[color:var(--color-raised)]",
          tone === "bad" ? "text-[color:var(--color-danger)]" : "text-primary",
        )}
      >
        <BookOpen weight="duotone" className="size-[18px]" />
      </span>
      <div className="min-w-0 flex-1">
        <div className="truncate text-[13px] font-medium text-foreground">
          Glossary <span className="font-normal text-muted-foreground">· {title}</span>
        </div>
        <div className="text-[12px] text-muted-foreground">{children}</div>
      </div>
      <span className="shrink-0">{action}</span>
    </div>
  );
}
