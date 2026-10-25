import { useEffect, useRef, useState } from "react"
import { useGlossaryRun } from "@/stores/glossary-store"
import { useAppStore } from "@/stores/app-store"
import { ReactorBar } from "@/features/translate/reactor-bar"
import type { GlossaryPhase } from "@/types/generated/GlossaryPhase"
import { titleCase } from "./model"

/** The six real glossary categories (engine: src-tauri/.../glossary/model.rs). */
const CATS: { key: string; label: string }[] = [
  { key: "characters", label: "Characters" },
  { key: "cultivation", label: "Cultivation" },
  { key: "skills", label: "Skills" },
  { key: "locations", label: "Locations" },
  { key: "items", label: "Items" },
  { key: "organizations", label: "Organizations" },
]

/** Chips shown per lane before collapsing the rest into "+N more". A season can
 *  yield thousands of terms; uncapped lanes would grow without bound. */
const LANE_CAP = 10

/** What a glossary run is doing, in the app's words (section 8). */
export const PHASE_LABEL: Record<GlossaryPhase, string> = {
  loading: "Reading subtitles",
  reference: "Reading your translated episodes",
  extracting: "Finding names",
  normalizing: "Tidying up",
  personalizing: "Looking up names",
  saving: "Saving glossary",
}

/**
 * Live glossary-build console. Terms stream into their six category lanes as
 * extraction batches land (engine emits on batch *completion*, deduped); a local
 * reveal queue drips them in with individual pops. Honest by construction: no
 * fake per-lane bars, no unknown "total" — the count is a running tally and the
 * only progress bar tracks *batches scanned*. The progress bar is pinned at the
 * top so growing lanes never push it off-screen; each lane caps at LANE_CAP.
 */
export function ExtractionConsole() {
  const phase = useGlossaryRun((s) => s.phase)
  const done = useGlossaryRun((s) => s.done)
  const total = useGlossaryRun((s) => s.total)
  const terms = useGlossaryRun((s) => s.glossTerms)
  const worldType = useAppStore((s) => s.worldType)

  // Reveal queue: `shown[cat]` lags `terms[cat]`, dripping forward each tick so
  // bursts arrive smoothly. Budget scales with backlog → never falls far behind.
  const termsRef = useRef(terms)
  termsRef.current = terms
  const [shown, setShown] = useState<Record<string, number>>({})
  useEffect(() => {
    const id = setInterval(() => {
      setShown((cur) => {
        const t = termsRef.current
        let backlog = 0
        for (const { key } of CATS) backlog += (t[key]?.length ?? 0) - (cur[key] ?? 0)
        if (backlog <= 0) return cur
        let budget = Math.max(1, Math.ceil(backlog / 5))
        const next = { ...cur }
        let advanced = true
        while (budget > 0 && advanced) {
          advanced = false
          for (const { key } of CATS) {
            if ((next[key] ?? 0) < (t[key]?.length ?? 0)) {
              next[key] = (next[key] ?? 0) + 1
              advanced = true
              if (--budget <= 0) break
            }
          }
        }
        return next
      })
    }, 60)
    return () => clearInterval(id)
  }, [])

  // Flash a lane when it just revealed a term.
  const prevShown = useRef<Record<string, number>>({})
  const [flash, setFlash] = useState<Set<string>>(new Set())
  useEffect(() => {
    const grew = new Set<string>()
    for (const { key } of CATS) {
      const n = shown[key] ?? 0
      if (n > (prevShown.current[key] ?? 0)) grew.add(key)
      prevShown.current[key] = n
    }
    if (grew.size === 0) return
    setFlash(grew)
    const tm = setTimeout(() => setFlash(new Set()), 380)
    return () => clearTimeout(tm)
  }, [shown])

  const revealed = CATS.reduce((sum, { key }) => sum + Math.min(shown[key] ?? 0, terms[key]?.length ?? 0), 0)

  const status =
    phase === "extracting" && total > 0
      ? `Finding names · batch ${done} of ${total}`
      : phase
        ? PHASE_LABEL[phase]
        : "Starting…"

  return (
    <div className="flex flex-col gap-5">
      {/* batch reactor — pinned at top so growing lanes never push it off-screen */}
      <div className="space-y-1.5">
        <div className="flex justify-between text-[11px] text-muted-foreground tabular-nums">
          <span>{status}</span>
          {phase === "extracting" && total > 0 ? <span>{Math.round((done / total) * 100)}%</span> : null}
        </div>
        <ReactorBar done={done} total={total} />
      </div>

      {/* world type + running tally */}
      <div className="flex flex-wrap items-center justify-between gap-4">
        <div className="flex items-center gap-3">
          <span className="text-[10px] uppercase tracking-[0.2em] text-[color:var(--color-ink-dim)]">World type</span>
          {worldType ? (
            <span
              className="text-[22px] font-bold tracking-[0.02em] text-[color:var(--color-gold-hi)]"
              style={{ textShadow: "0 0 18px color-mix(in oklch, var(--color-gold) 55%, transparent)" }}
            >
              {titleCase(worldType)}
            </span>
          ) : (
            <span className="text-[13px] text-muted-foreground">Detecting…</span>
          )}
        </div>
        <div className="text-right">
          <div className="text-[30px] font-bold leading-none text-[color:var(--color-ink-emphasis)] tabular-nums">
            {revealed}
          </div>
          <div className="text-[10px] uppercase tracking-[0.16em] text-[color:var(--color-ink-dim)]">
            terms found so far
          </div>
        </div>
      </div>

      {/* category lanes — capped at LANE_CAP chips + "+N more" */}
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-3">
        {CATS.map(({ key, label }) => {
          const have = terms[key]?.length ?? 0
          const count = Math.min(shown[key] ?? 0, have)
          const visible = (terms[key] ?? []).slice(0, Math.min(LANE_CAP, count))
          const more = count - visible.length
          return (
            <div
              key={key}
              className={`min-h-[104px] rounded-xl border p-3 transition-[border-color,box-shadow] duration-300 ${
                flash.has(key)
                  ? "border-[color:color-mix(in_oklch,var(--color-gold)_40%,transparent)] [box-shadow:0_0_0_1px_color-mix(in_oklch,var(--color-gold)_18%,transparent)]"
                  : "border-border"
              } bg-black/15`}
            >
              <div className="mb-2.5 flex items-center justify-between">
                <span className="text-[11px] font-semibold tracking-[0.04em] text-foreground">{label}</span>
                <span className="text-[13px] font-bold tabular-nums text-[color:var(--color-gold-hi)]">{count}</span>
              </div>
              <div className="flex flex-wrap gap-1.5">
                {visible.map((t) => (
                  <span
                    key={t.source}
                    title={`${t.source} → ${t.target}`}
                    className="rounded-full border border-[color:var(--input)] bg-[color:color-mix(in_oklch,var(--color-gold)_5%,transparent)] px-2 py-0.5 text-[10.5px] text-foreground [animation:gloss-pop_.42s_cubic-bezier(.16,1.5,.4,1)]"
                  >
                    {t.target || t.source}
                  </span>
                ))}
                {more > 0 ? (
                  <span
                    key="more"
                    className="rounded-full border border-border px-2 py-0.5 text-[10.5px] tabular-nums text-muted-foreground"
                  >
                    +{more} more
                  </span>
                ) : null}
              </div>
            </div>
          )
        })}
      </div>
    </div>
  )
}
