import { useEffect, useRef, useState } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { EPISODE_CHANGED_EVENT, ipc, onBackendEvent } from "@/lib/ipc";
import type { EpisodeChanged } from "@/types/generated/EpisodeChanged";
import type { EpisodeView } from "@/types/generated/EpisodeView";

export type EpisodeData = {
  /** Null until the first snapshot arrives. */
  view: EpisodeView | null;
  /** The last fetch's error; cleared by the next successful fetch. */
  error: string | null;
  /** Line id → `performance.now()` when its translation last appeared or
   *  changed. Survives refetches that bring no new text (Landed and Idle nudge
   *  back to back), so a fade isn't cut short. */
  landed: ReadonlyMap<number, number>;
};

const NONE_LANDED: ReadonlyMap<number, number> = new Map();

/** Landing times older than this are dropped; any fade is long over by then. */
const LANDED_KEEP_MS = 5000;

/**
 * One episode's live board. Fetches a full snapshot once the change listener is
 * attached (so no nudge can fall between the two), then again on every
 * `episode://changed` for this file or for every file. At most one fetch is in
 * flight; nudges that arrive meanwhile collapse into exactly one more fetch.
 */
export function useEpisode(file: string): EpisodeData {
  const [data, setData] = useState<EpisodeData>({ view: null, error: null, landed: NONE_LANDED });
  // The previous live snapshot's translations, keyed by line id. Null before the
  // first live snapshot, so the first load doesn't fade every row in.
  const seen = useRef<Map<number, string> | null>(null);

  useEffect(() => {
    let disposed = false;
    let inFlight = false;
    let again = false;
    let unlisten: UnlistenFn | undefined;

    const apply = (view: EpisodeView) => {
      if (view.kind !== "live") {
        seen.current = null;
        setData({ view, error: null, landed: NONE_LANDED });
        return;
      }
      const prev = seen.current;
      const next = new Map<number, string>();
      const fresh: number[] = [];
      for (const line of view.lines) {
        if (line.translation === null) continue;
        next.set(line.id, line.translation);
        if (prev && prev.get(line.id) !== line.translation) fresh.push(line.id);
      }
      seen.current = next;
      const now = performance.now();
      setData((d) => {
        let landed = d.landed;
        const stale = [...landed.values()].some((t) => now - t > LANDED_KEEP_MS);
        if (fresh.length > 0 || stale) {
          const m = new Map<number, number>();
          for (const [id, t] of landed) if (now - t <= LANDED_KEEP_MS) m.set(id, t);
          for (const id of fresh) m.set(id, now);
          landed = m.size > 0 ? m : NONE_LANDED;
        }
        return { view, error: null, landed };
      });
    };

    const refresh = async () => {
      if (inFlight) {
        again = true;
        return;
      }
      inFlight = true;
      try {
        do {
          again = false;
          try {
            const view = await ipc.episodeSnapshot(file);
            if (disposed) return;
            apply(view);
          } catch (err) {
            if (disposed) return;
            const error = err instanceof Error ? err.message : String(err);
            setData((d) => ({ ...d, error }));
          }
        } while (again && !disposed);
      } finally {
        inFlight = false;
      }
    };

    onBackendEvent<EpisodeChanged>(EPISODE_CHANGED_EVENT, (e) => {
      if (e.payload.file === null || e.payload.file === file) void refresh();
    })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      // Without the listener the window still shows the snapshot it can get.
      .catch(() => undefined)
      .finally(() => {
        if (!disposed) void refresh();
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [file]);

  return data;
}
