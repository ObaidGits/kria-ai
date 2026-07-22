# ADR: Hybrid Navigation — Hidden Dock + Palette + Orbit

Status: Accepted
Date: 2026-05-27
Owner: Homepage Presence Redesign
Scope: `ui/src/shell/HiddenDock.tsx`, `ui/src/palette/`,
`ui/src/shell/spaces/home/ContextualOrbit.tsx`, `ui/src/shell/router.ts`
Requirements: 6 (Orbit), 7 (Hidden Dock), 14 (navigation continuity)

## 1) Decision

Homepage navigation uses **three registers with strictly separated roles**:

- **Hidden Dock** — deliberate navigation. Invisible at rest; reveals on
  left-edge approach, Alt, ⌘K, pin, or keyboard/AT focus, over a dimmed Room.
  Preserves the canonical Space order and one-click switch.
- **Command Palette** — searchable navigation. Owns global search, recent,
  pinned, and is the deep-link/back/state-restore authority.
- **Contextual Orbit** — ambient capability awareness. Temporary light-points
  from `homeFocusStore.orbit` that route only, never send/execute.

There is exactly **one system per role**; any prior "sparks"/duplicate
capability-awareness affordance is removed. Navigation depth is ≤2 and a Space
switch is ≤1 interaction from any entry point.

## 2) Context

A presence homepage cannot carry a persistent standing dock without breaking the
resting calm, yet navigation must stay one-click and fully keyboard/AT
reachable. The redesign also inherited overlapping capability-hint mechanisms
(Orbit and "sparks") that duplicated the same role.

## 3) Rationale

- **Calm at rest, reachable on intent.** Recede the Dock visually while keeping
  it in the accessibility tree and tab order, so keyboard/AT users lose nothing.
- **Role separation prevents ambiguity.** Deliberate (Dock) vs searchable
  (Palette) vs ambient (Orbit) keeps each register predictable and prevents two
  systems from claiming the same job.
- **Routing-only Orbit.** Ambient hints that route (never execute) keep the
  runtime-authority invariant intact.

## 4) Enforcement

`HiddenDock` recedes via `transform` + `opacity` only — never `display:none` /
`visibility:hidden` — and paints on `:focus-within` (asserted in
`ui/src/shell/HiddenDock.test.tsx`). Only the home surface uses the Hidden Dock;
other Spaces keep the standing rail (`AppShell` chooses per route + flag). Orbit
points are classified route-only; the navigation-architecture test asserts a
single capability-awareness system.

## 5) Consequences

- **Positive:** resting calm preserved; navigation stays one-click and fully
  accessible; a single, predictable model per role.
- **Negative / limitation:** first-time mouse-only users may not discover the
  edge-reveal Dock immediately; mitigated by palette prominence, a one-time
  hint, and Orbit teaching.

## 6) Alternatives Considered

- **Keep a persistent standing dock on the homepage.** Rejected: breaks resting
  calm; contradicts presence-first.
- **Palette-only navigation.** Rejected: loses the fast, deliberate one-click
  Space switch and spatial memory.
- **Keep both Orbit and sparks.** Rejected: two systems for one role; removed.
