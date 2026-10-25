import { toast } from "sonner";
import type { BuildMode } from "@/types/generated/BuildMode";
import type { ProjectView } from "@/types/generated/ProjectView";
import { ipc } from "@/lib/ipc";
import { useGlossaryRun } from "@/stores/glossary-store";
import { useGlossaryUi } from "./ui-store";

export type BuildKind = "first" | "scan" | "rebuild";

const MODE: Record<BuildKind, BuildMode> = { first: "append", scan: "append", rebuild: "regenerate" };

/**
 * Start a glossary build over the given episodes. The page swaps to the build
 * progress screen while it runs; `kind` picks that screen's title and what the
 * workspace shows when it lands (new terms after a scan, the next step after
 * a first build). `chained`: the Episodes page translates as soon as it lands,
 * so there's no next step to show.
 */
export function startBuild(
  view: ProjectView,
  opts: {
    kind: BuildKind;
    files: string[];
    tidy: boolean;
    lookup?: boolean;
    showName?: string;
    chained?: boolean;
  },
) {
  const run = useGlossaryRun.getState();
  if (run.busy !== null) return;
  const mode = MODE[opts.kind];
  useGlossaryUi.setState({
    panel: null,
    improve: null,
    afterScan: opts.kind === "scan",
    firstBuild: opts.kind === "first" && !opts.chained ? view.folder : null,
  });
  run.startOp("build", view.folder);
  run.setBuildMode(mode);
  ipc
    .startGlossaryBuild({
      folder: view.folder,
      mode,
      files: opts.files,
      worldType: view.prefs.world_override ?? view.detected_world,
      sourceLang: view.prefs.source_lang,
      targetLang: view.prefs.target_lang,
      normalize: opts.tidy,
      personalize: !!opts.lookup,
      personalizeContext: opts.lookup ? (opts.showName ?? "").trim() : "",
    })
    .catch((e: unknown) => {
      // Rejected invoke = the run never started; un-stick the page and keep
      // the reason for whoever is waiting on this build.
      useGlossaryRun.setState({ error: String(e) });
      useGlossaryRun.getState().endOp();
      useGlossaryUi.setState({ afterScan: false });
      toast.error(String(e));
    });
}
