import { useEffect, useId, useState } from "react";
import { CaretRight } from "@phosphor-icons/react";
import type { Connection } from "@/types/generated/Connection";
import type { TuningDefaults } from "@/types/generated/TuningDefaults";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { cn } from "@/lib/utils";
import { effectiveTuning, formatSeconds, TUNING_FIELDS, type TuningField } from "./connection-model";

const FIELDS: Record<TuningField, { label: string; help: string; min: number; max: number }> = {
  batch_dialogue_limit: {
    label: "Lines per request",
    help: "Subtitle lines sent in one go. Fewer is safer but slower.",
    min: 1,
    max: 1000,
  },
  concurrency: {
    label: "Requests at once",
    help: "More is faster, but can hit the provider's rate limits.",
    min: 1,
    max: 32,
  },
  timeout: {
    label: "Response timeout (s)",
    help: "How long to wait for each answer before retrying.",
    min: 10,
    max: 3600,
  },
  max_tokens: {
    label: "Max response tokens",
    help: "Longest answer allowed. Too low cuts long batches short.",
    min: 256,
    max: 1_000_000,
  },
};

type Draft = Record<TuningField, string>;

const toDraft = (v: TuningDefaults): Draft => ({
  batch_dialogue_limit: String(v.batch_dialogue_limit),
  concurrency: String(v.concurrency),
  timeout: String(v.timeout),
  max_tokens: String(v.max_tokens),
});

export function tuningSummary(v: TuningDefaults): string {
  return `${v.batch_dialogue_limit} lines/batch · ${v.concurrency} at once · ${formatSeconds(v.timeout)} timeout`;
}

/**
 * Collapsed by default behind a one-line summary of the current values. Explicit Save.
 * A value equal to the provider's default is stored as null, so it keeps following
 * the default.
 */
export function TuningSection({
  conn,
  defaults,
  onSave,
}: {
  conn: Connection;
  defaults: TuningDefaults;
  onSave: (patch: Pick<Connection, TuningField>) => Promise<unknown>;
}) {
  const baseId = useId();
  const effective = effectiveTuning(conn, defaults);
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState<Draft>(() => toDraft(effective));
  const [errors, setErrors] = useState<Partial<Record<TuningField, string>>>({});
  const [saving, setSaving] = useState(false);

  // When the saved values change (a save, or a new provider), start the draft from them.
  const savedKey = TUNING_FIELDS.map((f) => effective[f]).join("|");
  useEffect(() => {
    setDraft(toDraft(effective));
    setErrors({});
  }, [savedKey]);

  const dirty = TUNING_FIELDS.some((f) => draft[f].trim() !== String(effective[f]));
  const atDefaults = TUNING_FIELDS.every((f) => draft[f].trim() === String(defaults[f]));

  const save = async () => {
    const next: Partial<Record<TuningField, string>> = {};
    const patch = {} as Pick<Connection, TuningField>;
    for (const f of TUNING_FIELDS) {
      const { min, max } = FIELDS[f];
      const raw = draft[f].trim();
      const n = Number(raw);
      if (raw === "" || !Number.isInteger(n)) next[f] = "Enter a whole number.";
      else if (n < min || n > max) next[f] = `Between ${min.toLocaleString()} and ${max.toLocaleString()}.`;
      else patch[f] = n === defaults[f] ? null : n;
    }
    setErrors(next);
    if (Object.keys(next).length > 0) return;
    setSaving(true);
    try {
      await onSave(patch);
    } finally {
      setSaving(false);
    }
  };

  const panelId = `${baseId}-panel`;
  return (
    <div className="pt-1.5">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={panelId}
        onClick={() => setOpen((o) => !o)}
        className="-mx-1.5 flex max-w-full items-center gap-2 rounded-md px-1.5 py-1 text-[12px] text-muted-foreground outline-none hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50"
      >
        <CaretRight
          weight="bold"
          className={cn("size-3 shrink-0 transition-transform", open && "rotate-90")}
        />
        <span className="font-medium text-foreground">Tuning</span>
        <span className="truncate font-mono text-[11px] text-[color:var(--color-ink-dim)] tabular-nums">
          {tuningSummary(effective)}
        </span>
      </button>

      {open ? (
        <div id={panelId} className="mt-2.5">
          <div className="grid grid-cols-2 gap-x-4 gap-y-1">
            {TUNING_FIELDS.map((f) => {
              const id = `${baseId}-${f}`;
              const err = errors[f];
              return (
                <div key={f} className="flex min-w-0 flex-col">
                  <label htmlFor={id} className="mb-1 text-[11.5px] font-semibold">
                    {FIELDS[f].label}
                  </label>
                  <Input
                    id={id}
                    inputMode="numeric"
                    value={draft[f]}
                    aria-invalid={err ? true : undefined}
                    aria-describedby={`${id}-msg`}
                    onChange={(e) => {
                      setDraft((d) => ({ ...d, [f]: e.target.value }));
                      if (err) setErrors((x) => ({ ...x, [f]: undefined }));
                    }}
                    className="h-8 font-mono text-[12.5px] tabular-nums"
                  />
                  <p
                    id={`${id}-msg`}
                    className={cn(
                      "mt-1 min-h-8 text-[11px] leading-4",
                      err ? "text-[color:var(--color-coral)]" : "text-muted-foreground",
                    )}
                  >
                    {err ?? `${FIELDS[f].help} Default ${defaults[f].toLocaleString()}.`}
                  </p>
                </div>
              );
            })}
          </div>
          <div className="mt-1 flex items-center gap-2">
            <Button size="sm" disabled={!dirty || saving} onClick={() => void save()}>
              {saving ? <Spinner className="size-3.5" /> : null}
              Save tuning
            </Button>
            {!atDefaults ? (
              <Button
                size="sm"
                variant="ghost"
                className="text-muted-foreground"
                onClick={() => {
                  setDraft(toDraft(defaults));
                  setErrors({});
                }}
              >
                Use recommended values
              </Button>
            ) : null}
          </div>
        </div>
      ) : null}
    </div>
  );
}
