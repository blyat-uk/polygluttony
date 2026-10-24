import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import type { FileResult } from "@/types/generated/FileResult";
import type { ProjectView } from "@/types/generated/ProjectView";
import type { StopInfo } from "@/types/generated/StopInfo";
import type { FileRow } from "@/stores/translation-store";
import { queryClient } from "@/lib/query-client";
import { folderTitle } from "@/lib/folder-title";
import { projectKey } from "@/features/project/use-project";
import { folderResultsKey } from "./queries";
import { untickTranslated } from "./selection";
import { notificationBody, tallyRun } from "./run-summary";

/**
 * What happens when a run ends, wherever the user is in the app: the files it
 * translated are unticked, the folder view and the stored results refetch, and
 * when Polygluttony isn't the focused window an OS notification says how it went.
 */
export function onRunFinished(
  folder: string,
  files: Record<string, FileRow>,
  results: FileResult[],
  stop: StopInfo | null,
) {
  queryClient.setQueryData<ProjectView>(projectKey(folder), (v) =>
    v
      ? { ...v, prefs: { ...v.prefs, selected_files: untickTranslated(v.prefs.selected_files, results) } }
      : v,
  );
  void queryClient.invalidateQueries({ queryKey: projectKey(folder) });
  void queryClient.invalidateQueries({ queryKey: folderResultsKey(folder) });
  void notifyIfAway(folderTitle(folder), notificationBody(tallyRun(files, results), stop));
}

const ASKED_KEY = "pg.notification-permission-asked";

async function notifyIfAway(title: string, body: string) {
  try {
    const focused = await getCurrentWindow()
      .isFocused()
      .catch(() => document.hasFocus());
    if (focused) return;
    let granted = await isPermissionGranted();
    if (!granted && !alreadyAsked()) {
      markAsked();
      granted = (await requestPermission()) === "granted";
    }
    if (granted) sendNotification({ title, body });
  } catch {
    // A notification is a nicety; the banner and the status bar still say it.
  }
}

/** Ask for permission once, ever: a "no" is respected. */
function alreadyAsked(): boolean {
  try {
    return localStorage.getItem(ASKED_KEY) === "1";
  } catch {
    return false;
  }
}

function markAsked() {
  try {
    localStorage.setItem(ASKED_KEY, "1");
  } catch {
    // Without storage we may ask again next session; that's fine.
  }
}
