---
inclusion: always
description: Minimum tokens with exact accuracy — locate before reading, verify before claiming
---
# Token Discipline (always-on)

Owner's #1 goal: **minimum token usage with exact accuracy.** Cheap is only good if
it is still correct.

## Locate before reading
- Find the exact lines first: symbol/AST search, or ripgrep scoped with `--include`
  and a result cap. Then read **only that line range**.
- Never read a whole large file to answer a narrow question.
- Use the code maps: `.codescout/RUST_MAP.md` for Rust, `codescout pack` for `ui/`.

## Never dump into context
`.codescout/graph.json` (2.6 MB) · `.codescout/analysis-meta.json` ·
`.codescout/importance.json` · `.codescout/tags.json` · `.codescout/rust-map.json` ·
`Cargo.lock` · `ui/package-lock.json` · anything under `target/`, `models/`, or `vendor/`.
Query these with `jq`/`python3` and print only the answer.

## Push bulk to a sub-agent
Wide searches, log dumps, reading many files, long build output — delegate it so only
the distilled result lands in the main context.

## Verify cheaply
- `cargo check -p <crate>` — not a full-workspace build
- `cargo test -p kria-core <test_name>` — focused test
- `cargo clippy -p <crate>` · `cd ui && npm run test:run`
- Full-workspace or release builds only at explicit phase gates.
Reason: this is one low-RAM laptop; kria-core alone is ~517k lines.

## Accuracy overrides brevity
- Never guess a path, signature, or behaviour to save tokens. Verify it with the
  cheapest command that actually proves it.
- Say what you verified and what you assumed. Do not present an assumption as a fact.
