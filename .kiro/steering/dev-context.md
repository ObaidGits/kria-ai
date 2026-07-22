---
inclusion: always
---

# KRIA Development Context (Permanent)

## Project stage
- KRIA is **under active development**, running **only on the owner's single laptop**.
- There are **no production users, no fleet, no remote deployment** yet.

## Risk posture (applies to all work)
- **Data loss is acceptable.** Losing all memory/DB data is not a big issue — no
  production data exists. Do not add heavy backup/restore ceremony, migration-safety
  gates, or "are you sure" friction *for the sake of protecting existing data*.
- **Deleting dead / deprecated / unused code is encouraged.** Remove cruft directly
  instead of preserving it "just in case." No need to keep deprecated shims,
  compatibility layers, or legacy paths around unless they are still actively used.
- **Prefer clean, direct changes over backward-compatibility scaffolding.** Since it's
  a single-dev pre-production codebase, breaking changes and hard cutovers are fine —
  no need for dual-run migrations, feature-flag rollback nets, or legacy coexistence
  purely to de-risk data/consumers.

## What this does NOT relax
- Still write correct, tested, well-structured code (this is the product being built).
- Still flag genuinely destructive *system/OS-level* actions (rm on non-project dirs,
  disk formatting, credential changes) — the relaxation is about **KRIA's own
  code and memory data**, not the machine.
- Still preserve architectural invariants and quality when implementing features.

## Practical implications
- When cleaning up: delete dead code rather than commenting it out or deprecating it.
- When refactoring memory/storage: a hard migration is fine; skip elaborate rollback
  machinery unless the owner explicitly asks for it.
- Optimize specs/plans for a **single-process, single-user, local laptop** reality
  first; treat multi-device / server / enterprise concerns as future-only.
- Treat the owner's laptop resources as a design and execution constraint. Prefer
  reuse, incremental work, and bounded concurrency over duplicate processes,
  terminals, scans, services, or heavyweight validation.
- "Single-process" describes the product's current deployment reality; it does not
  prohibit independent lightweight work in parallel when benefit exceeds resource cost.
- Resource efficiency must not weaken correctness, production quality, architectural
  invariants, testing rigor, or autonomous completion.
