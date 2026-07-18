# Context Package

## Task

Audit legacy ui/src/App.tsx and imported components/stores against active AppShell reachable from ui/src/index.tsx. Classify migrated, redesigned, intentional removal, lost. Find unreachable legacy files, duplicate stores/CSS/tests/assets/dependencies, shims/placeholders. Need exact graph references and symbols.

## Detected Stack

- typescript
- vitest

## Relevant Files

- **src/stores/index.ts** — tags: [index, src, stores, ts], importance: 483, role: seed, hop: 0
- **src/kit/index.ts** — tags: [index, kit, src, ts], importance: 410, role: seed, hop: 0

## Warnings

- src/stores/index.ts has fan-in > 10 (91 dependents)
- src/kit/index.ts has fan-in > 10 (74 dependents)

## Token Budget

- Point estimate: 1768 tokens
- Range: 1414 – 2122 tokens
- Reduction: 100% vs full project
