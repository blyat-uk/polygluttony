import { Fragment, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { CaretDown, CaretRight, Play, Stop } from "@phosphor-icons/react";
import { stateToCells, StatusGlyph, IssuePanel } from "./file-cells"
import { BatchCell } from "./batch-cell"
import { ReactorBar } from "./reactor-bar"
import { RunIntegrityRing } from "./run-integrity-ring"
import { batchCellState, pickHero, runIntegrity } from "./telemetry";
import { LogToggleButton, LogPanel } from "@/components/log-drawer";
import type { Tone } from "@/types/generated/Tone";
import { ipc } from "@/lib/ipc";
import { useAppStore } from "@/stores/app-store";
import { useProject, syncProjectStore, projectKey } from "@/features/project/use-project";
import { useTranslationRun } from "@/stores/translation-store";
import type { FileStateKind } from "@/types/generated/FileStateKind";
import { PageHeader } from "@/components/page-header";
import { Button } from "@/components/ui/button";
import { HelpText } from "@/components/help-text";
import { SectionHelp } from "@/components/section-help";

// ── constants ────────────────────────────────────────────────────────────────

const TONES: Tone[] = ["standard", "xianxia", "wuxia", "comedic", "funny"];

const SELECT_CLS =
  "h-9 rounded-md border border-input bg-[color:var(--card)] px-2 text-sm";

/** Lines-per-minute heuristic for the estimate banner. */
const LINES_PER_MINUTE = 600;

/** Approx line range label for batch i, given the file's total lines + batch count. */
function batchRange(i: number, row: { total: number; totalBatches: number }): string {
  if (!row.totalBatches || !row.total) return ""
  const per = Math.ceil(row.total / row.totalBatches)
  const start = i * per + 1
  const end = Math.min((i + 1) * per, row.total)
  return `lines ${start}–${end}`
}

// ── state chip helpers ────────────────────────────────────────────────────────

const STATE_LABEL: Record<FileStateKind, string> = {
  pending: "Waiting",
  translating: "Translating…",
  retranslating: "Retranslating…",
  cleanup: "Cleaning up…",
  verifying: "Verifying…",
  done: "Done",
  warning: "Needs a look",
  failed: "Failed",
};

const STATE_COLOR: Record<FileStateKind, string> = {
  pending: "var(--muted-foreground)",
  translating: "var(--primary)",
  retranslating: "var(--color-alert)",
  cleanup: "var(--color-state-cleanup)",
  verifying: "var(--color-state-verify)",
  done: "var(--color-success)",
  warning: "var(--color-alert)",
  failed: "var(--color-danger)",
};

/** Whether the state should pulse to indicate live activity. */
const STATE_ANIMATE = new Set<FileStateKind>([
  "translating",
  "retranslating",
  "cleanup",
  "verifying",
]);

function FileStateChip({ state }: { state: FileStateKind }) {
  const color = STATE_COLOR[state] ?? STATE_COLOR.pending;
  const animate = STATE_ANIMATE.has(state);
  return (
    <span className="inline-flex items-center gap-1.5 rounded-[13px] border border-border px-2.5 py-0.5 text-[11px] shrink-0">
      <span
        className={`size-2 rounded-full${animate ? " animate-pulse" : ""}`}
        style={{ background: color }}
      />
      {STATE_LABEL[state]}
    </span>
  );
}

// ── main component ────────────────────────────────────────────────────────────

export function TranslatePage() {
  const workdir = useAppStore((s) => s.workdir);
  const translationLabel = useAppStore((s) => s.translationLabel);
  const hasUsableConnection = useAppStore((s) => s.hasUsableConnection);
  const qc = useQueryClient();

  const { data: view } = useProject(workdir ?? "");

  const running = useTranslationRun((s) => s.running);
  const storeFiles = useTranslationRun((s) => s.files);
  // The hero card spotlights one file's live batches. It auto-follows the first
  // in-flight file, but the user can pin a different file by clicking its row.
  const [selectedHero, setSelectedHero] = useState<string | null>(null);
  const heroName = running ? (selectedHero ?? pickHero(storeFiles)) : null;
  const hero = heroName ? storeFiles[heroName] : null;
  const integ = runIntegrity(storeFiles);
  const logs = useTranslationRun((s) => s.logs);
  const results = useTranslationRun((s) => s.results);

  const [logsOpen, setLogsOpen] = useState(false);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const toggleExpand = (name: string) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });

  if (!workdir || !view) return null;

  const prefs = view.prefs;
  const selected = prefs.selected_files;
  const sourceLang = prefs.source_lang;
  const targetLang = prefs.target_lang;
  const tone = prefs.tone;

  // ── persist helpers ─────────────────────────────────────────────────────────

  const persistTone = (next: Tone) => {
    const nextPrefs = { ...prefs, tone: next };
    void ipc.saveFolderPrefs(view.folder, nextPrefs);
    qc.setQueryData(projectKey(view.folder), { ...view, prefs: nextPrefs });
    syncProjectStore({ ...view, prefs: nextPrefs });
  };

  // ── estimate banner ──────────────────────────────────────────────────────────

  const selectedFiles = view.files.filter((f) => selected.includes(f.name));
  const totalLines = selectedFiles.reduce((s, f) => s + f.dialogue_count, 0);
  const estimateMinutes = Math.ceil(totalLines / LINES_PER_MINUTE);

  const glossaryClause =
    view.glossary_terms != null
      ? ` · using glossary (${view.glossary_terms} terms)`
      : "";

  const connLabel = translationLabel ?? "no model";

  // ── actions ──────────────────────────────────────────────────────────────────

  const canStart = !running && selected.length > 0 && hasUsableConnection;

  const handleStart = () => {
    setExpanded(new Set());
    setSelectedHero(null);
    useTranslationRun.getState().start(selected);
    ipc.startTranslation({
      folder: workdir,
      files: selected,
      tone,
      sourceLang,
      targetLang,
    }).catch((err: unknown) => {
      const store = useTranslationRun.getState();
      store.reset();
      store.applyEvent({
        kind: "log",
        file: null,
        level: "error",
        phase: "error",
        message: err instanceof Error ? err.message : String(err),
      });
    });
  };

  const handleCancel = () => {
    void ipc.cancelTranslation();
  };

  // ── completion summary ────────────────────────────────────────────────────────

  let summaryEl: React.ReactNode = null;
  if (results !== null) {
    // Only files that actually succeeded count as "translated"/"clean". Failed
    // or cancelled files have success=false (and has_warnings=false), so they
    // must NOT be folded into "clean".
    const succeeded = results.filter((r) => r.success);
    const failed = results.length - succeeded.length;
    const clean = succeeded.filter((r) => !r.has_warnings).length;
    const warn = succeeded.length - clean;
    const toneCls =
      failed > 0
        ? "border-[color:var(--color-danger)]/30 bg-[color:var(--color-danger)]/5"
        : warn > 0
          ? "border-[color:var(--color-alert)]/30 bg-[color:var(--color-alert)]/5"
          : "border-[color:var(--color-success)]/30 bg-[color:var(--color-success)]/5";
    summaryEl = (
      <div className={`rounded-md border ${toneCls} px-4 py-2.5 text-sm`}>
        {succeeded.length > 0 ? (
          <>
            <span className="font-medium text-foreground">
              Translated {succeeded.length} file{succeeded.length !== 1 ? "s" : ""} —{" "}
            </span>
            <span className="text-[color:var(--color-success)]">{clean} clean</span>
            {warn > 0 ? (
              <>
                <span className="text-muted-foreground">, </span>
                <span className="text-[color:var(--color-alert)]">{warn} need a look</span>
              </>
            ) : null}
            {failed > 0 ? (
              <>
                <span className="text-muted-foreground">, </span>
                <span className="text-[color:var(--color-danger)]">{failed} failed or cancelled</span>
              </>
            ) : null}
          </>
        ) : (
          <>
            <span className="font-medium text-foreground">No files completed — </span>
            <span className="text-[color:var(--color-danger)]">{failed} failed or cancelled</span>
          </>
        )}
        <span className="text-muted-foreground">.</span>
      </div>
    );
  }

  // ── file table rows ───────────────────────────────────────────────────────────

  // Merge store state with static file list.
  const tableRows = selectedFiles.map((f) => {
    const row = storeFiles[f.name];
    const result = results?.find((r) => r.file === f.name);

    let state: FileStateKind = "pending";
    if (result) {
      state = !result.success ? "failed" : result.has_warnings ? "warning" : "done";
    } else if (row) {
      state = row.state;
    }

    return { file: f, row, state, result };
  });

  // ── render ────────────────────────────────────────────────────────────────────

  return (
    <div className="flex h-full flex-col">
      {/* Header */}
      <PageHeader
        title="Translate"
        description="Pick files, see an estimate, then run the pipeline."
        actions={
          <div className="flex items-center gap-2 shrink-0">
            <label className="text-[11.5px] font-semibold text-muted-foreground" htmlFor="tone-select-translate">
              Tone
            </label>
            <select
              id="tone-select-translate"
              className={SELECT_CLS}
              value={tone}
              onChange={(e) => persistTone(e.target.value as Tone)}
            >
              {[...TONES].sort().map((t) => (
                <option key={t} value={t}>
                  {t}
                </option>
              ))}
            </select>
          </div>
        }
      />

      {/* Scrollable body. Plain block (not a flex column): a flex child with
          overflow-hidden — the table wrapper — would otherwise be shrunk to fit
          and clip its rows instead of letting the body scroll. */}
      <div className="min-h-0 flex-1 overflow-auto p-5 space-y-4">

        {/* Estimate banner */}
        {selected.length > 0 ? (
          <div className="rounded-md border border-primary/30 bg-primary/5 px-4 py-2.5 text-[12.5px] text-foreground">
            <span className="tabular-nums">
              {selectedFiles.length} file{selectedFiles.length !== 1 ? "s" : ""} · {totalLines.toLocaleString()} lines
              {glossaryClause} · est.{" "}
              <span className="font-medium">~{estimateMinutes}m</span> on{" "}
              <span className="font-medium text-primary">{connLabel}</span>.
            </span>
          </div>
        ) : (
          <div className="rounded-md border border-border bg-[color:var(--card)] px-4 py-2.5 text-[12.5px] text-muted-foreground">
            Select files below to see an estimate.
          </div>
        )}

        <HelpText>
          Polygluttony translates each file, then checks its own work. While a run is active, click any
          file to watch its batches above. Files that need a look turn amber — click one to see the exact issues.
        </HelpText>

        {/* Completion summary */}
        {summaryEl}

        {/* Hero batch card (active file during a run) */}
        {hero && heroName ? (
          <div className="relative overflow-hidden rounded-xl border border-[color:color-mix(in_oklch,var(--color-gold)_22%,transparent)] bg-[linear-gradient(180deg,rgba(225,166,54,.05),rgba(225,166,54,.015))] p-4">
            <div className="mb-3 flex items-baseline justify-between">
              <span className="font-mono text-[14px] text-[color:var(--color-ink-emphasis)]">{heroName}</span>
              <span className="text-[11px] text-muted-foreground tabular-nums">
                {hero.total > 0 ? <><b className="font-semibold text-[color:var(--color-gold-hi)]">{hero.translated.toLocaleString()}</b>/{hero.total.toLocaleString()} translated</> : "starting…"}
              </span>
            </div>
            <div className="grid grid-cols-[1fr_158px] items-stretch gap-5">
              <div className="flex flex-col gap-2.5">
                {Array.from({ length: Math.max(hero.totalBatches, 1) }, (_, i) => (
                  <BatchCell key={i} index={i} range={batchRange(i, hero)} state={batchCellState(hero, i)} since={hero.inFlightSince} />
                ))}
              </div>
              <RunIntegrityRing done={integ.done} total={integ.total} retranslated={integ.retranslated} />
            </div>
          </div>
        ) : null}

        {/* File table */}
        <div className="rounded-md border border-border overflow-hidden">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-border bg-[color:var(--color-bg-raised)] text-[11px] text-muted-foreground uppercase tracking-wide">
                <th className="px-4 py-2 text-left font-medium w-[26%]">File</th>
                <th className="px-4 py-2 text-right font-medium w-[9%] tabular-nums">Lines</th>
                <th className="px-4 py-2 text-left font-medium w-[17%]">State</th>
                <th className="px-4 py-2 text-center font-medium w-[12%]">Translated</th>
                <th className="px-4 py-2 text-center font-medium w-[12%]">Verified</th>
                <th className="px-4 py-2 text-left font-medium">Progress</th>
              </tr>
            </thead>
            <tbody>
              {tableRows.length === 0 ? (
                <tr>
                  <td colSpan={6} className="px-4 py-6 text-center text-sm text-muted-foreground">
                    No files selected. Go to the Project view to select files.
                  </td>
                </tr>
              ) : (
                tableRows.map(({ file, row, state, result }) => {
                  const showProgress =
                    row &&
                    row.total > 0 &&
                    state !== "pending" &&
                    state !== "done" &&
                    state !== "warning";

                  const issues = (row?.issues.length ? row.issues : result?.issues) ?? [];
                  const expandable = state === "warning" && issues.length > 0;
                  const isExpanded = expandable && expanded.has(file.name);
                  const cells = stateToCells(state, row?.reachedVerify ?? false);
                  const isHero = running && heroName === file.name;
                  // While running, a click pins this file as the hero; otherwise
                  // a click expands a warning file's issue list.
                  const onActivate = running
                    ? () => setSelectedHero(file.name)
                    : expandable
                      ? () => toggleExpand(file.name)
                      : undefined;

                  return (
                    <Fragment key={file.name}>
                      <tr
                        className={`border-b border-border last:border-0 hover:bg-[color:var(--color-bg-hover)]${onActivate ? " cursor-pointer" : ""}${isHero ? " bg-[color:color-mix(in_oklch,var(--color-gold)_8%,transparent)]" : ""}`}
                        onClick={onActivate}
                        role={onActivate ? "button" : undefined}
                        tabIndex={onActivate ? 0 : undefined}
                        onKeyDown={
                          onActivate
                            ? (e) => {
                                if (e.key === "Enter" || e.key === " ") {
                                  e.preventDefault();
                                  onActivate();
                                }
                              }
                            : undefined
                        }
                      >
                        <td className="px-4 py-2.5 text-[12.5px] font-mono truncate max-w-0 w-[26%]">
                          <span className="truncate block">
                            {expandable ? (
                              isExpanded ? (
                                <CaretDown className="mr-1 inline size-3 text-muted-foreground" />
                              ) : (
                                <CaretRight className="mr-1 inline size-3 text-muted-foreground" />
                              )
                            ) : null}
                            {file.name}
                          </span>
                        </td>
                        <td className="px-4 py-2.5 text-right tabular-nums text-[12px] text-muted-foreground w-[9%]">
                          {file.dialogue_count.toLocaleString()}
                        </td>
                        <td className="px-4 py-2.5 w-[17%]">
                          <FileStateChip state={state} />
                        </td>
                        <td className="px-4 py-2.5 text-center w-[12%]">
                          <StatusGlyph kind={cells.translated} tone="translate" />
                        </td>
                        <td className="px-4 py-2.5 text-center w-[12%]">
                          <StatusGlyph kind={cells.verified} tone="verify" />
                        </td>
                        <td className="px-4 py-2.5 text-[12px] text-muted-foreground tabular-nums">
                          {showProgress && row ? (
                            <span>
                              {row.translated}/{row.total} · batch {row.batch}/{row.totalBatches}
                              {row.retries > 0 ? (
                                <span className="ml-2 text-[color:var(--color-alert)]">
                                  ↺ {row.retries}
                                </span>
                              ) : null}
                            </span>
                          ) : row?.error ? (
                            <span className="text-[color:var(--color-danger)] truncate block max-w-xs">
                              {row.error}
                            </span>
                          ) : (
                            <span>—</span>
                          )}
                        </td>
                      </tr>
                      {isExpanded ? (
                        <tr className="border-b border-border last:border-0">
                          <td colSpan={6} className="p-0">
                            <IssuePanel issues={issues} />
                          </td>
                        </tr>
                      ) : null}
                    </Fragment>
                  );
                })
              )}
            </tbody>
          </table>
        </div>

        <SectionHelp title="What the automatic checks look for">
          <ul className="ml-1 list-disc space-y-1 pl-4 text-[11.5px] text-muted-foreground">
            <li>Drift — the translation wandering off the original meaning.</li>
            <li>Dropped or merged lines.</li>
            <li>Terms that don’t match your glossary.</li>
          </ul>
          <p className="mt-1.5 text-[11.5px] text-muted-foreground">
            These are flagged as issues to review — there’s no quality score.
          </p>
        </SectionHelp>

        {/* Overall pipeline progress (while running) */}
        {running && integ.total > 0 ? (
          <div className="space-y-1.5">
            <div className="flex justify-between text-[11px] text-muted-foreground tabular-nums">
              <span>Pipeline · {integ.done} of {integ.total} batches complete</span>
              <span>{Math.round((integ.done / integ.total) * 100)}%</span>
            </div>
            <ReactorBar done={integ.done} total={integ.total} />
          </div>
        ) : null}

      </div>

      {/* Controls row + log toggle */}
      <div className="border-t border-border bg-[color:var(--popover)] px-5 py-3 flex items-center gap-3">
        {running ? (
          <Button variant="destructive" onClick={handleCancel}>
            <Stop className="size-4" /> Cancel
          </Button>
        ) : (
          <Button disabled={!canStart} onClick={handleStart}>
            <Play className="size-4" /> Start translation
          </Button>
        )}

        {selected.length === 0 && !running ? (
          <span className="text-[11.5px] text-muted-foreground">
            Select files in the Project view first.
          </span>
        ) : !hasUsableConnection && !running ? (
          <span className="text-[11.5px] text-[color:var(--color-alert)]">
            {translationLabel
              ? "A model needs its API key — add it in Connections."
              : "No AI model connected — set one up in Connections."}
          </span>
        ) : null}

        <span className="flex-1" />

        {/* Log toggle */}
        <LogToggleButton open={logsOpen} count={logs.length} onToggle={() => setLogsOpen((o) => !o)} />
      </div>

      {/* Log panel */}
      {logsOpen ? (
        <LogPanel
          lines={logs.map((entry) => ({
            at: entry.at,
            level: entry.level,
            message: entry.message,
            meta: [
              entry.file ? (
                <span key="f" className="shrink-0 text-muted-foreground truncate max-w-[120px]">
                  {entry.file}
                </span>
              ) : (
                <span key="f" className="shrink-0 text-muted-foreground">run</span>
              ),
              <span key="p" className="shrink-0 text-muted-foreground/60">[{entry.phase}]</span>,
            ],
          }))}
        />
      ) : null}
    </div>
  );
}
