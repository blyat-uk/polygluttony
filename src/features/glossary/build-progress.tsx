import { useGlossaryRun } from "@/stores/glossary-store";
import { RunScreen } from "./run-screen";
import { ExtractionConsole } from "./extraction-console";
import { useGlossaryUi } from "./ui-store";

export function BuildProgress() {
  // Stopping an append can only lose work that was never in the glossary.
  // Stopping a rebuild leaves the partial result IN PLACE OF the old glossary,
  // so the reassuring append wording would be a lie there.
  const rebuilding = useGlossaryRun((s) => s.buildMode) === "regenerate";
  const folder = useGlossaryRun((s) => s.folder);
  const first = useGlossaryUi((s) => s.firstBuild !== null && s.firstBuild === folder);
  return (
    <RunScreen
      title={rebuilding ? "Rebuilding from scratch" : first ? "Building glossary" : "Adding new episodes"}
      description="Finding names, terms and places in your subtitles."
      cancelNote={
        rebuilding
          ? "Stopping keeps the names found so far, in place of the old glossary. Restore an earlier version… brings the old one back."
          : "Stopping keeps the names found so far."
      }
      body={<ExtractionConsole />}
    />
  );
}
