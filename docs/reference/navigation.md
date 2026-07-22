# Navigation Architecture

> **Last Updated:** 2026-05-27
> **Scope:** Hybrid navigation model and the Modal-vs-Page decision framework
> (design.md §7, §10; Requirements 14, 18). Implementation:
> `ui/src/shell/HiddenDock.tsx`, `ui/src/palette/`, `ui/src/shell/router.ts`,
> `ui/src/shell/spaces/home/ContextualOrbit.tsx`.

---

## Hybrid Model (three registers, strictly separated roles)

The homepage uses three navigation registers with non-overlapping
responsibilities. There is exactly one system per role — no duplication.

| Register | Role | Behavior |
|---|---|---|
| **Hidden Dock** | Deliberate navigation | Invisible at rest; reveals on left-edge approach, Alt, ⌘K, pin, or keyboard/AT focus, over a dimmed Room. Canonical Space order and one-click switch preserved. Dismiss returns focus to its prior owner. |
| **Command Palette** | Searchable navigation | Owns global search, recent, and pinned; surfaces Space entries; the navigation authority for deep-linking. |
| **Contextual Orbit** | Ambient capability awareness | Partial/temporary light-points that appear on engagement from `homeFocusStore.orbit`; actionable points route only (never send/execute); static-dot fallback under reduced motion. |

Deep-linking, state restoration, back navigation, and recent/pinned all flow
through the typed router (`shell/router.ts`). Navigation depth is ≤2, and a Space
switch is ≤1 interaction from any entry point. Edge-reveal always has a
keyboard/palette equivalent (no hover-only affordances — see
`accessibility.md`).

Only the home surface (Converse, behind `home.presence.v2`) uses the Hidden
Dock. Every other Space keeps the standing rail governed by `kria-ui-redesign`;
the canonical Space order and one-click switch are identical either way.

---

## Modal-vs-Page Decision Framework (permanent guideline)

Goal: reduce unnecessary navigation and choose one surface deterministically.
This preserves the `kria-ui-redesign` invariants — **≤1 modal, no modal-on-modal,
single shared Inspector** — and is a permanent guideline, not a one-off.

Decision guidance (design.md §10):

- **Inline / in-place** when the change is a small, reversible edit to the
  current context.
- **Adaptive Context Surface / Focus** when it is a single contextual subject
  with ≤1 action.
- **Page (route)** when it is a distinct destination with its own state and
  deep-link.
- **Modal** only for a focused, blocking decision that must interrupt — and only
  one at a time.

**Developer rule:** one modal host, one Inspector host, one overlay-layer
manager. Reuse `ModalHost`, `InspectorHost`, and `overlayLayers`; new surfaces
register with these — no ad-hoc portals. Permission prompts route through the
Approval Center rather than stacking modals (RED steps the Core forward with a
single-line allow/deny; no modal-on-modal).

Every current modal and page is classified against this framework as part of the
cross-page cascade (design.md §17), leaving a single modal host, a single
Inspector, and a single capability-awareness system (Orbit).

---

## Related Documentation

- `../architecture/presence-homepage-runtime.md` — navigation in the runtime map.
- `../decisions/adr/005-hybrid-navigation.md` — hybrid navigation decision.
- `../decisions/adr/006-modal-vs-page-framework.md` — Modal-vs-Page decision.
- `accessibility.md` — keyboard/AT reachability of the Hidden Dock.
