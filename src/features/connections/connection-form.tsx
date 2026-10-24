import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ArrowClockwise, Eye, EyeSlash, XCircle } from "@phosphor-icons/react";
import type { Model } from "@/types/generated/Model";
import type { Provider } from "@/types/generated/Provider";
import type { ProviderInfo } from "@/types/generated/ProviderInfo";
import type { TestResult } from "@/types/generated/TestResult";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useDebouncedValue } from "@/hooks/use-debounced-value";
import { ipc } from "@/lib/ipc";
import {
  findProvider,
  hostOf,
  isHttpUrl,
  maskKey,
  modelLabel,
  newModel,
  providerKind,
  type ProviderKind,
  type Role,
} from "./connection-model";
import { commandFailure, problemCopy, problemDetails, type FieldKey } from "./error-copy";
import { ConnectButton, Details, Field, KeyLink, Msg, TextButton, type Phase } from "./form-parts";
import { ModelPicker } from "./model-picker";
import { ChosenProvider, ProviderTiles } from "./provider-tiles";
import { recordWorks } from "./test-status";
import { capabilitiesQuery, useCapabilities, useLiveModels } from "./use-connections";

type Errors = Partial<Record<FieldKey, string>>;
export type FormStage = "pick" | "fields";

const COPY: Record<Role, { heading: string; lead: string; remove: string }> = {
  model: {
    heading: "Connect an AI model",
    lead: "Pick a provider. You’ll need an API key from them (local models don’t need one).",
    remove: "Remove model",
  },
  "web-search": {
    heading: "Which AI should look up names online?",
    lead: "It finds the names fans already use. Anthropic, OpenAI and Google can search the web.",
    remove: "Remove this model",
  },
};

const FOOT_HINT: Record<ProviderKind, string> = {
  cloud: "Sends one tiny test message, then saves.",
  local: "Local models get smaller batches and longer timeouts automatically.",
  custom: "Sends one tiny test message and detects the API format, then saves.",
};

/**
 * The setup / edit form for one library model (first run, Edit, adding a model): provider
 * tiles → one chosen row → only the fields that provider needs → Connect (test, then save).
 * It only knows where to reach the model; Thinking and Tuning live on the steps.
 */
export function ConnectionForm({
  role,
  providers,
  providerKeys,
  initial,
  onSave,
  onCancel,
  onRemove,
  onStageChange,
  autoFocus = false,
  heading,
  lead,
}: {
  role: Role;
  providers: ProviderInfo[];
  providerKeys: Record<string, string>;
  /** The saved model when editing (its id is kept, even across a provider change); null to add. */
  initial: Model | null;
  /** Persist the model; `tested` is false for "Save anyway". Throwing keeps the form open. */
  onSave: (model: Model, tested: boolean) => Promise<void>;
  onCancel?: () => void;
  /** Edit mode only: the last control on the form. */
  onRemove?: () => void;
  onStageChange?: (stage: FormStage) => void;
  /** Focus the first provider tile on mount (the form was opened by a click). */
  autoFocus?: boolean;
  /** Overrides the role's heading above the provider tiles. */
  heading?: string;
  /** Overrides the role's lead line under the heading. */
  lead?: string;
}) {
  const qc = useQueryClient();
  const uid = useId();
  const [draft, setDraft] = useState<Model | null>(initial);
  const [keyMode, setKeyMode] = useState<"saved" | "edit">(initial?.api_key ? "saved" : "edit");
  const [reveal, setReveal] = useState(false);
  const [phase, setPhase] = useState<Phase>("idle");
  const [failure, setFailure] = useState<TestResult | null>(null);
  const [errors, setErrors] = useState<Errors>({});
  // Editing opens straight on the fields, so focus lands there instead of being dropped.
  const [focusTo, setFocusTo] = useState<"first" | "key" | "edit" | null>(initial ? "edit" : null);
  const [returnTo, setReturnTo] = useState<Provider | undefined>(() =>
    autoFocus ? (role === "web-search" ? providers.find((p) => p.can_search) : providers[0])?.provider : undefined,
  );
  const keyRef = useRef<HTMLInputElement>(null);
  const addressRef = useRef<HTMLInputElement>(null);
  const modelRef = useRef<HTMLButtonElement>(null);
  const connectRef = useRef<HTMLButtonElement>(null);

  const info = draft ? findProvider(providers, draft.provider) : undefined;
  const kind: ProviderKind = info ? providerKind(info) : "cloud";
  const choices = role === "web-search" ? providers.filter((p) => p.can_search) : providers;
  const stage: FormStage = draft && info ? "fields" : "pick";

  useEffect(() => onStageChange?.(stage), [stage, onStageChange]);

  // Live models: debounced while typing an address or key, then fetched once it settles.
  const probeSource = draft ? `${draft.provider}\n${draft.base_url.trim()}\n${draft.api_key.trim()}` : "";
  const settled = useDebouncedValue(probeSource, 500) === probeSource;
  const probe = draft && settled ? { ...draft, base_url: draft.base_url.trim(), api_key: draft.api_key.trim() } : null;
  const canProbe = !!probe && (kind === "cloud" ? probe.api_key !== "" : isHttpUrl(probe.base_url));
  // Cloud/custom only use the list to enrich the curated models, so a failure is ignored there.
  const live = useLiveModels(probe, canProbe);
  // Local: is the server there, and does it have any models? (Retry keeps the last answer
  // in `data`, so a failed refetch must win over it.)
  const server: "checking" | "unreachable" | "empty" | "found" =
    !settled || live.isFetching
      ? "checking"
      : live.isError
        ? "unreachable"
        : !live.data
          ? "checking"
          : live.data.length === 0
            ? "empty"
            : "found";

  // Local: preselect the first model the server reports.
  useEffect(() => {
    const first = live.data?.[0];
    if (kind === "local" && first) setDraft((d) => (d && !d.model ? { ...d, model: first } : d));
  }, [live.data, kind]);

  // Web search needs a model that can search.
  const caps = useCapabilities(role === "web-search" ? draft?.provider : undefined, draft?.model);
  const cantSearch = role === "web-search" && caps.data && !caps.isPlaceholderData && !caps.data.web_search;

  // Move focus somewhere sensible after the tiles collapse or "Replace" is pressed.
  useEffect(() => {
    if (!focusTo || !draft) return;
    if (focusTo === "key") keyRef.current?.focus();
    else if (focusTo === "edit") (kind === "custom" ? addressRef : modelRef).current?.focus();
    else if (kind === "cloud" && !draft.api_key) keyRef.current?.focus();
    else if (kind === "custom") addressRef.current?.focus();
    else connectRef.current?.focus();
    setFocusTo(null);
  }, [focusTo, draft, kind]);

  const savedKeyFor = (p: Provider) =>
    initial?.provider === p && initial.api_key ? initial.api_key : (providerKeys[p] ?? "");

  const resetFeedback = () => {
    setFailure(null);
    setErrors({});
  };

  const pick = (next: ProviderInfo) => {
    // Editing keeps the library entry (id, colour) even when the provider changes.
    const base =
      initial?.provider === next.provider
        ? initial
        : { ...newModel(next, providerKeys), id: initial?.id ?? "", color: initial?.color ?? 0 };
    setDraft(base);
    setKeyMode(base.api_key ? "saved" : "edit");
    setReveal(false);
    resetFeedback();
    setFocusTo("first");
  };

  const change = () => {
    setReturnTo(draft?.provider);
    setDraft(null);
    resetFeedback();
  };

  const update = (patch: Partial<Model>, field: FieldKey) => {
    setDraft((d) => (d ? { ...d, ...patch } : d));
    setFailure(null);
    setErrors((e) => ({ ...e, [field]: undefined }));
  };

  const connect = async (skipTest: boolean) => {
    if (!draft || !info) return;
    const conn: Model = {
      ...draft,
      base_url: draft.base_url.trim(),
      api_key: draft.api_key.trim(),
      model: draft.model.trim(),
    };
    const found = validate(conn, info, kind);
    if (!found.model && role === "web-search") {
      const c = await qc.fetchQuery(capabilitiesQuery(conn.provider, conn.model)).catch(() => null);
      if (c && !c.web_search) found.model = cantSearchText(info, conn.model);
    }
    setErrors(found);
    setFailure(null);
    const firstBad = (["key", "address", "model"] as const).find((f) => found[f]);
    if (firstBad) {
      ({ key: keyRef, address: addressRef, model: modelRef })[firstBad].current?.focus();
      return;
    }

    if (!skipTest) {
      setPhase("testing");
      const started = performance.now();
      let result: TestResult;
      try {
        // A web-search model is only worth saving if its web-search tool works too.
        result = await ipc.testConnection(conn, role === "web-search" ? true : undefined);
      } catch (e) {
        result = commandFailure(conn, e);
      }
      if (!result.ok) {
        setFailure(result);
        setPhase("idle");
        return;
      }
      // A passing test is what makes a model read "Connected" this session.
      recordWorks(conn, (performance.now() - started) / 1000);
      // Custom endpoints: use the API format the test detected.
      if (result.detected_driver) conn.driver = result.detected_driver;
    }

    setPhase("saving");
    try {
      await onSave(conn, !skipTest);
    } catch (e) {
      setFailure(commandFailure(conn, e));
    }
    setPhase("idle");
  };

  // ── pick stage ─────────────────────────────────────────────────────────────
  if (stage === "pick") {
    return (
      <div>
        <h3 className="mt-3 text-[15px] font-semibold text-[color:var(--color-ink-emphasis)]">
          {heading ?? COPY[role].heading}
        </h3>
        <p className="mt-0.5 mb-3 text-[12px] text-muted-foreground">{lead ?? COPY[role].lead}</p>
        <ProviderTiles providers={choices} onPick={pick} focus={returnTo} />
        {onCancel ? (
          <div className="mt-3">
            <Button type="button" variant="ghost" onClick={onCancel}>
              Cancel
            </Button>
          </div>
        ) : null}
      </div>
    );
  }

  // ── fields stage ───────────────────────────────────────────────────────────
  const d = draft as Model;
  const p = info as ProviderInfo;
  const problem = failure ? problemCopy(failure, p, d) : null;
  const fieldError = (f: FieldKey) => errors[f] ?? (problem?.field === f ? problem.text : undefined);
  const cardError = problem && problem.field === null ? problem.text : undefined;
  const details = failure && problem ? problemDetails(failure, problem.text) : null;
  const busy = phase !== "idle";
  const ids = { key: `${uid}-key`, address: `${uid}-address`, model: `${uid}-model` };

  const savedKey = savedKeyFor(d.provider);
  const keyField = (
    <Field
      id={ids.key}
      label={kind === "custom" ? "API key (optional)" : "API key"}
      aside={p.key_url ? <KeyLink url={p.key_url} /> : null}
      message={
        fieldError("key") ? (
          <Msg tone="error">{fieldError("key")}</Msg>
        ) : keyMode === "saved" && d.api_key ? (
          <Msg tone="help">
            {initial?.provider === d.provider
              ? "Stored only on this computer."
              : `Using the key you saved for ${p.label}. Stored only on this computer.`}
          </Msg>
        ) : (
          <Msg tone="help">
            Stored only on this computer.
            {savedKey ? (
              <>
                {" "}
                <TextButton
                  onClick={() => {
                    setKeyMode("saved");
                    update({ api_key: savedKey }, "key");
                  }}
                >
                  Keep saved key
                </TextButton>
              </>
            ) : null}
          </Msg>
        )
      }
    >
      {keyMode === "saved" && d.api_key ? (
        <div className="flex h-9 items-center gap-2 rounded-4xl border border-input bg-input/30 pr-1 pl-3">
          <span className="min-w-0 flex-1 truncate font-mono text-[12.5px]" title="Saved key">
            {maskKey(d.api_key)}
          </span>
          <Button
            type="button"
            size="xs"
            variant="ghost"
            className="text-primary"
            aria-label="Replace saved key"
            disabled={busy}
            onClick={() => {
              setKeyMode("edit");
              update({ api_key: "" }, "key");
              setFocusTo("key");
            }}
          >
            Replace
          </Button>
        </div>
      ) : (
        <div className="relative">
          <Input
            ref={keyRef}
            id={ids.key}
            type={reveal ? "text" : "password"}
            value={d.api_key}
            readOnly={busy}
            onChange={(e) => update({ api_key: e.target.value }, "key")}
            placeholder={keyPlaceholder(p)}
            autoComplete="off"
            spellCheck={false}
            aria-invalid={fieldError("key") ? true : undefined}
            aria-describedby={`${ids.key}-msg`}
            className="pr-10 font-mono text-[12.5px]"
          />
          <button
            type="button"
            aria-label={reveal ? "Hide key" : "Show key"}
            aria-pressed={reveal}
            onClick={() => setReveal((r) => !r)}
            className="absolute top-1/2 right-1.5 grid size-7 -translate-y-1/2 place-items-center rounded-full text-muted-foreground outline-none hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50"
          >
            {reveal ? <EyeSlash className="size-4" /> : <Eye className="size-4" />}
          </button>
        </div>
      )}
    </Field>
  );

  const retry = (
    <TextButton onClick={() => void live.refetch()}>
      <ArrowClockwise weight="bold" className="size-3" /> Retry
    </TextButton>
  );
  const modelCount = live.data?.length ?? 0;
  const addressField = (
    <Field
      id={ids.address}
      label={kind === "local" ? "Server address" : "Endpoint URL"}
      message={
        fieldError("address") ? (
          <Msg tone="error">{fieldError("address")}</Msg>
        ) : kind === "custom" ? (
          <Msg tone="help">The API format is detected when you connect.</Msg>
        ) : !isHttpUrl(d.base_url.trim()) ? (
          <Msg tone="help">Usually {p.base_url}</Msg>
        ) : server === "checking" ? (
          <Msg tone="busy">Checking {hostOf(d.base_url)}…</Msg>
        ) : server === "found" ? (
          <Msg tone="ok">
            {p.label} is running. Found {modelCount} {modelCount === 1 ? "model" : "models"}.
          </Msg>
        ) : server === "empty" ? (
          <Msg tone="warn">
            {p.label} is running, but no models are installed yet. {noModelsHint(p)} {retry}
          </Msg>
        ) : (
          <Msg tone="warn">
            Can’t reach {p.label} at {hostOf(d.base_url)} — is it running? {retry}
          </Msg>
        )
      }
    >
      <Input
        ref={addressRef}
        id={ids.address}
        value={d.base_url}
        readOnly={busy}
        onChange={(e) => update({ base_url: e.target.value }, "address")}
        placeholder={kind === "custom" ? "https://api.example.com/v1" : p.base_url}
        autoComplete="off"
        spellCheck={false}
        aria-invalid={fieldError("address") ? true : undefined}
        aria-describedby={`${ids.address}-msg`}
        className="font-mono text-[12.5px]"
      />
    </Field>
  );

  const modelErr = fieldError("model");
  const hint = kind === "cloud" ? modelHint(p, d.model) : null;
  const modelField = (
    <Field
      id={ids.model}
      label="Model"
      message={
        modelErr ? (
          <Msg tone="error">{modelErr}</Msg>
        ) : cantSearch ? (
          <Msg tone="warn">{cantSearchText(p, d.model)}</Msg>
        ) : kind === "cloud" ? (
          hint ? <Msg tone="help">{hint}</Msg> : null
        ) : (
          <Msg tone="help">Not listed? Open the menu and type its name.</Msg>
        )
      }
    >
      <ModelPicker
        ref={modelRef}
        id={ids.model}
        value={d.model}
        onChange={(model) => update({ model }, "model")}
        curated={p.models}
        live={live.data}
        liveLoading={canProbe && live.isFetching}
        sourceLabel={kind === "custom" ? "this server" : p.label}
        invalid={!!modelErr}
        disabled={busy}
        describedBy={`${ids.model}-msg`}
      />
    </Field>
  );

  return (
    <form
      noValidate
      onSubmit={(e) => {
        e.preventDefault();
        if (!busy) void connect(false);
      }}
    >
      <div className="mt-3 mb-3.5">
        <ChosenProvider info={p} onChange={change} disabled={busy} />
      </div>

      <div className="grid grid-cols-[minmax(0,1.2fr)_minmax(0,1fr)] gap-x-3 gap-y-1">
        {kind === "cloud" ? (
          <>
            {keyField}
            {modelField}
          </>
        ) : kind === "local" ? (
          <>
            {addressField}
            {modelField}
          </>
        ) : (
          <>
            {addressField}
            {keyField}
            {modelField}
          </>
        )}
      </div>

      <div className="mt-2 flex items-center gap-2">
        <ConnectButton ref={connectRef} phase={phase} retry={!!failure} />
        {failure?.can_save_anyway && !busy ? (
          <Button type="button" variant="outline" onClick={() => void connect(true)}>
            Save anyway
          </Button>
        ) : null}
        {onCancel ? (
          <Button type="button" variant="ghost" disabled={busy} onClick={onCancel}>
            Cancel
          </Button>
        ) : null}
        <div aria-live="polite" className="ml-1 flex min-h-9 min-w-0 flex-1 items-center">
          {cardError ? (
            <p
              title={cardError}
              className="flex items-start gap-1.5 text-[11.5px] leading-4 text-[color:var(--color-coral)] select-text"
            >
              <XCircle weight="fill" className="mt-px size-3.5 shrink-0" />
              <span className="line-clamp-2">{cardError}</span>
            </p>
          ) : (
            <p className="line-clamp-2 text-[11.5px] leading-4 text-muted-foreground">
              {FOOT_HINT[kind]}
            </p>
          )}
        </div>
      </div>
      {details ? <Details text={details} className="mt-1" /> : null}

      {onRemove ? (
        <div className="mt-4 border-t border-border pt-3">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={busy}
            className="-ml-2 text-[color:var(--color-coral)] hover:bg-[color-mix(in_oklch,var(--color-coral)_10%,transparent)] hover:text-[color:var(--color-coral)]"
            onClick={onRemove}
          >
            {COPY[role].remove}
          </Button>
        </div>
      ) : null}
    </form>
  );
}

// ── helpers ────────────────────────────────────────────────────────────────────

function validate(conn: Model, info: ProviderInfo, kind: ProviderKind): Errors {
  const e: Errors = {};
  if (kind === "cloud" && !conn.api_key) e.key = `Paste your ${info.label} API key.`;
  if (kind !== "cloud") {
    if (!conn.base_url) {
      e.address = kind === "local" ? "Enter the server address." : "Enter the endpoint URL.";
    } else if (!isHttpUrl(conn.base_url)) {
      e.address = "This should be a web address starting with http:// or https://.";
    }
  }
  if (!conn.model) {
    e.model = kind === "cloud" ? "Choose a model." : "Choose a model, or type its name in the menu.";
  }
  return e;
}

/** What to do when a local server answers with no models. */
function noModelsHint(info: ProviderInfo): ReactNode {
  if (info.provider === "ollama") {
    return (
      <>
        Run{" "}
        <code className="font-mono text-[11px] whitespace-nowrap text-foreground">
          ollama pull &lt;model&gt;
        </code>{" "}
        and press Retry.
      </>
    );
  }
  return `Download a model in ${info.label}, then Retry.`;
}

function cantSearchText(info: ProviderInfo, model: string): string {
  return `${modelLabel(info, model)} can’t search the web. Pick another model.`;
}

/** "Anthropic keys start with sk-ant-" → "sk-ant-…". */
function keyPlaceholder(info: ProviderInfo): string {
  const prefix = /start with (\S+)$/.exec(info.key_hint ?? "")?.[1];
  return prefix ? `${prefix}…` : "Paste your API key";
}

/** "Opus 5.5 = best quality · Haiku 5.5 = fastest & cheapest" — the other curated picks. */
function modelHint(info: ProviderInfo, value: string): string | null {
  const labels = info.models.map((m) => m.label);
  const family = labels[0]?.split(" ")[0];
  const shared = !!family && labels.every((l) => l.startsWith(`${family} `));
  const short = (label: string) => {
    const rest = label.slice(family!.length + 1);
    return shared && /^[A-Za-z]/.test(rest) ? rest : label;
  };
  const others = info.models.filter((m) => m.note && m.id.toLowerCase() !== value.toLowerCase());
  if (others.length === 0) return null;
  return others.map((m) => `${short(m.label)} = ${m.note!.toLowerCase()}`).join(" · ");
}
