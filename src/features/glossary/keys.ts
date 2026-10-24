export function glossaryKey(folder: string) {
  return ["glossary", folder] as const;
}

export function snapshotsKey(folder: string) {
  return ["glossary-snapshots", folder] as const;
}

// I3 — self-save suppression: every save renames glossary.json, which fires
// FileChanged → fileTick → invalidate. That invalidation can resolve after a
// newer optimistic setQueryData and silently revert the edit, so the page skips
// it for 1500 ms after an in-app save. External edits (OS editor) can't
// coincide with active in-app editing, so this is an accepted blind spot.
let lastLocalSaveAt = 0;

export function markLocalSave() {
  lastLocalSaveAt = Date.now();
}

export function recentlySavedLocally(): boolean {
  return Date.now() - lastLocalSaveAt < 1500;
}
