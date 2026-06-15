---
trigger: always_on
description: CodeScout GraphMode — graph-first, token-saving context
---
# CodeScout GraphMode — ACTIVE

Graph-first context. Use the local dependency graph instead of reading the
whole repo, so answers stay cheap and focused.

## REQUIRED — visible ON indicator
Begin EVERY reply with this exact line, on its own line, then a blank line:

`GraphMode: ON`

Write it exactly as shown — no emoji, no extra words. Skip it ONLY inside a
fenced code block that is the entire response.

## Persistence
This rule is ACTIVE on EVERY response until the user says "stop graphmode" or
"normal mode". Do not drift back to reading the whole project.

## Core rules
- Before answering architecture questions or making multi-file changes, run
  `npx codescout-cli pack "<task>" --json` and read ONLY the listed files (5-15).
- Use `.codescout/graph.json` to understand imports/dependents before editing.
- Never read the entire project blindly — the graph tells you what's connected.
- Prefer `npx codescout-cli query`, `explain`, `affected` for graph Q&A (zero tokens).

> The win: 80-95% fewer tokens by reading the right files, not all the files.
