export const IS_MAC = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);

/** "Ctrl F" / "⌘ F" for key hints. */
export function shortcut(key: string): string {
  return IS_MAC ? `⌘ ${key}` : `Ctrl ${key}`;
}

/** Ctrl on Linux/Windows, Cmd on macOS. */
export function hasMod(e: KeyboardEvent | React.KeyboardEvent): boolean {
  return IS_MAC ? e.metaKey : e.ctrlKey;
}

/** Focus is somewhere typing goes (Undo there is the field's own undo). */
export function isTextField(el: EventTarget | null): boolean {
  if (!(el instanceof HTMLElement)) return false;
  if (el.isContentEditable) return true;
  if (el instanceof HTMLTextAreaElement) return true;
  if (el instanceof HTMLInputElement) {
    return !["checkbox", "radio", "button", "submit", "reset", "range", "color", "file"].includes(el.type);
  }
  return false;
}
