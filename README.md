<div align="center">

<img src="assets/welcome.png" alt="Polygluttony" width="860">

# Polygluttony

**LLM-powered subtitle translation for donghua & anime — built to protect the things that break.**

[Website](https://pg.blyat.uk) · [Download](../../releases/latest) · [Documentation](https://github.com/blyat-uk/polygluttony/wiki) · macOS · Windows · Linux

</div>

---

Polygluttony translates `.ass` subtitle files with an LLM while guarding against the failure modes that wreck naive machine translation. Point it at a folder, connect a model (Anthropic, OpenAI, Google, a local model through Ollama or LM Studio, or any OpenAI- or Anthropic-compatible endpoint), optionally build a glossary, and run — watching live, honest telemetry the whole way.

## Why it's different

- **Line markers & partial-failure recovery** — every line is tracked, so when a model drops, merges, or reorders lines, Polygluttony detects exactly where it broke and salvages the correct prefix instead of failing the whole batch.
- **Drift detection** — after each episode, a five-signal weighted check and an AI spot-check catch translations wandering off the source, and only the lines that drifted are retranslated.
- **Your typesetting stays put** — override tags like `{\pos}` and `{\an8}` are lifted out before translating and put back after, and styles, timings and script info are copied from your file. Only the dialogue is translated.
- **Cross-episode glossary** — for Chinese sources, a six-category glossary, with auto-detected world type (xianxia / wuxia / historical / modern), keeps names and terms consistent across a whole season.
- **Verification, not a score** — every file checks its own work and surfaces an actionable issue list, never a number.
- **Mission-control UI** — a single window with live, batched telemetry: watch batches land and terms stream into the glossary, see retranslations as they happen, or open any episode in its own window and watch its lines arrive.

## Download

Grab the latest build for your OS from the [**Releases**](../../releases/latest) page:

| OS | File |
|---|---|
| Windows 10/11 (x64) | `Polygluttony-vX.Y.Z-windows.exe` (installer, no admin rights needed) |
| macOS (Apple Silicon) | `Polygluttony-vX.Y.Z-macos.zip` |
| Linux x86_64 | `Polygluttony-vX.Y.Z-linux.AppImage` |

> These builds aren't signed with a paid developer certificate, so the OS warns on first launch:
> - **macOS** — the first launch is blocked. Open **System Settings → Privacy & Security**, scroll to the bottom, and click **Open Anyway**, then launch again and confirm. (Right-click → Open no longer works on recent macOS.)
> - **Windows** — on the SmartScreen prompt, choose **More info → Run anyway**.
> - **Linux** — `chmod +x Polygluttony-*.AppImage`, then run it.
>
> More per system: **[Platform notes](https://github.com/blyat-uk/polygluttony/wiki/Platform-Notes)**.

## Documentation

Everything lives in the **[wiki](https://github.com/blyat-uk/polygluttony/wiki)**. New here? Start with the first two.

| Page | Covers |
|---|---|
| 🚀 **[Getting started](https://github.com/blyat-uk/polygluttony/wiki/Getting-Started)** | Install, connect a model, translate your first episode |
| 🧭 **[How it works](https://github.com/blyat-uk/polygluttony/wiki/How-It-Works)** | What happens between opening a folder and getting translated files |
| 🔌 [Connecting a model](https://github.com/blyat-uk/polygluttony/wiki/Connecting-a-Model) | Providers, API keys, local models, one model per step, tuning |
| ⚖️ [Choosing a model](https://github.com/blyat-uk/polygluttony/wiki/Choosing-a-Model) | Which model for which job, and what it costs you |
| 📁 [Folders & files](https://github.com/blyat-uk/polygluttony/wiki/Folders-and-Files) | What gets read, what gets written, what happens to your tags |
| 📖 [The glossary](https://github.com/blyat-uk/polygluttony/wiki/Glossary) | Keeping names and terms consistent across episodes |
| ▶️ [Translating](https://github.com/blyat-uk/polygluttony/wiki/Translating) | Running a translation and watching an episode live |
| ✅ [Reading the results](https://github.com/blyat-uk/polygluttony/wiki/Reading-the-Results) | What "Needs a look" means, and what to do about it |
| 🛡️ [What it protects](https://github.com/blyat-uk/polygluttony/wiki/What-It-Protects) | The ways AI translation goes wrong, and how they're caught |
| 🔒 [Privacy & your data](https://github.com/blyat-uk/polygluttony/wiki/Privacy-and-Your-Data) | What gets sent where, and where everything is stored |
| 🩺 [Troubleshooting](https://github.com/blyat-uk/polygluttony/wiki/Troubleshooting) | Fixes for the usual problems |

## Build from source

Requires [Bun](https://bun.sh) and [Rust](https://rustup.rs) (stable), plus the [Tauri prerequisites](https://tauri.app/start/prerequisites/) for your OS.

```bash
bun install
bun tauri dev      # run with hot reload
bun tauri build    # produce a distributable bundle
```

Tests, generated code and the source layout: [Building from source](https://github.com/blyat-uk/polygluttony/wiki/Building-from-Source).

## Stack

Tauri 2 · Rust · React 19 · TypeScript · Tailwind v4 · TanStack Router/Query.

## License

[MIT](LICENSE).
