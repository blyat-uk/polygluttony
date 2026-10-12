<div align="center">

<img src="assets/welcome.png" alt="Polygluttony" width="860">

# Polygluttony

**LLM-powered subtitle translation for donghua & anime — built to protect the things that break.**

[Website](https://pg.blyat.uk) · [Download](https://github.com/blyat-uk/polygluttony/releases/latest) · [Documentation](https://github.com/blyat-uk/polygluttony/wiki)

[![Latest release](https://img.shields.io/github/v/release/blyat-uk/polygluttony?label=release&color=e1a636)](https://github.com/blyat-uk/polygluttony/releases/latest)
![Windows · macOS · Linux](https://img.shields.io/badge/platform-Windows%20%C2%B7%20macOS%20%C2%B7%20Linux-555)
[![MIT](https://img.shields.io/github/license/blyat-uk/polygluttony?color=555)](LICENSE)
[![Docs](https://img.shields.io/badge/docs-wiki-e1a636)](https://github.com/blyat-uk/polygluttony/wiki)

</div>

---

Polygluttony translates `.ass` subtitle files with the AI model you connect, while guarding against the ways machine translation usually goes wrong. Open a folder, connect a model, optionally build a glossary, and translate, watching every batch land as it happens.

## Why it's different

- **Line markers & partial-failure recovery** — every line is tracked, so when a model drops, merges, or reorders lines, Polygluttony detects exactly where it broke and salvages the correct prefix instead of failing the whole batch.
- **Drift detection** — after each episode, a five-signal weighted check and an AI spot-check catch translations wandering off the source, and only the lines that drifted are retranslated.
- **Your typesetting stays put** — override tags like `{\pos}` and `{\an8}` are lifted out before translating and put back after, and styles, timings and script info are copied from your file. Only the dialogue is translated.
- **Cross-episode glossary** — for Chinese sources, a six-category glossary, with auto-detected world type (xianxia / wuxia / historical / modern), keeps names and terms consistent across a whole season.
- **Verification, not a score** — every file checks its own work and surfaces an actionable issue list, never a number.
- **Your choice of model** — Anthropic, OpenAI, Google, a local model through Ollama or LM Studio, or any OpenAI- or Anthropic-compatible endpoint. One model for everything, or a different one per step.

## Download

Get the latest build from the [releases page](https://github.com/blyat-uk/polygluttony/releases/latest):

| OS | File |
|---|---|
| Windows 10/11 (x64) | `Polygluttony-vX.Y.Z-windows.exe` (installer, no admin rights needed) |
| macOS (Apple Silicon) | `Polygluttony-vX.Y.Z-macos.zip` |
| Linux x86_64, any distribution | `Polygluttony-vX.Y.Z-linux.AppImage` |

The builds aren't signed with a paid developer certificate, so the first launch takes one extra step on Windows and macOS: see [Platform notes](https://github.com/blyat-uk/polygluttony/wiki/Platform-Notes).

## Documentation

Everything lives in the **[wiki](https://github.com/blyat-uk/polygluttony/wiki)**. New here? Start with the first two.

| Page | Covers |
|---|---|
| 🚀 **[Getting started](https://github.com/blyat-uk/polygluttony/wiki/Getting-Started)** | Install, connect a model, translate your first episode |
| 💻 **[Platform notes](https://github.com/blyat-uk/polygluttony/wiki/Platform-Notes)** | The first launch on Windows, macOS and Linux |
| 🧭 [How it works](https://github.com/blyat-uk/polygluttony/wiki/How-It-Works) | What happens between opening a folder and getting translated files |
| 🔌 [Connecting a model](https://github.com/blyat-uk/polygluttony/wiki/Connecting-a-Model) | Providers, API keys, local models, one model per step, tuning |
| ⚖️ [Choosing a model](https://github.com/blyat-uk/polygluttony/wiki/Choosing-a-Model) | Which model for which job, and what it costs you |
| 📁 [Folders & files](https://github.com/blyat-uk/polygluttony/wiki/Folders-and-Files) | What gets read, what gets written, what happens to your tags |
| 🌐 [Languages](https://github.com/blyat-uk/polygluttony/wiki/Languages) | The 15 languages, and what's different about Chinese → English |
| 📖 [The glossary](https://github.com/blyat-uk/polygluttony/wiki/Glossary) | Keeping names and terms consistent across episodes |
| ▶️ [Translating](https://github.com/blyat-uk/polygluttony/wiki/Translating) | Running a translation and watching an episode live |
| 🎭 [Tones](https://github.com/blyat-uk/polygluttony/wiki/Tones) | Standard, xianxia, wuxia, comedic and funny |
| ✅ [Reading the results](https://github.com/blyat-uk/polygluttony/wiki/Reading-the-Results) | What "Needs a look" means, and what to do about it |
| ✏️ [Custom prompts](https://github.com/blyat-uk/polygluttony/wiki/Custom-Prompts) | Changing exactly what the AI is told |
| 🛡️ [What it protects](https://github.com/blyat-uk/polygluttony/wiki/What-It-Protects) | The ways AI translation goes wrong, and how they're caught |
| 🔒 [Privacy & your data](https://github.com/blyat-uk/polygluttony/wiki/Privacy-and-Your-Data) | What gets sent where, and where everything is stored |
| 🩺 [Troubleshooting](https://github.com/blyat-uk/polygluttony/wiki/Troubleshooting) | Fixes for the usual problems |
| 🗑️ [Uninstalling](https://github.com/blyat-uk/polygluttony/wiki/Uninstalling) | Removing the app and everything it stored |
| 🛠️ [Building from source](https://github.com/blyat-uk/polygluttony/wiki/Building-from-Source) | Run, test and package Polygluttony yourself |

## License

[MIT](LICENSE)
