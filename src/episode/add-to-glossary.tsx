import { useEffect, useId, useState, type RefObject } from "react";
import { Popover as PopoverPrimitive } from "radix-ui";
import { emit } from "@tauri-apps/api/event";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent } from "@/components/ui/popover";
import { GLOSSARY_CHANGED_EVENT, ipc, type GlossaryChanged } from "@/lib/ipc";
import { findTerm, GLOSSARY_CATEGORIES, plainError, withTerm, type GlossaryCategory } from "./review";

type Rect = { left: number; top: number; width: number; height: number };

/** Text selected inside one line's Original or Translation cell. */
export type LineSelection = { text: string; column: "source" | "translation"; rect: Rect };

/** The Add to glossary shortcut, as this platform writes it. */
export const ADD_SHORTCUT = /Mac|iPhone|iPad/.test(navigator.platform) ? "⌘ E" : "Ctrl E";

/** Longer than this, a selection is a sentence, not a term. */
const MAX_TERM = 60;

/** The text selected in a line of the table under `root`, kept in step as
 *  the selection changes or the lines scroll. */
export function useLineSelection(root: RefObject<HTMLElement | null>): LineSelection | null {
  const [sel, setSel] = useState<LineSelection | null>(null);
  useEffect(() => {
    const read = () => {
      const s = window.getSelection();
      if (!s || s.isCollapsed || s.rangeCount === 0) return setSel(null);
      const range = s.getRangeAt(0);
      const node = range.commonAncestorContainer;
      const el = node instanceof Element ? node : node.parentElement;
      const cell = el?.closest<HTMLElement>("[data-col]");
      const text = s.toString().replace(/\s+/g, " ").trim();
      if (!cell || !root.current?.contains(cell) || !text || text.length > MAX_TERM) {
        return setSel(null);
      }
      const r = range.getBoundingClientRect();
      setSel({
        text,
        column: cell.dataset.col === "translation" ? "translation" : "source",
        rect: { left: r.left, top: r.top, width: r.width, height: r.height },
      });
    };
    document.addEventListener("selectionchange", read);
    window.addEventListener("scroll", read, true);
    window.addEventListener("resize", read);
    return () => {
      document.removeEventListener("selectionchange", read);
      window.removeEventListener("scroll", read, true);
      window.removeEventListener("resize", read);
    };
  }, [root]);
  return sel;
}

/** What the Add to glossary popover opens with, and where. */
export type GlossaryDraft = { source: string; target: string; anchor: Rect };

/** The draft a selection makes: a selected original becomes the source, a
 *  selected translation the translation. */
export function draftFrom(sel: LineSelection | null, fallback: Rect): GlossaryDraft {
  if (!sel) return { source: "", target: "", anchor: fallback };
  return sel.column === "source"
    ? { source: sel.text, target: "", anchor: sel.rect }
    : { source: "", target: sel.text, anchor: sel.rect };
}

/** The small button over a selection. Keeps the selection when pressed. */
export function SelectionButton({ sel, onOpen }: { sel: LineSelection; onOpen: () => void }) {
  const top = Math.max(6, sel.rect.top - 30);
  const left = Math.max(70, Math.min(window.innerWidth - 70, sel.rect.left + sel.rect.width / 2));
  return (
    <button
      type="button"
      onMouseDown={(e) => e.preventDefault()}
      onClick={onOpen}
      style={{ top, left }}
      className="fixed z-40 inline-flex h-6 -translate-x-1/2 items-center gap-1.5 rounded-full border border-[color:color-mix(in_oklch,var(--color-gold)_45%,transparent)] bg-[color:var(--color-raised)] px-2.5 text-[11px] text-[color:var(--color-gold-hi)] shadow-[0_6px_18px_-8px_rgba(0,0,0,.8)] outline-none hover:border-[color:var(--color-gold)] focus-visible:ring-[3px] focus-visible:ring-ring/50"
    >
      Add to glossary <Kbd>{ADD_SHORTCUT}</Kbd>
    </button>
  );
}

export function Kbd({ children }: { children: string }) {
  return (
    <span className="rounded-[4px] border border-[color:var(--input)] px-1 font-mono text-[9.5px] text-muted-foreground">
      {children}
    </span>
  );
}

const FIELD_LABEL = "mb-1 block text-[10.5px] font-medium uppercase tracking-wide text-muted-foreground";

/** Add a term from the text you're looking at: source, translation and
 *  category, saved straight into the folder's glossary as yours. */
export function AddToGlossary({
  folder,
  draft,
  onClose,
  onAdded,
}: {
  folder: string;
  draft: GlossaryDraft;
  onClose: () => void;
  onAdded: (source: string, target: string, categoryLabel: string) => void;
}) {
  const id = useId();
  const [source, setSource] = useState(draft.source);
  const [target, setTarget] = useState(draft.target);
  const [category, setCategory] = useState<GlossaryCategory>("characters");
  const [existing, setExisting] = useState<{ category: GlossaryCategory; target: string } | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // A term the glossary already has opens with its category and translation.
  useEffect(() => {
    let gone = false;
    const term = draft.source.trim();
    if (!term) return;
    ipc
      .loadGlossary(folder)
      .then((load) => {
        if (gone || load.status !== "ok") return;
        const found = findTerm(load.doc, term);
        if (!found) return;
        setExisting(found);
        setCategory(found.category);
        setTarget((t) => t || found.target);
      })
      .catch(() => undefined);
    return () => {
      gone = true;
    };
  }, [folder, draft.source]);

  const ready = source.trim() !== "" && target.trim() !== "" && !saving;

  const save = async () => {
    if (!ready) return;
    setSaving(true);
    setError(null);
    try {
      const load = await ipc.loadGlossary(folder);
      if (load.status === "missing") {
        throw new Error("This folder has no glossary yet. Build one on the Glossary page first.");
      }
      if (load.status === "unreadable") {
        throw new Error("The glossary file can't be read right now. Fix it on the Glossary page first.");
      }
      const next = withTerm(load.doc, load.meta, category, source.trim(), target.trim());
      await ipc.saveGlossary(folder, next.doc, next.meta);
      // The main window reloads its copy, so a later edit there can't save over this term.
      void emit(GLOSSARY_CHANGED_EVENT, { folder } satisfies GlossaryChanged).catch(() => undefined);
      const label = GLOSSARY_CATEGORIES.find((c) => c.key === category)?.label ?? category;
      onAdded(source.trim(), target.trim(), label);
    } catch (e) {
      setError(plainError(e));
      setSaving(false);
    }
  };

  const { anchor } = draft;
  return (
    <Popover open onOpenChange={(open) => (open ? undefined : onClose())}>
      <PopoverPrimitive.Anchor asChild>
        <span
          aria-hidden
          className="pointer-events-none fixed"
          style={{ left: anchor.left, top: anchor.top, width: anchor.width, height: anchor.height }}
        />
      </PopoverPrimitive.Anchor>
      <PopoverContent side="bottom" align="start" className="w-[320px] p-3.5">
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void save();
          }}
          className="flex flex-col gap-2.5"
        >
          <p className="text-[13px] font-medium text-[color:var(--color-ink-emphasis)]">Add to glossary</p>
          <div>
            <label htmlFor={`${id}-src`} className={FIELD_LABEL}>
              Original
            </label>
            <Input
              id={`${id}-src`}
              value={source}
              autoFocus={draft.source === ""}
              onChange={(e) => setSource(e.target.value)}
              className="h-8"
            />
          </div>
          <div>
            <label htmlFor={`${id}-tgt`} className={FIELD_LABEL}>
              Translation
            </label>
            <Input
              id={`${id}-tgt`}
              value={target}
              autoFocus={draft.source !== ""}
              onChange={(e) => setTarget(e.target.value)}
              className="h-8"
            />
          </div>
          <div>
            <label htmlFor={`${id}-cat`} className={FIELD_LABEL}>
              Category
            </label>
            <select
              id={`${id}-cat`}
              value={category}
              onChange={(e) => setCategory(e.target.value as GlossaryCategory)}
              className="h-8 w-full rounded-4xl border border-input bg-input/30 px-3 text-sm outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50"
            >
              {GLOSSARY_CATEGORIES.map((c) => (
                <option key={c.key} value={c.key}>
                  {c.label}
                </option>
              ))}
            </select>
          </div>
          {existing ? (
            <p className="text-[11px] text-muted-foreground">
              Already in the glossary as “{existing.target}”. Saving replaces it.
            </p>
          ) : null}
          {error ? <p className="text-[11.5px] text-[color:var(--color-danger)]">{error}</p> : null}
          <div className="mt-0.5 flex justify-end gap-2">
            <Button type="button" variant="ghost" size="sm" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" size="sm" disabled={!ready}>
              {saving ? "Adding…" : "Add"}
            </Button>
          </div>
        </form>
      </PopoverContent>
    </Popover>
  );
}
