import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { elementScroll, useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDownIcon, ArrowUpIcon } from "@phosphor-icons/react";
import { cn } from "@/lib/utils";
import type { EpisodeLine } from "@/types/generated/EpisodeLine";
import type { VerifyIssue } from "@/types/generated/VerifyIssue";
import { estimateRowHeight } from "./format";
import { CELL_PAD, LineRow, ROW_GRID } from "./line-row";
import { usePrefersReducedMotion } from "./use-reduced-motion";

/** How long after landing a translation keeps its fade: at least the
 *  animation's length, so a re-render mid-fade doesn't cut it short. A row
 *  scrolled into view later just shows the text. */
const LAND_WINDOW_MS = 1200;

/** Scroll events this soon after one of our own scroll writes are ours. */
const AUTO_GRACE_MS = 250;
/** …and for a smooth scroll, until it arrives or this much time has passed. */
const SMOOTH_GRACE_MS = 1200;

const SCROLL_KEYS = new Set([
  "ArrowUp",
  "ArrowDown",
  "PageUp",
  "PageDown",
  "Home",
  "End",
  " ",
]);

const COLUMNS = ["#", "Time", "Original", "Translation"] as const;

type ExpectedScroll = { target: number; smooth: boolean; at: number };

export function LineTable({
  lines,
  live,
  issues,
  landed,
}: {
  lines: EpisodeLine[];
  /** Indices of the lines being sent or re-sent, ascending. */
  live: number[];
  issues: Map<number, VerifyIssue[]>;
  /** Line id → when its translation landed (`performance.now()`). */
  landed: ReadonlyMap<number, number>;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const reducedMotion = usePrefersReducedMotion();

  // Follow the live batch until the user scrolls. State drives the pill; the
  // ref is what the native listeners read.
  const [follow, setFollowState] = useState(true);
  const followRef = useRef(true);
  const setFollow = useCallback((on: boolean) => {
    followRef.current = on;
    setFollowState(on);
  }, []);

  // Every scroll the virtualizer performs (follow jumps, size corrections,
  // reconcile steps) goes through `scrollToFn`, so we note where it's headed and
  // can tell those scroll events from the user's.
  const expected = useRef<ExpectedScroll | null>(null);
  // Where follow puts the first live row: a third of the way down. Set just
  // before each follow jump and passed on every render so it sticks.
  const followPad = useRef(0);

  // Width of the body's scrollbar, mirrored on the header row so the columns
  // line up whether or not the platform draws a classic scrollbar.
  const [gutter, setGutter] = useState(0);
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const measure = () => setGutter(el.offsetWidth - el.clientWidth);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const getItemKey = useCallback((i: number) => lines[i]?.id ?? i, [lines]);

  const virtualizer = useVirtualizer({
    count: lines.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (i) => estimateRowHeight(lines[i]),
    getItemKey,
    overscan: 10,
    scrollPaddingStart: followPad.current,
    scrollToFn: (offset, options, instance) => {
      expected.current = {
        target: offset + (options.adjustments ?? 0),
        smooth: options.behavior === "smooth",
        at: performance.now(),
      };
      elementScroll(offset, options, instance);
    },
  });

  const scrollToLive = useCallback(
    (index: number) => {
      const el = scrollRef.current;
      if (!el) return;
      const pad = Math.round(el.clientHeight / 3);
      followPad.current = pad;
      virtualizer.options.scrollPaddingStart = pad;
      // Smooth for a short hop; a long one jumps (a smooth scroll across
      // thousands of unmeasured rows is slow and keeps retargeting).
      const item = virtualizer.measurementsCache[index];
      const far = !item || Math.abs(item.start - pad - el.scrollTop) > el.clientHeight * 2;
      virtualizer.scrollToIndex(index, {
        align: "start",
        behavior: reducedMotion || far ? "auto" : "smooth",
      });
    },
    [virtualizer, reducedMotion],
  );

  // A new set of lines went out (any id not in the previous set; a set that
  // only shrinks as lines land isn't new): bring its first line into view.
  // Layout effect, so a window opened mid-batch paints at the batch, not the top.
  const prevLive = useRef<Set<number>>(new Set());
  useLayoutEffect(() => {
    const ids = live.map((i) => lines[i].id);
    const prev = prevLive.current;
    const isNew = ids.some((id) => !prev.has(id));
    prevLive.current = new Set(ids);
    if (isNew && followRef.current) scrollToLive(live[0]);
  }, [live, lines, scrollToLive]);

  // User scrolls turn follow off. Input that scrolls (wheel, touch drag, scroll
  // keys) counts at once; any other scroll event counts unless it lines up with
  // one of the virtualizer's own scroll writes. That catches scrollbar drags
  // even on engines that don't report pointer events on the scrollbar.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    let pointerDown = false;
    let lastTop = el.scrollTop;

    const userScrolled = () => {
      if (!followRef.current) return;
      setFollow(false);
      // Stop the virtualizer steering back toward the live rows.
      virtualizer.cancelScroll();
    };
    const onKey = (e: KeyboardEvent) => {
      if (!SCROLL_KEYS.has(e.key)) return;
      const t = e.target as HTMLElement | null;
      // Space or arrows on the issue marker don't scroll.
      if (t && t !== el && t.closest("button, a, input, textarea, select")) return;
      userScrolled();
    };
    const onPointerDown = () => {
      pointerDown = true;
    };
    const onPointerUp = () => {
      pointerDown = false;
    };
    const onScroll = () => {
      const top = el.scrollTop;
      const prevTop = lastTop;
      lastTop = top;
      // Scrollbar drag, text-selection autoscroll, middle-click autoscroll.
      if (pointerDown) return userScrolled();
      const ex = expected.current;
      if (ex) {
        const age = performance.now() - ex.at;
        if (age < (ex.smooth ? SMOOTH_GRACE_MS : AUTO_GRACE_MS)) {
          // A smooth scroll that has arrived stops covering later events.
          if (Math.abs(top - ex.target) < 2) ex.smooth = false;
          return;
        }
      }
      // The browser pulled the position up because the list got shorter.
      if (top < prevTop && top >= el.scrollHeight - el.clientHeight - 1) return;
      userScrolled();
    };

    el.addEventListener("wheel", userScrolled, { passive: true });
    el.addEventListener("touchmove", userScrolled, { passive: true });
    el.addEventListener("keydown", onKey);
    el.addEventListener("pointerdown", onPointerDown);
    el.addEventListener("scroll", onScroll, { passive: true });
    window.addEventListener("pointerup", onPointerUp);
    window.addEventListener("pointercancel", onPointerUp);
    window.addEventListener("blur", onPointerUp);
    return () => {
      el.removeEventListener("wheel", userScrolled);
      el.removeEventListener("touchmove", userScrolled);
      el.removeEventListener("keydown", onKey);
      el.removeEventListener("pointerdown", onPointerDown);
      el.removeEventListener("scroll", onScroll);
      window.removeEventListener("pointerup", onPointerUp);
      window.removeEventListener("pointercancel", onPointerUp);
      window.removeEventListener("blur", onPointerUp);
    };
  }, [virtualizer, setFollow]);

  const items = virtualizer.getVirtualItems();
  const now = performance.now();

  // "Jump to live" while lines are out and none of them is on screen.
  const range = virtualizer.range;
  let pill: "up" | "down" | null = null;
  if (!follow && live.length > 0 && range) {
    const visible = live.some((i) => i >= range.startIndex && i <= range.endIndex);
    if (!visible) pill = live[0] > range.endIndex ? "down" : "up";
  }

  const jumpToLive = () => {
    if (live.length === 0) return;
    setFollow(true);
    scrollToLive(live[0]);
    // The pill goes away; keep keyboard focus on the lines rather than <body>.
    scrollRef.current?.focus({ preventScroll: true });
  };

  return (
    <div
      role="table"
      aria-rowcount={lines.length + 1}
      aria-label="Episode lines"
      className="relative flex min-h-0 flex-1 flex-col"
    >
      <div className="shrink-0 border-b border-border bg-[color:var(--card)]" style={{ paddingRight: gutter }}>
        <div
          role="row"
          aria-rowindex={1}
          className={cn(ROW_GRID, "py-1.5 text-[10.5px] font-medium uppercase tracking-wide text-muted-foreground")}
        >
          {COLUMNS.map((label, i) => (
            <div key={label} role="columnheader" className={CELL_PAD[i]}>
              {label}
            </div>
          ))}
        </div>
      </div>

      <div
        ref={scrollRef}
        role="rowgroup"
        tabIndex={0}
        className="min-h-0 flex-1 overflow-y-auto outline-none [overflow-anchor:none] focus-visible:shadow-[inset_0_0_0_1px_color-mix(in_oklch,var(--color-gold)_35%,transparent)]"
      >
        <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
          {items.map((item) => {
            const line = lines[item.index];
            if (!line) return null;
            return (
              <LineRow
                key={item.key}
                line={line}
                index={item.index}
                start={item.start}
                issues={issues.get(line.id)}
                fresh={now - (landed.get(line.id) ?? -Infinity) < LAND_WINDOW_MS}
                measure={virtualizer.measureElement}
              />
            );
          })}
        </div>
      </div>

      {pill ? (
        <button
          type="button"
          onClick={jumpToLive}
          style={{ right: 20 + gutter }}
          className="absolute bottom-4 z-10 inline-flex items-center gap-1.5 rounded-full border border-[color:color-mix(in_oklch,var(--color-gold)_40%,transparent)] bg-[color:var(--color-raised)] py-1.5 pl-2.5 pr-3 text-[11.5px] font-medium text-[color:var(--color-gold-hi)] shadow-[0_8px_24px_-8px_rgba(0,0,0,.8)] outline-none hover:border-[color:color-mix(in_oklch,var(--color-gold)_65%,transparent)] hover:bg-[color:var(--color-bg-hover)] focus-visible:ring-2 focus-visible:ring-ring/60 motion-safe:animate-in motion-safe:fade-in-0 motion-safe:slide-in-from-bottom-1"
        >
          {pill === "down" ? <ArrowDownIcon size={12} weight="bold" /> : <ArrowUpIcon size={12} weight="bold" />}
          Jump to live
        </button>
      ) : null}
    </div>
  );
}
