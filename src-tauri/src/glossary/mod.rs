//! Glossary extraction and management.
//!
//! Ports the Python `glossary/` package: collect dialogue across files, detect
//! world type (xianxia / wuxia / historical / modern), extract the six-category
//! glossary in parallel via the LLM, dedupe/merge, and optionally normalize and
//! personalize. Supports injecting reference terminology.
//!
//! Current submodules:
//! - `model`       — `GlossaryDoc` IPC shape, term ops (merge, dedupe, parse)
//! - `io`          — atomic pretty-printed save + load, backup and import for the glossary JSON file
//! - `diff`        — pure diff between two glossary snapshots (`GlossaryDiff`)
//! - `meta`        — sidecar metadata (term origins, scanned files, history) and its IPC shapes
//! - `world_detector` — keyword-heuristic world-type detection (no LLM)
//! - `reference`   — reference terminology types, cache, ref/ discovery, async extractor (O11)
//! - `prompts`     — prompt assembly for extraction, normalize, and personalize passes
//! - `build`       — O10 build orchestrator (`build_glossary`) + cross-file batch helper
//! - `normalize`   — per-category normalize pass (`normalize_pass`, O12)
//! - `personalize` — personalization pass (`personalize_pass`, build step 8)
//! - `run`         — run manager (single glossary-op slot, exclusivity with translation)
//! - `watch`       — O15 glossary.json file watcher (`FileChanged` events)

pub mod build;
pub mod diff;
pub mod io;
pub mod meta;
pub mod model;
pub mod normalize;
pub mod personalize;
pub mod prompts;
pub mod reference;
pub mod run;
pub mod watch;
pub mod world_detector;
