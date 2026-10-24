import { useEffect, useRef } from "react";
import { CheckCircle } from "@phosphor-icons/react";
import { OpenFolderButton } from "@/features/welcome/open-folder-button";

/** Right after the first model connects: say so, and point at the next step — opening a
 *  folder of subtitles — instead of leaving the page to guess. */
export function NextStepCard({ tested }: { tested: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  // The setup form that had focus is gone; land on the next step (after the card below
  // has settled its own focus).
  useEffect(() => {
    const frame = requestAnimationFrame(() =>
      ref.current?.querySelector<HTMLButtonElement>("button")?.focus(),
    );
    return () => cancelAnimationFrame(frame);
  }, []);
  return (
    <div
      ref={ref}
      role="status"
      className="flex flex-wrap items-center gap-x-4 gap-y-2 rounded-2xl border border-[color-mix(in_oklch,var(--color-gold)_40%,var(--border))] bg-card px-5 py-4"
    >
      <CheckCircle weight="fill" className="size-5 shrink-0 text-[color:var(--color-jade)]" />
      <div className="min-w-0 flex-1 basis-56">
        <p className="text-[13px] font-semibold text-[color:var(--color-ink-emphasis)]">
          {tested ? "Connected." : "Saved."} Next: open a folder of subtitles
        </p>
        <p className="text-[11.5px] text-muted-foreground">
          Or drag a folder onto the window.
        </p>
      </div>
      <OpenFolderButton />
    </div>
  );
}
