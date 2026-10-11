import {
  BookOpen,
  Folder,
  Lightning,
  NotePencil,
  Play,
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
  needsFolder?: boolean;
}

const ITEMS: RailItem[] = [
  { to: "/project", label: "Project", icon: Folder, group: "workflow" },
  { to: "/glossary", label: "Glossary", icon: BookOpen, group: "workflow", needsFolder: true },
  { to: "/translate", label: "Translate", icon: Play, group: "workflow", needsFolder: true },
  { to: "/connections", label: "Connections", icon: Lightning, group: "setup" },
  { to: "/prompts", label: "Prompts", icon: NotePencil, group: "setup" },
  { to: "/help", label: "Help", icon: Question, group: "setup" },
];

export function NavRail() {
  const workdir = useAppStore((s) => s.workdir);
  const hasUsableConnection = useAppStore((s) => s.hasUsableConnection);
  const hasUntranslated = useAppStore((s) => s.hasUntranslated);
  const glossaryTerms = useAppStore((s) => s.glossaryTerms);
  const pathname = useRouterState({ select: (s) => s.location.pathname });

  // Returns a gating hint when the destination is disabled, else null.
  const gateHint = (item: RailItem): string | null => {
    if (!item.needsFolder) return null;
    if (!workdir) return "Open a folder first";
    if (item.to === "/translate") {
      if (!hasUsableConnection) return "Connect an AI model";
      if (!hasUntranslated) return "No untranslated files in this folder";
    }
    return null;
  };

  const workflow = ITEMS.filter((i) => i.group === "workflow");
  const setup = ITEMS.filter((i) => i.group === "setup");

  const render = (item: RailItem) => {
    const hint = gateHint(item);
    const disabled = hint !== null;
    // Project is dual-role: it opens Welcome ("/") until a folder is picked, the
    // Project view after — and highlights on both so they read as one place.
    const isProject = item.to === "/project";
    const to = isProject && !workdir ? "/" : item.to;
    const active = isProject
      ? pathname === "/" || pathname.startsWith("/project")
      : pathname.startsWith(item.to);
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
        <Icon weight={active ? "fill" : "regular"} className={cn("size-5", active && "[filter:drop-shadow(0_0_7px_color-mix(in_oklch,var(--color-gold)_75%,transparent))]")} />
        {item.label}
        {item.to === "/connections" ? (
          <span
            className={
              hasUsableConnection
                ? "text-[color:var(--color-success)]"
                : "text-[color:var(--color-alert)]"
            }
          >
            {hasUsableConnection ? "✓" : "⚠"}
          </span>
        ) : null}
        {item.to === "/glossary" && glossaryTerms ? (
          <span className="text-[9px] tabular-nums text-muted-foreground">{glossaryTerms}</span>
        ) : null}
      </div>
    );
    if (disabled) {
      return (
        <Tooltip key={item.to}>
          <TooltipTrigger asChild>
            <div>{body}</div>
          </TooltipTrigger>
          <TooltipContent side="right">{hint}</TooltipContent>
        </Tooltip>
      );
    }
    return (
      <Link key={item.to} to={to as never}>
        {body}
      </Link>
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
