---
inclusion: always
description: CodeScout GraphMode — map-first context routing per language (token-saving)
---
# CodeScout GraphMode — ACTIVE

Map-first context. Find the few files that matter, never read the repo blindly.

## REQUIRED — visible ON indicator
Begin EVERY reply with this exact line, on its own line, then a blank line:

`GraphMode: ON`

Write it exactly as shown — no emoji, no extra words. Skip it ONLY inside a
fenced code block that is the entire response.

## Persistence
ACTIVE on EVERY response until the user says "stop graphmode" or "normal mode".

## Route by language — no single map covers this repo
- **Rust (`crates/`, ~660k lines, the bulk)** → read `.codescout/RUST_MAP.md` FIRST:
  crate table, hotspots ranked by dependents, every `#[tauri::command]`, and
  per-module public-symbol counts. Locate the module, then open **only that file's
  relevant line range**. Machine-readable: `.codescout/rust-map.json`.
  Refresh: `python3 scripts/rust-map.py` (1.7s, no LLM).
  ⚠️ `.codescout/graph.json` contains **zero `.rs` files** — never use it for Rust.
- **TypeScript (`ui/`)** → `codescout pack "<task>" --json`, then read only the files
  it lists. Imports/dependents: `.codescout/graph.json`. Refresh: `codescout map`.
- **Python (`sidecars/`)** → scoped ripgrep with `--include`; no map covers it.

Call `codescout` directly (installed globally) — not `npx`. Valid subcommands:
`init`, `map`, `pack`, `security`, `clean`, `doctor`, `trash`. There is no
`query`, `explain`, or `affected` subcommand.

> The win: read the 5–15 files that matter instead of all ~1800.
