import { useId, useState, type ReactNode } from "react";
import { MagnifyingGlass, Plus } from "@phosphor-icons/react";
import { toast } from "sonner";
import type { ConnectionsConfig } from "@/types/generated/ConnectionsConfig";
import type { ModelCapabilities } from "@/types/generated/ModelCapabilities";
import type { ProviderInfo } from "@/types/generated/ProviderInfo";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import {
  findProvider,
  isUsable,
  modelLabel,
  providerName,
  removalForgetsKey,
} from "./connection-model";
import { ConnectionForm, type FormStage } from "./connection-form";
import { ProviderMark } from "./provider-tiles";
import { CardEyebrow, EyebrowNote, RoleCard, StatusDot } from "./role-card";
import { useConnectionMutations } from "./use-connections";

/** The glossary checkbox this powers, named exactly as it appears there. */
const GLOSSARY_OPTION = "“Look up established names online”";

/**
 * ② Web search — optional. Covered by the translation model, offered (when it can't
 * search, or is switched off), a separate model, or waiting for translation.
 */
export function WebSearchCard({
  config,
  providers,
  translationCaps,
}: {
  config: ConnectionsConfig;
  providers: ProviderInfo[];
  /** Capabilities of the translation model; undefined while loading or when there is none. */
  translationCaps: ModelCapabilities | undefined;
}) {
  const titleId = useId();
  const m = useConnectionMutations();
  const [form, setForm] = useState<"add" | "edit" | null>(null);
  const [stage, setStage] = useState<FormStage>("pick");
  const [confirmRemove, setConfirmRemove] = useState(false);
  const ws = config.web_search;
  const eyebrow = (aside: ReactNode) => (
    <CardEyebrow n={2} title="Web search" id={titleId} aside={aside} />
  );

  const remove = () =>
    m.updateWebSearch.mutate(
      { mode: "off" },
      {
        onSuccess: () => {
          setForm(null);
          toast.success("Web search model removed");
        },
      },
    );

  const keyNote =
    ws.mode === "separate" &&
    removalForgetsKey(ws.connection, config.translation, config.provider_keys)
      ? ` Your saved ${providerName(findProvider(providers, ws.connection.provider), ws.connection)} API key will be forgotten too.`
      : null;
  const removeDialog = (
    <AlertDialog open={confirmRemove} onOpenChange={setConfirmRemove}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Remove the web search model?</AlertDialogTitle>
          <AlertDialogDescription>
            The glossary won’t look up names online until you set web search up again.
            {keyNote}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Keep it</AlertDialogCancel>
          <AlertDialogAction variant="destructive" onClick={remove}>
            Remove
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );

  // Adding or editing a separate model: the shared form, search-capable providers only.
  if (form) {
    const editing = form === "edit" && ws.mode === "separate";
    return (
      <RoleCard variant="solid" labelledBy={titleId}>
        {eyebrow(
          editing ? (
            <StatusDot tone="jade" glow>
              Connected
            </StatusDot>
          ) : (
            <StatusDot tone="muted" hollow>
              {stage === "fields" ? "Not connected yet" : "Not set up"}
            </StatusDot>
          ),
        )}
        <ConnectionForm
          key={form}
          role="web-search"
          providers={providers}
          providerKeys={config.provider_keys}
          initial={editing ? ws.connection : null}
          autoFocus
          onStageChange={setStage}
          onSave={async (connection, tested) => {
            await m.saveWebSearch.mutateAsync({ mode: "separate", connection });
            setForm(null);
            const info = findProvider(providers, connection.provider);
            const name = `${providerName(info, connection)} · ${modelLabel(info, connection.model)}`;
            toast.success(tested ? `Web search connected: ${name}` : `Web search saved (not yet confirmed): ${name}`);
          }}
          onCancel={() => setForm(null)}
          onRemove={editing ? () => setConfirmRemove(true) : undefined}
        />
        {removeDialog}
      </RoleCard>
    );
  }

  // d · a separate web-search model.
  if (ws.mode === "separate") {
    const conn = ws.connection;
    const info = findProvider(providers, conn.provider);
    const name = providerName(info, conn);
    const reusing = conn.api_key !== "" && config.provider_keys[conn.provider] === conn.api_key;
    return (
      <RoleCard variant="solid" labelledBy={titleId}>
        {eyebrow(
          isUsable(conn) ? (
            <StatusDot tone="jade" glow>
              Connected
            </StatusDot>
          ) : (
            <StatusDot tone="amber" hollow>
              Needs an API key
            </StatusDot>
          ),
        )}
        <div className="mt-3 flex items-center gap-3">
          <ProviderMark info={info} provider={conn.provider} size="lg" />
          <div className="min-w-0 flex-1">
            <div className="truncate text-[14px] font-semibold text-[color:var(--color-ink-emphasis)]">
              {name}
            </div>
            <div className="truncate text-[11.5px] text-muted-foreground">
              {modelLabel(info, conn.model)}
              {reusing ? ` · reusing your saved ${name} key` : ""}
            </div>
          </div>
          <div className="flex shrink-0 gap-1.5">
            <Button variant="outline" size="sm" onClick={() => setForm("edit")}>
              Edit
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="text-muted-foreground"
              disabled={m.updateWebSearch.isPending}
              onClick={() => setConfirmRemove(true)}
            >
              Remove
            </Button>
          </div>
        </div>
        {removeDialog}
      </RoleCard>
    );
  }

  // — no translation yet: nothing to attach web search to.
  if (!config.translation) {
    return (
      <RoleCard variant="dashed" labelledBy={titleId}>
        {eyebrow(<EyebrowNote>optional · set up translation first</EyebrowNote>)}
      </RoleCard>
    );
  }

  // a · the translation model searches.
  if (ws.mode === "translation") {
    return (
      <RoleCard variant="solid" labelledBy={titleId}>
        {eyebrow(<StatusDot tone="aqua">On · same model</StatusDot>)}
        <div className="mt-3 flex items-center gap-3">
          <span
            aria-hidden
            className="grid size-7 shrink-0 place-items-center rounded-lg border border-[color-mix(in_oklch,var(--color-aqua)_25%,transparent)] bg-[color-mix(in_oklch,var(--color-aqua)_8%,transparent)] text-[color:var(--color-aqua)]"
          >
            <MagnifyingGlass weight="bold" className="size-3.5" />
          </span>
          <div className="min-w-0">
            <p className="text-[12.5px]">Handled by your translation model.</p>
            <p className="text-[11.5px] text-muted-foreground">
              Used by {GLOSSARY_OPTION} in Glossary.
            </p>
          </div>
        </div>
        <Button
          variant="link"
          size="sm"
          className="mt-2 h-auto px-0 py-0.5 text-[12px]"
          onClick={() => setForm("add")}
        >
          Use a different model instead
        </Button>
      </RoleCard>
    );
  }

  // Off. Wait for the translation model's capabilities before saying which "off" it is.
  if (!translationCaps) {
    return (
      <RoleCard variant="dashed" labelledBy={titleId}>
        {eyebrow(<EyebrowNote>optional</EyebrowNote>)}
      </RoleCard>
    );
  }

  // c · the translation model could search, but it's switched off.
  if (translationCaps.web_search) {
    return (
      <RoleCard variant="dashed" labelledBy={titleId}>
        {eyebrow(
          <StatusDot tone="muted" hollow>
            Off
          </StatusDot>,
        )}
        <p className="mt-2.5 text-[12px] text-muted-foreground">
          Turn on <span className="font-medium text-foreground">Web search</span> on your
          translation model above, or use a separate model.
        </p>
        <Button variant="outline" size="sm" className="mt-3" onClick={() => setForm("add")}>
          <Plus weight="bold" /> Add a separate model
        </Button>
      </RoleCard>
    );
  }

  // b · the translation model can't search.
  return (
    <RoleCard variant="dashed" labelledBy={titleId}>
      {eyebrow(<EyebrowNote>optional</EyebrowNote>)}
      <p className="mt-2.5 text-[12.5px]">Your translation model can’t search the web.</p>
      <p className="mt-0.5 text-[11.5px] text-muted-foreground">
        Add one so the glossary can look up names fans already use. Anthropic, OpenAI and Google
        can search.
      </p>
      <Button size="sm" className="mt-3" onClick={() => setForm("add")}>
        <Plus weight="bold" /> Add web search
      </Button>
    </RoleCard>
  );
}
