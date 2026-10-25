import { useEffect, useRef, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Check, Question, Warning } from "@phosphor-icons/react";
import { toast } from "sonner";
import type { PromptMeta } from "@/types/generated/PromptMeta";
import { ipc } from "@/lib/ipc";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { MOD } from "@/features/help/keys";
import { EditedMark } from "./prompt-list";

/** Case-tolerant token presence — mirrors the Rust validator. */
function tokenPresent(text: string, token: string): boolean {
  return text.includes(token.toLowerCase()) || text.includes(token.toUpperCase());
}

interface Props {
  meta: PromptMeta;
  /** Loaded (saved) text from the backend. */
  loaded: string;
  /** Unsaved draft, or null when the editor shows `loaded`. */
  draft: string | null;
  onDraftChange: (draft: string | null) => void;
  onSave: (text: string) => Promise<void>;
  /** Drop the unsaved draft, back to the saved text. */
  onDiscard: () => void;
  /** Back to the default at once; the page offers Undo (the saved text). Only
   *  offered without unsaved edits, so Undo brings back what the user had. */
  onReset: () => Promise<void>;
  /** Whether a save mutation is in flight — disables the Save button. */
  saving?: boolean;
}

export function PromptEditor({
  meta,
  loaded,
  draft,
  onDraftChange,
  onSave,
  onDiscard,
  onReset,
  saving = false,
}: Props) {
  const [helpOpen, setHelpOpen] = useState(false);
  const [resetting, setResetting] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const text = draft ?? loaded;
  const dirty = draft !== null && draft !== loaded;
  const missing = meta.placeholders.filter((p) => p.required && !tokenPresent(text, p.token));
  const canSave = dirty && missing.length === 0 && text.trim().length > 0;

  // Ctrl/Cmd+S saves, from the text box too. The latest state, without re-subscribing.
  const saveRef = useRef<() => void>(() => {});
  saveRef.current = () => {
    if (canSave && !saving) void onSave(text);
  };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey || e.shiftKey || e.key.toLowerCase() !== "s") return;
      e.preventDefault();
      if (!e.repeat) saveRef.current();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  /** Insert a placeholder token at the caret (replacing any selection). */
  const insertToken = (token: string) => {
    const el = textareaRef.current;
    const start = el?.selectionStart ?? text.length;
    const end = el?.selectionEnd ?? start;
    const next = text.slice(0, start) + token + text.slice(end);
    onDraftChange(next);
    requestAnimationFrame(() => {
      el?.focus();
      el?.setSelectionRange(start + token.length, start + token.length);
    });
  };

  const loadFromFile = async () => {
    try {
      const path = await openDialog({
        multiple: false,
        filters: [{ name: "Text", extensions: ["txt", "md", "prompt"] }],
      });
      if (typeof path !== "string") return;
      onDraftChange(await ipc.readPromptFile(path));
    } catch (e) {
      toast.error(String(e));
    }
  };

  const restoreButton = (
    <Button
      variant="outline"
      size="sm"
      disabled={!meta.modified || dirty || resetting}
      onClick={async () => {
        setResetting(true);
        try {
          await onReset();
        } finally {
          setResetting(false);
        }
      }}
    >
      Restore default
    </Button>
  );

  return (
    <div className="flex min-w-0 flex-1 flex-col gap-3 p-4">
      <div className="flex items-center justify-between">
        <h2 className="text-[15px] font-medium">{meta.name}</h2>
        {meta.modified ? <EditedMark /> : null}
      </div>

      {meta.placeholders.length ? (
        <div className="flex flex-wrap items-center gap-1.5 text-[12px]">
          <span className="text-muted-foreground">Placeholders:</span>
          {meta.placeholders.map((p) => {
            const present = tokenPresent(text, p.token);
            return (
              <button
                key={p.token}
                type="button"
                onClick={() => insertToken(p.token)}
                title={`${p.description}${p.required ? "" : " (optional)"} — click to insert at the caret`}
                className={cn(
                  "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 font-mono text-[11px]",
                  present
                    ? "border-[color:var(--color-success)] text-[color:var(--color-success)]"
                    : p.required
                      ? "border-[color:var(--color-alert)] text-[color:var(--color-alert)]"
                      : "border-border text-muted-foreground",
                )}
              >
                {present ? <Check className="size-3" /> : null}
                {p.token}
              </button>
            );
          })}
          <button
            type="button"
            onClick={() => setHelpOpen((v) => !v)}
            aria-label="What do these tags do?"
            aria-expanded={helpOpen}
            className="inline-flex size-5 items-center justify-center rounded-full border border-border text-muted-foreground hover:bg-[color:var(--color-bg-hover)]"
          >
            <Question className="size-3.5" />
          </button>
        </div>
      ) : null}

      {helpOpen && meta.placeholders.length ? (
        <div className="rounded-md border border-border bg-[color:var(--popover)] p-3 text-[12px]">
          <p className="mb-1.5 font-medium">
            What these tags do — they are replaced at runtime before the prompt is sent:
          </p>
          <ul className="space-y-1">
            {meta.placeholders.map((p) => (
              <li key={p.token}>
                <code className="font-mono text-[11px]">{p.token}</code>
                {p.required ? null : <span className="text-muted-foreground"> (optional)</span>}
                {" — "}
                {p.description}
              </li>
            ))}
          </ul>
          <p className="mt-1.5 text-muted-foreground">
            Tags match in ALL-CAPS or all-lowercase form. Required tags must appear at least once —
            Save is disabled otherwise.
          </p>
        </div>
      ) : null}

      <Textarea
        ref={textareaRef}
        value={text}
        onChange={(e) => onDraftChange(e.target.value)}
        spellCheck={false}
        aria-label={meta.name}
        className="min-h-0 flex-1 resize-none font-mono text-[12px] leading-relaxed"
      />

      {missing.length ? (
        <p className="flex items-center gap-1.5 text-[12px] text-[color:var(--color-alert)]">
          <Warning className="size-3.5" />
          Missing required placeholder{missing.length > 1 ? "s" : ""}:{" "}
          {missing.map((p) => p.token).join(", ")} — add {missing.length > 1 ? "them" : "it"} back
          to save.
        </p>
      ) : null}

      <div className="flex items-center justify-between gap-3">
        <span className="text-[11px] text-muted-foreground">
          Changes apply to the next run. {MOD}S saves.
        </span>
        <div className="flex shrink-0 gap-2">
          {dirty ? (
            <Button variant="ghost" size="sm" disabled={saving} onClick={onDiscard}>
              Discard edits
            </Button>
          ) : null}
          <Button variant="outline" size="sm" onClick={() => void loadFromFile()}>
            Load from file…
          </Button>
          {/* Undo brings back the saved text, so unsaved typing has to be saved
              or discarded first: Restore default would lose it for good. */}
          {dirty && meta.modified ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="inline-flex" tabIndex={0}>
                  {restoreButton}
                </span>
              </TooltipTrigger>
              <TooltipContent side="top">Save or discard your edits first.</TooltipContent>
            </Tooltip>
          ) : (
            restoreButton
          )}
          <Button size="sm" disabled={!canSave || saving} onClick={() => void onSave(text)}>
            Save
          </Button>
        </div>
      </div>
    </div>
  );
}
