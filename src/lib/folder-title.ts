/** Folder names that don't say which show it is: a working "translate" folder
 *  or a season folder. */
const GENERIC = /^translate$|season/i;

/** A Windows drive ("C:") is never a show name. */
const DRIVE = /^[a-z]:$/i;

/** The name a folder goes by in a list: its own, or for a generic one
 *  (`…/Ranger Wars/translate`, `…/Show/Season 1/translate`) the nearest
 *  ancestor that names the show. Falls back to its own name when every
 *  ancestor is generic. */
export function folderTitle(path: string): string {
  const parts = path.split(/[/\\]/).filter(Boolean);
  for (let i = parts.length - 1; i >= 0; i--) {
    if (!GENERIC.test(parts[i]) && !DRIVE.test(parts[i])) return parts[i];
  }
  return parts[parts.length - 1] ?? path;
}
