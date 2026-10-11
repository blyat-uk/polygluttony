import { Fragment, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { CaretDown, CaretRight, Eye, Play, Stop } from "@phosphor-icons/react";
import { stateToCells, StatusGlyph, IssuePanel } from "./file-cells"
import { ReactorBar } from "./reactor-bar"
import { FileStateChip } from "./file-state-chip"
import { runProgress } from "./telemetry";
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

/** Opens the episode's own window, or focuses it if it's already open. */
function watchEpisode(file: string) {
  ipc.openEpisodeWindow(file).catch((err: unknown) => {
    toast.error(`Couldn't open ${file}`, {
      description: err instanceof Error ? err.message : String(err),
    });
  });
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
  const runFolder = useTranslationRun((s) => s.folder);
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

  // ── run progress ─────────────────────────────────────────────────────────────

  const progress = runProgress(
    storeFiles,
    new Map(view.files.map((f) => [f.name, f.dialogue_count])),
  );

  // ── actions ──────────────────────────────────────────────────────────────────

  const canStart = !running && selected.length > 0 && hasUsableConnection;

  const handleStart = () => {
    setExpanded(new Set());
    useTranslationRun.getState().start(selected, workdir);
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
          Double-click an episode during or after a run to watch it in its own window. Episodes that need a
          look turn amber; click one to see the exact issues.
        </HelpText>

        {/* Completion summary */}
        {summaryEl}

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
                  // Files of the current or last run have a live board behind
                  // them, so they can open in their own window. Same-named
                  // files in another folder don't.
                  const watchable = row !== undefined && runFolder === workdir;

                  return (
                    <Fragment key={file.name}>
                      <tr
                        className={`group border-b border-border last:border-0 hover:bg-[color:var(--color-bg-hover)]${expandable || watchable ? " cursor-pointer" : ""}`}
                        onClick={
                          expandable
                            ? (e) => {
                                // The second click of a double-click would fold it back.
                                if (e.detail > 1) return;
                                toggleExpand(file.name);
                              }
                            : undefined
                        }
                        onDoubleClick={watchable ? () => watchEpisode(file.name) : undefined}
                        // Keep a double-click from selecting the word under the cursor.
                        onMouseDown={
                          watchable
                            ? (e) => {
                                if (e.detail > 1) e.preventDefault();
                              }
                            : undefined
                        }
                        role={expandable ? "button" : undefined}
                        tabIndex={expandable ? 0 : undefined}
                        aria-expanded={expandable ? isExpanded : undefined}
                        onKeyDown={
                          expandable
                            ? (e) => {
                                // Keys on the eye button are the button's own.
                                if (e.target !== e.currentTarget) return;
                                if (e.key === "Enter" || e.key === " ") {
                                  e.preventDefault();
                                  toggleExpand(file.name);
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
                          <div className="flex items-center gap-2">
                            <div className="min-w-0 flex-1">
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
                            </div>
                            {watchable ? (
                              <button
                                type="button"
                                aria-label={`Watch ${file.name} live`}
                                title="Watch live"
                                className="-my-1 grid size-6 shrink-0 place-items-center rounded-full text-muted-foreground opacity-0 outline-none transition hover:text-foreground group-hover:opacity-100 focus-visible:opacity-100 focus-visible:ring-[3px] focus-visible:ring-ring/50"
                                onClick={(e) => {
                                  e.stopPropagation();
                                  // A double-click is two clicks; open once.
                                  if (e.detail > 1) return;
                                  watchEpisode(file.name);
                                }}
                                onDoubleClick={(e) => e.stopPropagation()}
                              >
                                <Eye className="size-4" />
                              </button>
                            ) : null}
                          </div>
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
        {running && progress.total > 0 ? (
          <div className="space-y-1.5">
            <div className="flex justify-between text-[11px] text-muted-foreground tabular-nums">
              <span>
                Pipeline · {progress.done.toLocaleString()} of {progress.total.toLocaleString()} lines ·{" "}
                {progress.finished.toLocaleString()} of {progress.files.toLocaleString()}{" "}
                episode{progress.files !== 1 ? "s" : ""} finished
              </span>
              <span>{Math.round((progress.done / progress.total) * 100)}%</span>
            </div>
            {/* ReactorBar draws a divider per unit of `total`; thousands of lines
                would bury the fill, so it gets the fraction as one segment. */}
            <ReactorBar done={progress.done / progress.total} total={1} />
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
