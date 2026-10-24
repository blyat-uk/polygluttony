import { useEffect, useRef, type RefObject } from "react";

interface PrimaryAction {
  run: () => void;
  enabled: boolean;
}

// Registrations in mount order; the newest enabled one is the page's action.
const registered: RefObject<PrimaryAction>[] = [];

/** Register the current page's main action for the app-wide Ctrl/Cmd+Enter
 *  shortcut. Pass `enabled: false` while the action is unavailable. */
export function usePrimaryAction(run: () => void, enabled = true) {
  const ref = useRef<PrimaryAction>({ run, enabled });
  ref.current = { run, enabled };
  useEffect(() => {
    registered.push(ref);
    return () => {
      const i = registered.indexOf(ref);
      if (i >= 0) registered.splice(i, 1);
    };
  }, []);
}

/** Run the current page's main action. False when there is none to run. */
export function runPrimaryAction(): boolean {
  for (let i = registered.length - 1; i >= 0; i--) {
    const action = registered[i].current;
    if (action.enabled) {
      action.run();
      return true;
    }
  }
  return false;
}
