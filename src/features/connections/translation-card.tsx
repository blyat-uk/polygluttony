import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Check } from "@phosphor-icons/react";
import { toast } from "sonner";
import type { Connection } from "@/types/generated/Connection";
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
import { Switch } from "@/components/ui/switch";
import {
  findProvider,
  hostOf,
  isUsable,
  normaliseThinking,
  providerKind,
  providerName,
  removalForgetsKey,
} from "./connection-model";
import { ConnectionForm, type FormStage } from "./connection-form";
import { ModelPicker } from "./model-picker";
import { ProviderMark } from "./provider-tiles";
import { CardEyebrow, RoleCard, StatusDot } from "./role-card";
import { ThinkingControl, thinkingHelp } from "./thinking-control";
import { TuningSection } from "./tuning-section";
import { capabilitiesQuery, useConnectionMutations, useLiveModels } from "./use-connections";

type Mutations = ReturnType<typeof useConnectionMutations>;

/** ① Translation — the required slot. A summary when connected, the setup form otherwise. */
export function TranslationCard({
  config,
  providers,
  caps,
}: {
  config: ConnectionsConfig;
  providers: ProviderInfo[];
  /** Capabilities of the saved translation model (shared with the web-search card). */
  caps: ModelCapabilities | undefined;
}) {
  const titleId = useId();
  const m = useConnectionMutations();
  const [editing, setEditing] = useState(false);
  const [stage, setStage] = useState<FormStage>("pick");
  const [confirmRemove, setConfirmRemove] = useState(false);
  // After leaving the form, put focus back on Edit rather than dropping it.
  const [returnFocus, setReturnFocus] = useState(false);
  const feedback = useSaveFeedback(
    m.updateTranslation.isPending || m.updateWebSearch.isPending,
    m.updateTranslation.isError || m.updateWebSearch.isError,
  );

  const conn = config.translation;
  const formMode = !conn || editing;
  const searchConn = config.web_search.mode === "separate" ? config.web_search.connection : null;
  const keyNote =
    conn && removalForgetsKey(conn, searchConn, config.provider_keys)
      ? ` Your saved ${providerName(findProvider(providers, conn.provider), conn)} API key will be forgotten too.`
      : null;
  const leaveForm = () => {
    setEditing(false);
    setReturnFocus(true);
  };

  const status = conn ? (
    isUsable(conn) ? (
      <StatusDot tone="jade" glow>
        Connected
      </StatusDot>
    ) : (
      <StatusDot tone="amber" hollow>
        Needs an API key
      </StatusDot>
    )
  ) : (
    <StatusDot tone="amber" hollow>
      {stage === "fields" ? "Not connected yet" : "Not set up"}
    </StatusDot>
  );

  const remove = async () => {
    try {
      await m.removeTranslation.mutateAsync();
      setEditing(false);
      toast.success("Translation connection removed");
    } catch (e) {
      toast.error(`Couldn’t remove it: ${String(e)}`);
    }
  };

  return (
    <RoleCard variant="primary" labelledBy={titleId}>
      <CardEyebrow
        n={1}
        title="Translation"
        id={titleId}
        aside={
          <>
            {!formMode ? <SaveFeedback state={feedback} /> : null}
            {status}
          </>
        }
      />
      {formMode ? (
        <ConnectionForm
          key={conn ? "edit" : "setup"}
          role="translation"
          providers={providers}
          providerKeys={config.provider_keys}
          initial={conn}
          onStageChange={setStage}
          onSave={async (c, tested) => {
            await m.saveTranslation.mutateAsync(c);
            leaveForm();
            const name = providerName(findProvider(providers, c.provider), c);
            if (tested) toast.success(`Connected to ${name}`);
            else toast.success(`Saved without confirming — ${name} didn’t answer the test.`);
          }}
          onCancel={conn ? leaveForm : undefined}
          onRemove={conn ? () => setConfirmRemove(true) : undefined}
        />
      ) : (
        <TranslationSummary
          conn={conn}
          info={findProvider(providers, conn.provider)}
          config={config}
          caps={caps}
          m={m}
          focusEdit={returnFocus}
          onEdit={() => {
            setReturnFocus(false);
            setEditing(true);
          }}
        />
      )}

      <AlertDialog open={confirmRemove} onOpenChange={setConfirmRemove}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove the translation connection?</AlertDialogTitle>
            <AlertDialogDescription>
              You won’t be able to translate until you connect a provider again.
              {keyNote}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep it</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => void remove()}>
              Remove
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </RoleCard>
  );
}

/** Connected state: provider + inline model menu, then only the controls this model supports.
 *  Model, Thinking and Web search save as soon as they change. */
function TranslationSummary({
  conn,
  info,
  config,
  caps,
  m,
  focusEdit,
  onEdit,
}: {
  conn: Connection;
  info: ProviderInfo | undefined;
  config: ConnectionsConfig;
  caps: ModelCapabilities | undefined;
  m: Mutations;
  focusEdit: boolean;
  onEdit: () => void;
}) {
  const qc = useQueryClient();
  const thinkingId = useId();
  const searchId = useId();
  const editRef = useRef<HTMLButtonElement>(null);
  // Live models are fetched the first time the menu opens, then stay cached.
  const [menuOpened, setMenuOpened] = useState(false);
  const live = useLiveModels(conn, menuOpened);

  useEffect(() => {
    if (focusEdit) editRef.current?.focus();
  }, [focusEdit]);

  const kind = info ? providerKind(info) : "custom";
  const levels = caps?.thinking_levels ?? [];
  const showSearch = !!caps?.web_search && config.web_search.mode !== "separate";

  const changeModel = async (model: string) => {
    if (model === conn.model) return;
    const next = await qc.fetchQuery(capabilitiesQuery(conn.provider, model)).catch(() => null);
    m.updateTranslation.mutate({
      ...conn,
      model,
      thinking: next ? normaliseThinking(conn.thinking, next) : conn.thinking,
    });
  };

  return (
    <>
      <div className="mt-3 flex items-center gap-3">
        <ProviderMark info={info} provider={conn.provider} size="lg" />
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-baseline gap-2">
            <span className="truncate text-[14.5px] font-semibold text-[color:var(--color-ink-emphasis)]">
              {providerName(info, conn)}
            </span>
            {kind === "local" ? (
              <span className="truncate font-mono text-[11px] text-[color:var(--color-ink-dim)]">
                {hostOf(conn.base_url)}
              </span>
            ) : kind === "custom" ? (
              <span className="shrink-0 text-[11px] text-[color:var(--color-ink-dim)]">
                custom endpoint
              </span>
            ) : null}
          </div>
          <div className="mt-1 flex min-w-0">
            <ModelPicker
              variant="inline"
              ariaLabel="Translation model"
              value={conn.model}
              onChange={(model) => void changeModel(model)}
              curated={info?.models ?? []}
              live={live.data}
              liveLoading={live.isFetching}
              sourceLabel={kind === "custom" ? "this server" : (info?.label ?? "the provider")}
              onOpenChange={(open) => {
                if (open) setMenuOpened(true);
              }}
            />
          </div>
        </div>
        <Button ref={editRef} variant="outline" size="sm" onClick={onEdit}>
          Edit
        </Button>
      </div>

      <div className="mt-3.5 mb-2 h-px bg-border" />

      {levels.length > 0 ? (
        <OptionRow labelId={thinkingId} label="Thinking" help={thinkingHelp(conn.thinking)}>
          <ThinkingControl
            labelledBy={thinkingId}
            levels={levels}
            value={conn.thinking}
            onChange={(thinking) => m.updateTranslation.mutate({ ...conn, thinking })}
          />
        </OptionRow>
      ) : null}
      {showSearch ? (
        <OptionRow
          labelId={searchId}
          label="Web search"
          help="Also look up established names for the glossary."
        >
          <Switch
            aria-labelledby={searchId}
            checked={config.web_search.mode === "translation"}
            onCheckedChange={(on) =>
              m.updateWebSearch.mutate(on ? { mode: "translation" } : { mode: "off" })
            }
          />
        </OptionRow>
      ) : null}
      {info ? (
        <TuningSection
          conn={conn}
          defaults={info.tuning}
          onSave={async (patch) => {
            try {
              await m.updateTranslation.mutateAsync({ ...conn, ...patch });
              toast.success("Tuning saved");
            } catch {
              // The mutation already rolled back and reported the error.
            }
          }}
        />
      ) : null}
    </>
  );
}

function OptionRow({
  labelId,
  label,
  help,
  children,
}: {
  labelId: string;
  label: string;
  help: string;
  children: ReactNode;
}) {
  return (
    <div className="flex min-h-10 flex-wrap items-center gap-x-3.5 gap-y-1 py-1">
      <span id={labelId} className="w-[92px] shrink-0 text-[12px] font-medium">
        {label}
      </span>
      {children}
      <span className="min-w-0 flex-1 basis-48 text-[11.5px] leading-4 text-muted-foreground">
        {help}
      </span>
    </div>
  );
}

type Feedback = "saving" | "saved" | null;

/** "Saving…" while an inline change is in flight, then "Saved" for a moment. */
function useSaveFeedback(pending: boolean, failed: boolean): Feedback {
  const [justSaved, setJustSaved] = useState(false);
  const wasPending = useRef(false);
  useEffect(() => {
    const finished = wasPending.current && !pending;
    wasPending.current = pending;
    if (!finished || failed) return;
    setJustSaved(true);
    const t = setTimeout(() => setJustSaved(false), 1600);
    return () => clearTimeout(t);
  }, [pending, failed]);
  return pending ? "saving" : justSaved ? "saved" : null;
}

function SaveFeedback({ state }: { state: Feedback }) {
  return (
    <span aria-live="polite" className="text-[11px] text-[color:var(--color-ink-dim)]">
      {state === "saving" ? (
        "Saving…"
      ) : state === "saved" ? (
        <span className="inline-flex items-center gap-1">
          <Check weight="bold" className="size-3" /> Saved
        </span>
      ) : null}
    </span>
  );
}
