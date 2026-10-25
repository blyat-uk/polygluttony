/** The shortcut modifier as this computer shows it: "⌘" on a Mac, "Ctrl+" elsewhere. */
export const MOD =
  typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform) ? "⌘" : "Ctrl+";
