# ADR: Homepage — Presence-First Pivot from AI-OS

Status: Accepted
Date: 2026-05-27
Owner: Homepage Presence Redesign
Scope: KRIA home surface (`ui/src/shell/spaces/home/`, mounted from
`ui/src/shell/spaces/ConverseSpace.tsx` behind `home.presence.v2`)
Requirements: 1 (Room/atmosphere), 12/24 (Focus engine), 22.3 (docs/ADRs)

## 1) Decision

The KRIA home surface is a **presence homepage**, not an application dashboard or
"AI-OS" control panel. At rest it is a light in a dark Room that speaks at most
one line. It is composed of exactly six elements, each answering one question:
Core (who is here?), Voice Line + Adaptive Context Surface (what matters now?),
Composer + Chips (what can I do?), Contextual Orbit (what can KRIA do for me?),
Hidden Dock / Palette (where else can I go?), and Trust Indicator (can I trust
it?). No element answers two questions; none is decorative. Emptiness is solved
with light, depth, and reaction — never with placeholder widgets or empty cards.

The homepage is a **presentation + pure read-model layer only**. The Focus engine
(`homeFocusStore`) fuses existing signals and emits one `FocusFrame`; it performs
no orchestration, no tool calls, and no send/execute.

## 2) Context

The prior direction (`kria-ui-redesign`) treated the home surface as a
Core-forward Converse empty state within a dashboard-shaped shell. Product review
found that a dashboard of panels reads as "software" and fails the five-second
"presence, not page" test on ordinary Linux hardware. KRIA's value is a
local-first assistant that is *present*, not a console of widgets.

## 3) Rationale

- **Felt presence beats feature density.** A calm room with one light and one
  line communicates a living assistant; a grid of cards communicates a tool.
- **Runtime-authority invariant preserved.** Keeping the homepage a pure
  read-model means it cannot become a shadow planner. `TurnGate` remains the sole
  top-level planner; the homepage never bypasses policy, HITL, or audit. This is
  asserted by `ui/src/shell/spaces/home/authority.test.ts`.
- **Honesty over theater.** Every capability tier yields a valid resting frame;
  cold start is truthfully generic with no fabricated personalization.

## 4) Enforcement

Behind `home.presence.v2`, `ConverseSpace` routes the empty home surface to
`HomeSpace`. The guardrail lint/test (`ui/src/shell/spaces/home/guardrails.ts`,
spec `guardrails.md`) enforces the resting-calm invariants: single Adaptive
Context Surface, ≤3 chips, no accent on the Room base, `coreHint` never written.
The Focus engine's purity and no-send/no-navigate behavior is scanned in tests.

## 5) Consequences

- **Positive:** the home surface passes the five-second presence test; the
  read-model boundary keeps AI authority contained; the surface is never blank.
- **Negative / limitation:** discoverability of hidden controls (Dock, chips)
  depends on the palette, a one-time hint, and Orbit teaching. Accepted and
  mitigated, not eliminated.
- **Rollback:** flipping `home.presence.v2` OFF restores the legacy Converse
  empty state with no rebuild.

## 6) Alternatives Considered

- **Keep the dashboard/AI-OS empty state.** Rejected: reads as software; fails
  the presence test.
- **Make the homepage an active agent that acts on ambient signals.** Rejected:
  violates the runtime-authority invariant; the homepage stages/route only.
