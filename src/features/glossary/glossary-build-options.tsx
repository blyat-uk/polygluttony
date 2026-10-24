import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Globe } from "@phosphor-icons/react";
import { ipc } from "@/lib/ipc";
import { webSearchStatusKey } from "@/features/connections/use-connections";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { HelpText } from "@/components/help-text";

/** Label of the model that does web search (e.g. "Claude Sonnet 5.5"), or null.
 *  The shared key is refreshed by every Models change. */
export function useSearchModel() {
  const { data } = useQuery({ queryKey: webSearchStatusKey, queryFn: ipc.webSearchStatus });
  return data ?? null;
}

/** "Tidy up" checkbox for a scan. */
export function TidyOption({
  checked,
  onChange,
  label = "Tidy up",
  help = "Fixes inconsistent spellings and drops terms too generic to be useful.",
  disabled = false,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label?: string;
  help?: string;
  disabled?: boolean;
}) {
  return (
    <div>
      <label className="flex cursor-pointer items-start gap-2.5">
        <Checkbox
          checked={checked}
          onCheckedChange={(v) => onChange(v === true)}
          disabled={disabled}
          className="mt-0.5"
        />
        <span className="text-sm text-foreground select-none">{label}</span>
      </label>
      <div className="ml-6.5">
        <HelpText>{help}</HelpText>
      </div>
    </div>
  );
}

/** "Look up names online" for a first build. Only offered when a model that
 *  can search the web is set up; the show name steers the search. */
export function LookupOption({
  checked,
  onChange,
  showName,
  onShowNameChange,
  disabled = false,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  showName: string;
  onShowNameChange: (v: string) => void;
  disabled?: boolean;
}) {
  const searchModel = useSearchModel();
  return (
    <div>
      <label
        className={`flex items-start gap-2.5 ${searchModel ? "cursor-pointer" : "cursor-not-allowed opacity-50"}`}
      >
        <Checkbox
          checked={checked && !!searchModel}
          onCheckedChange={(v) => onChange(v === true)}
          disabled={disabled || !searchModel}
          className="mt-0.5"
        />
        <span className="text-sm text-foreground select-none">Look up names online</span>
      </label>
      <div className="ml-6.5">
        {searchModel ? (
          <HelpText>
            {searchModel} searches the web for the names fans of this show already use.
          </HelpText>
        ) : (
          <p className="mt-1 flex items-start gap-1 text-[11px] leading-snug text-muted-foreground">
            <Globe className="mt-px size-3 shrink-0" />
            <span>
              Needs a model that can search the web. Set one up in{" "}
              <Link to="/connections" className="text-primary hover:underline">
                Models
              </Link>
              .
            </span>
          </p>
        )}
        {checked && searchModel ? (
          <div className="mt-2">
            <Input
              value={showName}
              onChange={(e) => onShowNameChange(e.target.value)}
              placeholder="Show name"
              aria-label="Show name"
              disabled={disabled}
            />
          </div>
        ) : null}
      </div>
    </div>
  );
}
