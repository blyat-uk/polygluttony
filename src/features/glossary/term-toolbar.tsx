import { CaretDown, MagnifyingGlass, X } from "@phosphor-icons/react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { SHOW_FILTERS, SHOW_LABEL, type ShowFilter } from "./filters";
import { CATEGORIES, CATEGORY_HINT, CATEGORY_LABEL, type Category } from "./model";
import { shortcut } from "./keyboard";

export function TermToolbar({
  searchRef,
  search,
  onSearch,
  searchPlaceholder,
  show,
  onShow,
  showCounts,
  category,
  onCategory,
  categoryCounts,
  shown,
  total,
}: {
  searchRef: React.RefObject<HTMLInputElement | null>;
  search: string;
  onSearch: (s: string) => void;
  searchPlaceholder: string;
  show: ShowFilter;
  onShow: (s: ShowFilter) => void;
  showCounts: Record<ShowFilter, number>;
  category: Category | null;
  onCategory: (c: Category | null) => void;
  categoryCounts: Record<Category | "all", number>;
  shown: number;
  total: number;
}) {
  return (
    <div className="flex flex-col gap-2.5 px-5 pt-3 pb-3">
      <div className="flex items-center gap-2">
        <div className="relative min-w-0 flex-1">
          <MagnifyingGlass className="pointer-events-none absolute top-1/2 left-3 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            ref={searchRef}
            value={search}
            onChange={(e) => onSearch(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape" && search) {
                e.preventDefault();
                onSearch("");
              }
            }}
            placeholder={searchPlaceholder}
            aria-label="Search terms"
            className="pr-16 pl-8"
          />
          {search ? (
            <button
              type="button"
              aria-label="Clear search"
              onClick={() => onSearch("")}
              className="absolute top-1/2 right-3 -translate-y-1/2 text-muted-foreground hover:text-foreground"
            >
              <X className="size-3.5" />
            </button>
          ) : (
            <kbd className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2 rounded-md border border-border px-1.5 py-px font-mono text-[10px] text-muted-foreground">
              {shortcut("F")}
            </kbd>
          )}
        </div>

        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            {/* A fixed width, like the count beside it: the search field
                keeps its size whatever the Show label says. */}
            <Button
              variant="outline"
              size="sm"
              className={cn("w-[290px] justify-between", show !== "all" && "border-primary/60 text-primary")}
            >
              <span className="truncate">Show: {SHOW_LABEL[show]}</span> <CaretDown className="size-3" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-72">
            <DropdownMenuRadioGroup value={show} onValueChange={(v) => onShow(v as ShowFilter)}>
              {SHOW_FILTERS.map((f) => (
                <DropdownMenuRadioItem key={f} value={f}>
                  <span className="flex-1">{SHOW_LABEL[f]}</span>
                  <span className="text-[11px] tabular-nums text-muted-foreground">{showCounts[f]}</span>
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuContent>
        </DropdownMenu>

        <span className="min-w-[13ch] shrink-0 text-right text-[11.5px] tabular-nums text-muted-foreground">
          {shown.toLocaleString()} of {total.toLocaleString()}
        </span>
      </div>

      <div className="flex flex-wrap items-center gap-1.5" role="toolbar" aria-label="Categories">
        <Chip active={category === null} onClick={() => onCategory(null)} count={categoryCounts.all}>
          All
        </Chip>
        {CATEGORIES.map((c) => (
          <Tooltip key={c}>
            <TooltipTrigger asChild>
              <Chip
                active={category === c}
                count={categoryCounts[c]}
                onClick={() => onCategory(category === c ? null : c)}
              >
                {CATEGORY_LABEL[c]}
              </Chip>
            </TooltipTrigger>
            <TooltipContent side="bottom" sideOffset={4}>
              {CATEGORY_HINT[c]}
            </TooltipContent>
          </Tooltip>
        ))}
      </div>
    </div>
  );
}

function Chip({
  active,
  count,
  onClick,
  children,
  ...rest
}: {
  active: boolean;
  count: number;
  onClick: () => void;
  children: React.ReactNode;
} & React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      {...rest}
      className={cn(
        "inline-flex h-7 items-center gap-1.5 rounded-[13px] border px-2.5 text-[12px] transition-colors outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50",
        active
          ? "border-primary/60 bg-[color:color-mix(in_oklch,var(--color-gold)_10%,transparent)] text-primary"
          : "border-border text-foreground hover:border-[color:var(--input)] hover:bg-[color:var(--color-bg-hover)]",
        count === 0 && !active && "text-muted-foreground",
      )}
    >
      {children}
      <span className={cn("tabular-nums", active ? "text-primary/80" : "text-muted-foreground")}>
        {count}
      </span>
    </button>
  );
}
