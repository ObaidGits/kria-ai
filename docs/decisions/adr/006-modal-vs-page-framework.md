# ADR: Modal vs Page Decision Framework

Status: Accepted
Date: 2026-05-27
Owner: Homepage Presence Redesign
Scope: Global surface selection — `ModalHost`, `InspectorHost`, `overlayLayers`,
Approval Center routing; classified across all Spaces (design.md §10, §17)
Requirements: 10 (permission UX), 18 (modal/page discipline)

## 1) Decision

Adopt a **permanent Modal-vs-Page decision framework** to choose one surface
deterministically and reduce unnecessary navigation:

- **Inline / in-place** — small, reversible edits to the current context.
- **Adaptive Context Surface / Focus** — a single contextual subject, ≤1 action.
- **Page (route)** — a distinct destination with its own state and deep-link.
- **Modal** — only a focused, blocking decision that must interrupt, and only
  **one at a time**.

Hard invariants (carried over from `kria-ui-redesign`): **≤1 modal, no
modal-on-modal, a single shared Inspector.** Developer rule: one modal host, one
Inspector host, one overlay-layer manager; new surfaces register with these —
no ad-hoc portals.

## 2) Context

Without a shared rule, surfaces drift into stacked modals and duplicate
inspectors, which fragment focus and break the calm/keyboard-first contract.
Permission prompts in particular tend to become modal-on-modal.

## 3) Rationale

- **Determinism.** A single rule removes per-feature guesswork about which
  surface to use.
- **Focus integrity.** One modal, one Inspector, one overlay manager keeps focus
  management and AT behavior predictable.
- **Permission calm.** Routing permission through the Approval Center (RED steps
  the Core forward with a single-line allow/deny) avoids modal stacking while
  reusing `approvalStore`.

## 4) Enforcement

Every current modal/page is classified against the framework during the
cross-page cascade (design.md §17). Reuse of `ModalHost`, `InspectorHost`, and
`overlayLayers` is required; the homepage permission UX routes to the Approval
Center rather than opening a second modal. See `../reference/navigation.md`.

## 5) Consequences

- **Positive:** consistent surface choice; no nested modals; single Inspector;
  calmer permission flow.
- **Negative / limitation:** some interactions that were quick ad-hoc modals now
  route to a page or the Focus surface; this is the intended trade to reduce
  navigation noise and focus fragmentation.

## 6) Alternatives Considered

- **Case-by-case surface choice.** Rejected: produces modal sprawl and
  inconsistent focus behavior.
- **Modal-heavy permission prompts.** Rejected: modal-on-modal breaks the
  invariant and the calm contract.
