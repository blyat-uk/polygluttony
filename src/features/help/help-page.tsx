import type { ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { Check } from "@phosphor-icons/react";
import { PAGE_SHORTCUTS } from "@/components/app-layout";
import { PageHeader } from "@/components/page-header";
import { cn } from "@/lib/utils";
import { useAppStore } from "@/stores/app-store";
import { useTranslationRun } from "@/stores/translation-store";
import { usePickFolder } from "@/features/welcome/use-pick-folder";
import { MOD } from "./keys";

// ── inline helpers ────────────────────────────────────────────────────────────

/** Source-language term, shown monospace. */
function Zh({ children }: { children: ReactNode }) {
  return <span className="font-mono text-foreground">{children}</span>;
}

/** Translated output, accent-colored. */
function Out({ children }: { children: ReactNode }) {
  return <span className="text-primary">{children}</span>;
}

function Section({ title, sub, children }: { title: string; sub?: string; children: ReactNode }) {
  return (
    <section>
      <div
        className={`flex items-center gap-2 text-[10px] font-semibold uppercase tracking-[0.09em] text-muted-foreground ${sub ? "mb-1" : "mb-3"}`}
      >
        <span className="size-1.5 rounded-full bg-primary" />
        {title}
      </div>
      {sub ? <p className="mb-3 text-[12px] text-muted-foreground">{sub}</p> : null}
      {children}
    </section>
  );
}

// ── getting started ───────────────────────────────────────────────────────────

type StepId = "model" | "folder" | "glossary" | "translate";

const STEPS: {
  id: StepId;
  title: string;
  optional?: boolean;
  body: string;
  /** Where the link goes; `null` = the folder dialog. */
  to: string | null;
  link: string;
}[] = [
  {
    id: "model",
    title: "Connect a model",
    body: "Polygluttony needs an AI model to build your glossary and translate. Pick a provider, paste your API key and press Connect — or use a model running on your computer. One model does every step; you can give steps their own models later.",
    to: "/connections",
    link: "Open Models",
  },
  {
    id: "folder",
    title: "Open a folder",
    body: "Point Polygluttony at a folder of subtitles, or drag it onto the window. It shows each episode and whether it’s translated yet.",
    to: null,
    link: "Open folder",
  },
  {
    id: "glossary",
    title: "Build glossary",
    optional: true,
    body: "A glossary keeps names and terms the same in every episode. Translate builds one first, and adds new episodes to it later, while “Build a glossary first” is ticked. You can also build it on its own on the Glossary page.",
    to: "/glossary",
    link: "Open Glossary",
  },
  {
    id: "translate",
    title: "Translate",
    body: "Tick the episodes and press the gold button. Polygluttony reads them for the glossary first when that’s ticked, then translates, checks its own work as it goes and marks anything that needs a look.",
    to: "/episodes",
    link: "Open Episodes",
  },
];

/** Which of the getting-started steps are already behind you. */
function useStepsDone(): Record<StepId, boolean> {
  const model = useAppStore((s) => s.hasUsableConnection);
  const folder = useAppStore((s) => s.workdir !== null);
  const glossary = useAppStore((s) => (s.glossaryTerms ?? 0) > 0);
  const translated = useTranslationRun((s) => (s.results ?? []).some((r) => r.success));
  return { model, folder, glossary, translate: translated };
}

function GettingStarted() {
  const done = useStepsDone();
  const { pick } = usePickFolder();
  return (
    <Section title="Getting started">
      <div className="space-y-2">
        {STEPS.map((s, i) => (
          <div
            key={s.id}
            className="flex items-start gap-3 rounded-lg border border-border bg-[color:var(--card)] px-3.5 py-3"
          >
            <span
              className={cn(
                "mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-full text-[12px]",
                done[s.id]
                  ? "border border-[color:var(--color-jade)] text-[color:var(--color-jade)]"
                  : "bg-[color:var(--popover)] text-primary",
              )}
            >
              {done[s.id] ? <Check weight="bold" className="size-3.5" /> : i + 1}
            </span>
            <div className="min-w-0 flex-1">
              <div className="text-[13px] font-semibold text-foreground">
                {s.title}
                {done[s.id] ? (
                  <span className="ml-1.5 text-[11px] font-normal text-[color:var(--color-jade)]">
                    · done
                  </span>
                ) : s.optional ? (
                  <span className="ml-1.5 text-[11px] font-normal text-muted-foreground">
                    — optional
                  </span>
                ) : null}
              </div>
              <p className="mt-0.5 text-[12.5px] text-foreground/90">{s.body}</p>
              {s.to === null ? (
                <button
                  type="button"
                  onClick={() => void pick()}
                  className="mt-1.5 inline-block rounded-sm text-[11.5px] font-medium text-primary outline-none hover:underline focus-visible:ring-[3px] focus-visible:ring-ring/50"
                >
                  → {s.link}
                </button>
              ) : (
                <Link
                  to={s.to as never}
                  className="mt-1.5 inline-block text-[11.5px] font-medium text-primary hover:underline"
                >
                  → {s.link}
                </Link>
              )}
            </div>
          </div>
        ))}
      </div>
    </Section>
  );
}

// ── keyboard shortcuts ────────────────────────────────────────────────────────

function Key({ children }: { children: ReactNode }) {
  return (
    <kbd className="inline-flex min-w-5 items-center justify-center rounded-md border border-border bg-[color:var(--color-raised)] px-1.5 py-px font-mono text-[11px] text-foreground">
      {children}
    </kbd>
  );
}

const SHORTCUTS: { keys: string[]; does: string }[] = [
  {
    keys: [`${MOD}1`, `${MOD}5`],
    does: PAGE_SHORTCUTS.map((p) => p.label).join(", "),
  },
  { keys: [`${MOD}O`], does: "Open a folder" },
  { keys: [`${MOD}Enter`], does: "The page’s main button, e.g. Translate" },
  { keys: [`${MOD}S`], does: "Save a prompt" },
  { keys: [`${MOD}Z`], does: "Undo a glossary change (add Shift to redo)" },
  { keys: [`${MOD}F`], does: "Search the glossary" },
  { keys: ["Esc"], does: "Cancel an edit, close a panel" },
];

function Shortcuts() {
  return (
    <Section title="Keyboard shortcuts">
      <div className="grid grid-cols-[max-content_1fr] items-center gap-x-4 gap-y-2 rounded-lg border border-border bg-[color:var(--card)] px-4 py-3 text-[12.5px]">
        {SHORTCUTS.map((s) => (
          <div key={s.does} className="contents">
            <span className="flex items-center gap-1 text-muted-foreground">
              <Key>{s.keys[0]}</Key>
              {s.keys[1] ? (
                <>
                  … <Key>{s.keys[1]}</Key>
                </>
              ) : null}
            </span>
            <span className="text-foreground/90">{s.does}</span>
          </div>
        ))}
      </div>
    </Section>
  );
}

// ── behind the scenes ─────────────────────────────────────────────────────────

function Explainer({
  title,
  example,
  children,
}: {
  title: string;
  example: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="rounded-lg border border-border bg-[color:var(--card)] px-4 py-3.5">
      <div className="text-[13px] font-semibold text-foreground">{title}</div>
      <p className="mt-1 text-[12.5px] text-foreground/90">{children}</p>
      <div className="mt-2 border-l-2 border-border pl-2.5 text-[11.5px] text-muted-foreground">
        {example}
      </div>
    </div>
  );
}

function BehindTheScenes() {
  return (
    <Section title="Behind the scenes" sub="What Polygluttony is doing for you while it translates.">
      <div className="space-y-2">
        <Explainer
          title="It keeps names and terms consistent"
          example={
            <>
              e.g. <Zh>张伟</Zh> stays <Out>“Zhang Wei”</Out> in episode 1 and episode 50 — never
              “Zhang Wei” then “Wei Zhang”.
            </>
          }
        >
          Translate builds a glossary first and adds each new episode to it — or Import from
          translated episodes you already have — and Polygluttony hands those agreed
          translations to the AI on every episode.
        </Explainer>

        <Explainer
          title="It tunes to the world type"
          example={
            <div className="grid grid-cols-[max-content_1fr] gap-x-3 gap-y-1">
              <span className="font-semibold text-foreground">Xianxia</span>
              <span>
                cultivation realms &amp; techniques — <Zh>金丹</Zh> → <Out>“Golden Core”</Out>,{" "}
                <Zh>渡劫</Zh> → <Out>“Tribulation”</Out>
              </span>
              <span className="font-semibold text-foreground">Wuxia</span>
              <span>
                sects &amp; internal arts — <Zh>内力</Zh> → <Out>“internal energy”</Out>,{" "}
                <Zh>掌门</Zh> → <Out>“Sect Leader”</Out>
              </span>
              <span className="font-semibold text-foreground">Historical</span>
              <span>
                court titles &amp; honorifics — <Zh>陛下</Zh> → <Out>“Your Majesty”</Out>,{" "}
                <Zh>太子</Zh> → <Out>“Crown Prince”</Out>
              </span>
              <span className="font-semibold text-foreground">Modern</span>
              <span>
                everyday register &amp; slang — <Zh>总裁</Zh> → <Out>“CEO”</Out>, names kept natural
              </span>
            </div>
          }
        >
          It reads the subtitles, works out the world type, and picks the right kind of
          terminology. You can change it if it guessed wrong.
        </Explainer>

        <Explainer
          title="It matches the tone you pick"
          example={
            <div className="space-y-1.5">
              <div>
                Input: <Zh>你算什么东西</Zh>
              </div>
              <div className="grid grid-cols-[max-content_1fr] gap-x-3 gap-y-1">
                <span className="font-semibold text-primary">Standard</span>
                <span>“Who do you think you are?”</span>
                <span className="font-semibold text-primary">Xianxia</span>
                <span>“What standing have you to address this one?”</span>
                <span className="font-semibold text-primary">Wuxia</span>
                <span>“And just who are you to challenge me?”</span>
                <span className="font-semibold text-primary">Comedic</span>
                <span>“Uh… and you are?”</span>
                <span className="font-semibold text-primary">Funny</span>
                <span>“Bro, you’re nobody.”</span>
              </div>
            </div>
          }
        >
          Same line, different delivery. Choose the register once and it carries across the whole
          batch.
        </Explainer>

        <Explainer
          title="It checks its own work as it goes"
          example={
            <>
              e.g. if lines start paraphrasing partway through an episode, it notices and
              retranslates just those, with a few lines around them for context.
            </>
          }
        >
          While translating it catches the AI dropping, merging or reordering lines. Then Verify
          checks each finished episode for lines that wander from the original meaning, including
          glossary terms that went missing — and redoes just the part that went wrong. Afterward
          you get a list of anything that still needs a look. It’s issues, not a score.
        </Explainer>

        <Explainer
          title="It leaves your typesetting alone"
          example={
            <>
              e.g. <Zh>{"{\\an8}这是什么?"}</Zh> becomes <Zh>{"{\\an8}"}</Zh>
              <Out>“What is this?”</Out> — the tag and styling are untouched.
            </>
          }
        >
          Only the dialogue is translated. Styles, timings and positioning come back as they were,
          and tags inside a line go back at the same relative spot. Embedded fonts and comment lines
          aren’t copied over.
        </Explainer>
      </div>
    </Section>
  );
}

// ── troubleshooting ───────────────────────────────────────────────────────────

const TROUBLE: {
  q: string;
  a: ReactNode;
  links: { to: string; label: string }[];
}[] = [
  {
    q: "Translate is greyed out",
    a: "Either no model is connected yet (or it still needs its API key), or no episode is ticked.",
    links: [
      { to: "/connections", label: "Models" },
      { to: "/episodes", label: "Episodes" },
    ],
  },
  {
    q: "Look up names online is greyed out",
    a: "It needs a model that can search the web. On the Models page, turn on Look up names online for your model, or add one that can — Anthropic, OpenAI and Google can search the web.",
    links: [{ to: "/connections", label: "Models" }],
  },
  {
    q: "I want to fix a translation by hand",
    a: (
      <>
        Open the episode from Episodes to fix single lines, or open the translated .ass file in
        your subtitle editor. To change how a name or term is translated in every episode, change
        it on the Glossary page, or open the whole glossary in your own editor with{" "}
        <span className="font-semibold text-foreground">
          Glossary ▸ ··· ▸ Open in your text editor
        </span>{" "}
        —
        Polygluttony reloads it automatically when you save.
      </>
    ),
    links: [{ to: "/glossary", label: "Glossary" }],
  },
  {
    q: "A batch failed or some lines came back wrong",
    a: "Polygluttony retries on its own and keeps the good lines. If it still fails, Episodes says what went wrong, and Logs shows the exact spot.",
    links: [{ to: "/episodes", label: "Episodes" }],
  },
];

function Troubleshooting() {
  return (
    <Section title="Troubleshooting">
      <div className="space-y-2">
        {TROUBLE.map((t) => (
          <div
            key={t.q}
            className="rounded-lg border border-border bg-[color:var(--card)] px-4 py-3"
          >
            <div className="flex items-center gap-2 text-[13px] font-semibold text-foreground">
              <span className="flex size-[18px] shrink-0 items-center justify-center rounded-full border border-[color:var(--color-alert)] bg-[color:var(--popover)] text-[11px] font-bold text-[color:var(--color-alert)]">
                ?
              </span>
              “{t.q}”
            </div>
            <p className="mt-1 pl-[26px] text-[12.5px] text-foreground/90">{t.a}</p>
            <div className="mt-1.5 pl-[26px] text-[11.5px]">
              {t.links.map((l, j) => (
                <span key={l.to}>
                  {j > 0 ? (
                    <span className="text-muted-foreground"> · </span>
                  ) : (
                    <span className="text-muted-foreground">→ </span>
                  )}
                  <Link to={l.to as never} className="font-medium text-primary hover:underline">
                    {l.label}
                  </Link>
                </span>
              ))}
            </div>
          </div>
        ))}
      </div>
    </Section>
  );
}

// ── page ──────────────────────────────────────────────────────────────────────

export function HelpPage() {
  return (
    <div className="flex h-full flex-col">
      <PageHeader title="Help" description="A quick guide to how Polygluttony works." />
      <div className="flex-1 overflow-auto p-5">
        <div className="mx-auto max-w-[640px] space-y-8 pb-10">
          <GettingStarted />
          <Shortcuts />
          <BehindTheScenes />
          <Troubleshooting />
        </div>
      </div>
    </div>
  );
}
