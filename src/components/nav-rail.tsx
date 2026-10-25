import {
  BookOpen,
  FilmStrip,
  Lightning,
  NotePencil,
  Question,
  type Icon,
} from "@phosphor-icons/react";
import { Link, useRouterState } from "@tanstack/react-router";
import { useAppStore } from "@/stores/app-store";
import { cn } from "@/lib/utils";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Seal } from "@/components/seal";

interface RailItem {
  to: string;
  label: string;
  icon: Icon;
  group: "workflow" | "setup";
  /** Ctrl/Cmd + this digit goes here (the key handler lives in the app frame). */
  key: number;
  needsFolder?: boolean;
}

const ITEMS: RailItem[] = [
  { to: "/episodes", label: "Episodes", icon: FilmStrip, group: "workflow", key: 1 },
  { to: "/glossary", label: "Glossary", icon: BookOpen, group: "workflow", key: 2, needsFolder: true },
  { to: "/connections", label: "Models", icon: Lightning, group: "setup", key: 3 },
  { to: "/prompts", label: "Prompts", icon: NotePencil, group: "setup", key: 4 },
  { to: "/help", label: "Help", icon: Question, group: "setup", key: 5 },
];

const IS_MAC = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.userAgent);

/** "Ctrl+1", or "⌘1" on macOS. */
const shortcutLabel = (digit: number) => (IS_MAC ? `⌘${digit}` : `Ctrl+${digit}`);

export function NavRail() {
  const workdir = useAppStore((s) => s.workdir);
  const hasUsableConnection = useAppStore((s) => s.hasUsableConnection);
  const glossaryTerms = useAppStore((s) => s.glossaryTerms);
  const pathname = useRouterState({ select: (s) => s.location.pathname });

  const workflow = ITEMS.filter((i) => i.group === "workflow");
  const setup = ITEMS.filter((i) => i.group === "setup");

  const render = (item: RailItem) => {
    const hint = item.needsFolder && !workdir ? "Open a folder first" : null;
    const disabled = hint !== null;
    // Episodes is dual-role: it opens Welcome ("/") until a folder is open, the
    // episode list after, and highlights on both so they read as one place.
    const isEpisodes = item.to === "/episodes";
    const to = isEpisodes && !workdir ? "/" : item.to;
    const active = isEpisodes
      ? pathname === "/" || pathname.startsWith("/episodes")
      : pathname.startsWith(item.to);
    const warn = item.to === "/connections" && !hasUsableConnection;
    const Icon = item.icon;
    const body = (
      <div
        className={cn(
          "flex w-16 flex-col items-center gap-1 rounded-md py-2 text-[10px]",
          active && "bg-[color:var(--popover)] text-primary",
          disabled
            ? "cursor-not-allowed text-muted-foreground/50"
            : "hover:bg-[color:var(--color-bg-hover)]",
        )}
      >
        <Icon
          weight={active ? "fill" : "regular"}
          className={cn(
            "size-5",
            active &&
              "[filter:drop-shadow(0_0_7px_color-mix(in_oklch,var(--color-gold)_75%,transparent))]",
          )}
        />
        {item.label}
        {warn ? (
          <span className="text-[color:var(--color-alert)]" aria-label="No model is ready">
            ⚠
          </span>
        ) : null}
        {item.to === "/glossary" && glossaryTerms ? (
          <span className="text-[9px] tabular-nums text-muted-foreground">{glossaryTerms}</span>
        ) : null}
      </div>
    );
    const tip = (
      <TooltipContent side="right" className="flex items-center gap-2">
        <span>{hint ?? (warn ? "Models · no model is ready yet" : item.label)}</span>
        <kbd className="rounded border border-border px-1 font-mono text-[10px] opacity-75">
          {shortcutLabel(item.key)}
        </kbd>
      </TooltipContent>
    );
    return (
      <Tooltip key={item.to}>
        <TooltipTrigger asChild>
          {disabled ? (
            <div aria-disabled="true">{body}</div>
          ) : (
            <Link to={to as never} aria-label={item.label}>
              {body}
            </Link>
          )}
        </TooltipTrigger>
        {tip}
      </Tooltip>
    );
  };

  return (
    <nav className="flex h-full w-20 flex-col items-center gap-1 border-r border-border bg-[color:var(--sidebar)] py-3">
      <div className="mb-2 grid place-items-center text-primary [filter:drop-shadow(0_0_8px_color-mix(in_oklch,var(--color-gold)_60%,transparent))]">
        <Seal className="size-7" />
      </div>
      {workflow.map(render)}
      <div className="flex-1" />
      {setup.map(render)}
    </nav>
  );
}
