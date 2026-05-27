# KRIA Development Guide

## 1. Purpose

This guide defines the canonical engineering workflow for building and changing KRIA safely. It aligns local development practices with runtime authority, safety, and observability requirements.

Responsibilities:
- Standardize local setup, build, and test workflows.
- Define how subsystem changes are made without breaking authority boundaries.
- Keep engineering workflow deterministic and production-aligned.

Non-goals:
- This guide is not a product onboarding tutorial.
- It does not redefine subsystem architecture contracts.

## 2. Architecture Overview

Primary development surfaces:
- Rust workspace (`crates/*`) for core runtime.
- Docs under canonical subsystem paths.
- Scripts/automation in `scripts/`, `justfile`, and Docker artifacts.

Workflow architecture:
1. Implement change in bounded subsystem.
2. Validate with existing test/build/eval tools.
3. Update canonical doc if architecture contract changed.
4. Preserve single-source authority docs (no duplicate truth files).

## 3. Runtime Execution Flow

For engineering changes:
1. Start from target subsystem boundary and contracts.
2. Implement minimal coherent change across code + canonical docs.
3. Run subsystem and cross-cutting validations.
4. Evaluate safety, authority, and fallback behavior impacts.
5. Merge only when invariants and integration contracts hold.

Authority boundaries:
- Development process must not weaken runtime orchestration authority.
- Substrate integrations remain execution surfaces, never orchestration controllers.

## 4. Core Components

| Component | Contract |
|---|---|
| Cargo workspace | Build/test foundation for runtime crates |
| `justfile`/scripts | Standardized local and CI command entry points |
| Canonical docs | Architecture/source-of-truth contracts |
| Eval system | Regression signal for behavior and policy correctness |

Invariants:
- Architecture changes require corresponding canonical doc updates.
- No parallel conflicting canonical docs for same subsystem.
- Safety and execution authority behavior changes require explicit evaluation coverage.

## 5. Integration Contracts

| Integration | Development contract |
|---|---|
| Orchestration | Preserve runtime authority semantics in all changes |
| Providers/Tools | Keep schema/contracts stable or versioned |
| Memory | Maintain backward-compatible persistence/migration discipline |
| OpenClaw/n8n/MCP | Treat as substrate integrations with explicit boundaries |
| Hardware | Validate degradation/recovery behavior for resource-sensitive changes |
| Safety | Ensure policy/HITL/audit paths remain intact |
| GUI/Browser/Voice | Keep interaction-specific invariants and fallback behavior |

## 6. Failure Handling & Recovery

- Build/test failure: isolate to subsystem and restore green baseline.
- Contract regression: update implementation or docs to remove divergence.
- Integration flakiness: classify deterministic vs environmental failures.
- Risky behavior change: gate rollout behind explicit policy/config controls.

Recovery principle:
- Prefer small reversible changes over broad coupled refactors.

## 7. Performance & Constraints

Constraints:
- Rust build/test cycles can be heavy for full workspace operations.
- Local hardware impacts reproducibility for performance-sensitive paths.
- Integration tests may require optional external systems.

Tradeoff:
- Fast local iteration must not bypass critical safety/eval checks.

## 8. Security & Safety

Development controls:
- Do not introduce bypasses around policy/HITL/audit.
- Keep secrets out of code/docs and use configuration channels.
- Treat dangerous command execution paths as high-risk during testing.

Trust boundaries:
- External providers/integrations remain untrusted during development and testing.

## 9. Observability

Capture expectations:
- Changes should preserve or improve diagnostic signals.
- New critical flows should add actionable logs/metrics where missing.
- Eval and runtime diagnostics should remain correlated across subsystems.

## 10. Future Evolution

1. Strengthen subsystem-specific contributor playbooks linked to canonical docs.
2. Improve automated docs-consistency checks for authority-map drift.
3. Expand targeted regression suites for high-risk orchestration paths.
4. Keep workflow optimized for production safety and maintainability.
