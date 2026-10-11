import type { ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { PageHeader } from "@/components/page-header";

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

const STEPS = [
  {
    n: 1,
    title: "Connect an AI model",
    body: "Polygluttony needs an AI model to build your glossary and translate. Pick a provider, paste your API key and press Connect — or use a model running on your computer. One model does every step; you can give steps their own models later.",
    to: "/connections",
    link: "Open Connections",
  },
  {
    n: 2,
    title: "Open a folder",
    body: "Point Polygluttony at a folder of subtitles. It scans them and shows what’s already translated and what isn’t.",
    to: "/",
    link: "Choose a folder",
  },
  {
    n: 3,
    title: "Build a glossary",
    optional: true,
    body: "A glossary keeps names and terms consistent across every episode. Skip it and translate right away if you like.",
    to: "/glossary",
    link: "Open Glossary",
  },
  {
    n: 4,
    title: "Translate",
    body: "Pick your files and press Translate. Polygluttony checks its own work as it goes and flags anything that looks off.",
    to: "/translate",
    link: "Open Translate",
  },
] as const;

function GettingStarted() {
  return (
    <Section title="Getting started">
      <div className="space-y-2">
        {STEPS.map((s) => (
          <div
            key={s.n}
            className="flex items-start gap-3 rounded-lg border border-border bg-[color:var(--card)] px-3.5 py-3"
          >
            <span className="mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-full bg-[color:var(--popover)] text-[12px] text-primary">
              {s.n}
            </span>
            <div className="min-w-0 flex-1">
              <div className="text-[13px] font-semibold text-foreground">
                {s.title}
                {"optional" in s && s.optional ? (
                  <span className="ml-1.5 text-[11px] font-normal text-muted-foreground">
                    — optional
                  </span>
                ) : null}
              </div>
              <p className="mt-0.5 text-[12.5px] text-foreground/90">{s.body}</p>
              <Link
                to={s.to as never}
                className="mt-1.5 inline-block text-[11.5px] font-medium text-primary hover:underline"
              >
                → {s.link}
              </Link>
            </div>
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
          Build a glossary once — or pull one from subtitles you’ve already translated — and
          Polygluttony hands those agreed translations to the AI on every episode.
        </Explainer>

        <Explainer
          title="It tunes to the genre"
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
          It reads the subtitles, works out the genre, and prioritizes the right kind of
          terminology. You can override the guess.
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
              e.g. if the lines start paraphrasing partway through a batch, it flags where the drift
              began and retranslates from there.
            </>
          }
        >
          While translating it watches for the AI drifting off the original meaning, dropping or
          merging lines, or breaking glossary consistency — and re-does just the part that went
          wrong. Afterward you get a list of anything still worth a look. It’s issues, not a score.
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
          Only the dialogue is translated. Positioning, styling, fonts, and effects come back
          exactly as they were.
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
    a: "Either no AI model is connected yet (or it still needs its API key), or every file in the folder is already translated.",
    links: [
      { to: "/connections", label: "Connections" },
      { to: "/project", label: "Project" },
    ],
  },
  {
    q: "Look up names online is disabled",
    a: "It needs a model that can search the web. In Connections, turn on Web search for your model, or add one that can — Anthropic, OpenAI and Google can search.",
    links: [{ to: "/connections", label: "Connections" }],
  },
  {
    q: "I want to fix a translation by hand",
    a: (
      <>
        Open the glossary in your own editor with{" "}
        <span className="font-semibold text-foreground">Glossary ▸ Open in editor</span> —
        Polygluttony reloads it automatically when you save.
      </>
    ),
    links: [{ to: "/glossary", label: "Glossary" }],
  },
  {
    q: "A batch failed or some lines came back wrong",
    a: "Polygluttony retries automatically and keeps the good lines. If it still fails, the log panel in Translate shows the exact spot.",
    links: [{ to: "/translate", label: "Translate" }],
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
          <BehindTheScenes />
          <Troubleshooting />
        </div>
      </div>
    </div>
  );
}
