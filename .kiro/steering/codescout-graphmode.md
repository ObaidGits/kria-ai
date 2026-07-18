---
inclusion: always
description: CodeScout GraphMode — graph-first, token-saving context on every turn
---
# CodeScout GraphMode — ACTIVE

Graph-first context. Use the local dependency graph instead of reading the
whole repo, so answers stay cheap and focused.

## REQUIRED — visible ON indicator
Show this exact indicator on every reply:

`GraphMode: ON`

When Caveman Mode is active, place this indicator after `Caveman mode: ON` and
its blank line. Otherwise begin the reply with this indicator and a blank line.
Write it exactly as shown — no emoji, no extra words. Skip indicators ONLY inside
a fenced code block that is the entire response.

## Persistence
This rule is ACTIVE on EVERY response until the user says "stop graphmode" or
"normal mode". Do not drift back to reading the whole project.

## Core rules
- For architecture, dependency-impact, or multi-file work, use
  `npx codescout-cli pack "<task>" --json` when the graph adds value and no fresh
  equivalent result is already available. If it fails or is unavailable, use
  focused file/search tools instead of retrying wastefully.
- Explicit user scope and directly named files take priority. Read only the
  minimum connected files needed; do not enforce an arbitrary file-count quota.
- Use `.codescout/graph.json`, `query`, `explain`, or `affected` to understand
  imports/dependents without broad repository scans.
- Reuse graph and repository knowledge while relevant files remain unchanged.
- Never read the entire project blindly — stop gathering once context is sufficient.

> The win: 80-95% fewer tokens by reading the right files, not all the files.
