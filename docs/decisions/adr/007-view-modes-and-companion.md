# ADR: View Modes — Canonical Set + Companion

Status: Accepted
Date: 2026-05-27
Owner: Homepage Presence Redesign
Scope: `ui/src/shell/windowModeManager` / `WindowModeSwitch`,
`ui/src/shell/viewModeResponsibilityMatrix.ts`,
`ui/src/shell/spaces/home/CompanionEmber.tsx`, `readingMode.ts`
Requirements: 11 (Reading Mode), 13 (view modes), 15 (Companion)
Supersedes: `kria-ui-redesign` window-mode naming (Compact/Standard/Immersive)

## 1) Decision

The canonical window-mode set is **Immersive / Standard / Mini / Companion**.
This supersedes the earlier Compact/Standard/Immersive naming. Definitions:

- **Immersive** — maximal breathing room; chrome recedes; canvas/conversation
  leads.
- **Standard** — the default composition.
- **Mini** — the compact companion window (replaces "Compact").
- **Companion** — a detached floating **ember** that inherits Core state, stays
  present, brightens only for real needs, and supports click-to-talk.

The Core is the **continuity anchor** across transitions. Shared state (thread,
Core state, draft, Focus subject) is preserved across every mode change.
`shell/viewModeResponsibilityMatrix.ts` fixes exactly what shows/hides/persists
per mode.

**Reading Mode** is a homepage macro-state (not a window mode): on first send the
homepage recedes into depth (never a page/dock swap), the Room hard-dims behind a
near-solid AA-contrast reading backing, and it reverses when the conversation
empties.

## 2) Context

`kria-ui-redesign` shipped a Compact/Standard/Immersive window-mode axis and a
`MiniCompanions`/`detachableSurfaces` mechanism. The presence redesign needs a
persistent, low-cost detached presence (the ember) and a consistent naming that
matches the new model, without contradictory terminology across code and docs.

## 3) Rationale

- **One canonical vocabulary.** Contradictory naming (Compact vs Mini,
  overlapping Companion semantics) causes code/doc drift; a single set removes
  it.
- **Continuous, not swapped.** Anchoring on the Core and preserving shared state
  makes transitions feel continuous rather than like page reloads. Reading Mode
  as a depth-recession (not a route swap) keeps `ConverseSpace` mounted across
  the empty↔reading boundary.
- **Cheap presence.** The Companion ember is a cheap 2D surface that inherits
  Core state, so presence persists without heavy rendering.

## 4) Enforcement

The mode axis is reconciled to the canonical set in `windowModeManager` /
`WindowModeSwitch`, with contradictory naming removed from code and docs. The
responsibility matrix is unit-tested (`viewModeResponsibilityMatrix.test.ts`).
The ember falls back to in-app/tray when the compositor restricts always-on-top
surfaces.

## 5) Consequences

- **Positive:** consistent naming; state survives transitions; a persistent,
  low-cost detached presence.
- **Negative / limitation:** always-on-top Companion behavior is
  compositor-dependent on Linux; the in-app/tray fallback is the honest
  degradation.

## 6) Alternatives Considered

- **Keep Compact/Standard/Immersive.** Rejected: no detached-presence concept;
  naming conflicts with the new model.
- **Reading Mode as a separate route/page.** Rejected: a page swap breaks
  conversation continuity and Core anchoring.
