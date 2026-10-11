import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Globe } from "@phosphor-icons/react";
import { ipc } from "@/lib/ipc";
import { webSearchStatusKey } from "@/features/connections/use-connections";
import { Checkbox } from "@/components/ui/checkbox";
import { Textarea } from "@/components/ui/textarea";
import { HelpText } from "@/components/help-text";

/** The normalize + personalize + context controls shared by the Build screen
 *  (CreateView) and the Update screen (UpdateView).
 *
 *  Owns the web-search-model query rather than taking it as a prop: both
 *  callers need identical gating, and the shared react-query key (refreshed by
 *  every Connections change) means mounting it from either screen costs one fetch. */
export function GlossaryBuildOptions({
  normalize,
  onNormalizeChange,
  personalize,
  onPersonalizeChange,
  context,
  onContextChange,
  normalizeHelp,
  disabled = false,
}: {
  normalize: boolean;
  onNormalizeChange: (v: boolean) => void;
  personalize: boolean;
  onPersonalizeChange: (v: boolean) => void;
  context: string;
  onContextChange: (v: string) => void;
  normalizeHelp: string;
  disabled?: boolean;
}) {
  // Label of the model that will search (e.g. "Claude Sonnet 5.5"), or null when none resolves.
  const { data: searchModel } = useQuery({
    queryKey: webSearchStatusKey,
    queryFn: ipc.webSearchStatus,
  });

  return (
    <>
      <label className="flex items-start gap-2.5 cursor-pointer mb-3">
        <Checkbox
          checked={normalize}
          onCheckedChange={(v) => onNormalizeChange(v === true)}
          disabled={disabled}
          className="mt-0.5"
        />
        <span className="text-sm text-foreground select-none">Clean up &amp; standardize</span>
      </label>
      <div className="ml-6 mb-4">
        <HelpText>{normalizeHelp}</HelpText>
      </div>

      <label
        className={`flex items-start gap-2.5 mb-1 ${!searchModel ? "opacity-50 cursor-not-allowed" : "cursor-pointer"}`}
      >
        <Checkbox
          checked={personalize}
          onCheckedChange={(v) => onPersonalizeChange(v === true)}
          disabled={disabled || !searchModel}
          className="mt-0.5"
        />
        <span className="text-sm text-foreground select-none">
          Look up established names online
        </span>
      </label>
      <div className="ml-6 mb-3">
        {!searchModel ? (
          <p className="mt-1 flex items-start gap-1 text-[11px] leading-snug text-muted-foreground">
            <Globe className="mt-px size-3 shrink-0 text-muted-foreground" />
            <span>
              Needs a model that can search the web — set up Web search in{" "}
              <Link to="/connections" className="text-primary hover:underline">
                Connections
              </Link>
              .
            </span>
          </p>
        ) : (
          <HelpText>
            {searchModel} searches the web for this show&apos;s commonly-used names, so your
            glossary matches what fans expect.
          </HelpText>
        )}
      </div>

      {personalize && searchModel ? (
        <div className="ml-6 mb-2">
          <Textarea
            value={context}
            onChange={(e) => onContextChange(e.target.value)}
            placeholder="Show name (first line), wiki links or notes…"
            className="text-sm"
          />
        </div>
      ) : null}
    </>
  );
}
