import { useGlossaryRun } from "@/stores/glossary-store";
import { RunScreen } from "./run-screen";

export function ImportProgress() {
  const opDetail = useGlossaryRun((s) => s.opDetail);
  return (
    <RunScreen
      title="Importing from translated episodes"
      description={`Reading the names and terms in ${opDetail ?? "your translated files"}.`}
      cancelNote="Stopping keeps the terms found so far."
    />
  );
}
