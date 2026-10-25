export type RowItem<T> = { type: "row"; row: T } | { type: "gap"; count: number; key: string };

/**
 * Fold long runs of quiet rows (done, unticked) so the new episode at the bottom
 * shows without scrolling: a run of `min` or more keeps its first and last row
 * and folds the middle into one "· · · 10 more done · · ·" gap.
 */
export function collapseQuietRuns<T>(
  rows: T[],
  quiet: (row: T) => boolean,
  keyOf: (row: T) => string,
  min = 5,
): RowItem<T>[] {
  const out: RowItem<T>[] = [];
  let i = 0;
  while (i < rows.length) {
    if (!quiet(rows[i])) {
      out.push({ type: "row", row: rows[i] });
      i++;
      continue;
    }
    let j = i;
    while (j < rows.length && quiet(rows[j])) j++;
    const run = rows.slice(i, j);
    if (run.length >= min) {
      out.push({ type: "row", row: run[0] });
      out.push({ type: "gap", count: run.length - 2, key: `gap-${keyOf(run[1])}` });
      out.push({ type: "row", row: run[run.length - 1] });
    } else {
      for (const row of run) out.push({ type: "row", row });
    }
    i = j;
  }
  return out;
}
