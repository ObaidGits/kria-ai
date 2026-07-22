# Accessibility Contract — Presence Homepage

> **Last Updated:** 2026-05-27
> **Scope:** Homepage-presence accessibility contract (design.md §15,
> Requirement 21). Tests: `ui/src/shell/spaces/home/*Accessibility*.test.tsx`,
> `ui/e2e/accessibility.spec.ts`.

---

## Keyboard Operability

Full keyboard operability with visible focus is required for every homepage
element: Core, Voice Line, Composer, Chips, Orbit, Adaptive Context Surface,
Hidden Dock, Trust Indicator, and the Companion ember. Tab order is stable.

There are **no hover-only or cursor-only affordances.** Every edge-reveal or
hover interaction has an Alt / palette / keyboard equivalent. The Hidden Dock is
the canonical example: it is invisible at rest but never removed from the
accessibility tree or tab order — it recedes via `transform` + `opacity` only
(never `display:none` / `visibility:hidden`) and paints on keyboard entry via
`:focus-within`, so a keyboard or AT user can Tab straight into it.

Focus rings are rendered as light but are real and contrast-sufficient.

---

## Live Regions (announce once)

The Voice Line and Adaptive Context Surface are labelled live regions that
**announce once** on change — no repeated or chatty announcements, no consecutive
repeats. The Orbit and Dock carry labels, roles, and `aria-current` for the
active Space. Every `coreStore` state exposes a per-state `aria-label` and a text
equivalent (`coreNarration`), so presence conveyed by light/motion is always
available to assistive technology.

---

## Preferences

The homepage honors, and is tested against:

- **Reduced motion** — static Room, static Core frame, 3D forced off (see
  `motion.md`).
- **High contrast** — surfaces meet AA on the *real composited* surface, not the
  token in isolation.
- **Steady lighting** — disables time-of-day undertone drift (mood-only).
- **Color-blind-safe** — state is never encoded by hue alone; label/shape/text
  carry the meaning.

---

## Contrast on Real Surfaces

Contrast is validated **AA on real composited surfaces** — the living-glass
fills and dimmed Room backdrops are measured as rendered, including Reading
Mode's near-solid backing, not against idealized flat colors.

---

## Linux Assistive Technology

The homepage is validated against Linux desktop accessibility (AT-SPI via the
platform webview). Edge/hover affordances keep keyboard/palette equivalents so
the surface is fully operable under Orca and keyboard-only navigation.

> **Note:** Full WCAG conformance cannot be asserted from automated checks
> alone; it requires manual testing with assistive technologies and expert
> review. This contract defines the design intent and the automated/manual
> checks the homepage is held to.
