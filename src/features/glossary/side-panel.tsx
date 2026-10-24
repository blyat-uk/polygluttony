import { useEffect, useRef, type ReactNode } from "react";
import { X } from "@phosphor-icons/react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

/**
 * The workspace's right-side panel, drawn over the list (not a modal: the
 * list stays usable beside it). Esc and ✕ call `onClose`; the owner decides
 * whether closing needs a confirm. An Esc already used by a menu, select,
 * dialog or inline edit (defaultPrevented) is left alone.
 */
export function SidePanel({
  title,
  onClose,
  canClose = true,
  children,
  footer,
  className,
}: {
  title: ReactNode;
  onClose: () => void;
  /** False while something runs: ✕ and Esc do nothing. */
  canClose?: boolean;
  children: ReactNode;
  footer?: ReactNode;
  className?: string;
}) {
  const close = useRef(onClose);
  close.current = onClose;
  const enabled = useRef(canClose);
  enabled.current = canClose;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented || !enabled.current) return;
      e.preventDefault();
      close.current();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <aside
      className={cn(
        "absolute inset-y-0 right-0 z-20 flex w-[420px] max-w-full flex-col border-l border-[color:var(--input)] bg-popover",
        "animate-in fade-in-0 slide-in-from-right-4 duration-200",
        className,
      )}
    >
      <div className="flex items-center justify-between gap-3 border-b border-border px-5 py-3">
        <h2 className="text-[14px] font-semibold text-foreground">{title}</h2>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="Close"
          disabled={!canClose}
          onClick={onClose}
        >
          <X className="size-4" />
        </Button>
      </div>
      <div className="min-h-0 flex-1 overflow-auto px-5 py-4">{children}</div>
      {footer ? (
        <div className="flex items-center justify-end gap-2 border-t border-border px-5 py-3">
          {footer}
        </div>
      ) : null}
    </aside>
  );
}

/** Small uppercase label above a group of controls. */
export function PanelLabel({ children }: { children: ReactNode }) {
  return (
    <div className="mb-2 text-[10.5px] font-semibold tracking-[0.12em] text-muted-foreground uppercase">
      {children}
    </div>
  );
}
