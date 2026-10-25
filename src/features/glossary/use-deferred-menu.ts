import { useRef } from "react";

/**
 * Run a menu item's action only after the menu has closed. While open, a
 * Radix menu traps focus, so a dialog or panel opened from `onSelect` would
 * lose focus straight back to the menu trigger. Pass `onCloseAutoFocus` to
 * the menu content and wrap item handlers in `pick`.
 */
export function useDeferredMenu() {
  const pending = useRef<(() => void) | null>(null);
  return {
    pick: (fn: () => void) => () => {
      pending.current = fn;
    },
    onCloseAutoFocus: (e: Event) => {
      const fn = pending.current;
      if (!fn) return;
      pending.current = null;
      e.preventDefault();
      fn();
    },
  };
}
