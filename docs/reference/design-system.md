# Design System — Presence Homepage

> **Last Updated:** 2026-05-27
> **Scope:** Homepage-presence tokens and component stories. See
> `../architecture/presence-homepage-runtime.md` for runtime behavior.

---

## Token Sources

Design tokens are authored as source JSON under `ui/tokens/` and generated into
CSS custom properties at `ui/src/styles/tokens.generated.css`. Consume the
generated CSS variables; never hand-write raw colors (the raw-color CI lint
enforces this — dark and light parity is required for every new token).

The homepage redesign adds these token families (design.md §12):

### Shared-light (published by the Core render tick, ≤1/frame, paused on blur)

- `--core-x`, `--core-y` — Core light position.
- `--core-intensity` — light strength (rises when the Composer gains focus).
- `--core-hue` — presence hue, mapped from `coreStore` state.
- `--core-lean` — directional lean toward the Composer/arrival.

Room, Composer, chips, and Dock consumers react to these variables; the Core is
the single light source in the Room.

### Environmental

- `--room-gradient-*`, `--room-undertone` — base atmosphere and time-of-day
  undertone (mood-only; disabled under a steady-lighting preference).
- `--floor-sheen-alpha` — floor reflection strength.
- `--particle-alpha`, `--particle-count-max` — particle field (≤30,
  transform-only).

### Motion

- `--motion-duration-breath` (~5s), `--motion-duration-recede` (~600ms) — added
  alongside the existing `--motion-duration-fast/base/slow`.
- `--motion-easing-breath` (soft ease-in-out), `--motion-easing-presence` (long
  entrance for bloom/recede) — added alongside `--motion-easing-standard/
  entrance/exit`. Motion CSS lives in `ui/src/styles/motion.css`; see
  `motion.md`.

### Glass (living glass)

- `--glass-fill-rest`, `--glass-fill-active`, `--glass-blur`, `--glass-border`.

### Presence-state hue mapping

Each `coreStore` state maps to a hue/motion pair per design.md §4.1. Every state
keeps a per-state `aria-label` and a reduced-motion static frame.

---

## Component Stories (Histoire)

Homepage components ship with Histoire stories and Vitest/Playwright harness
entries (scaffolded in task 0.4). Stories under `ui/src/shell/spaces/home/`:

- `HomeSpace.stories.tsx` — the composed, never-blank home shell.
- `HomepageScaffolds.stories.tsx` — extended `CorePresence` (3D-gated renderer +
  shared-light) and `TrustIndicator`.

Each homepage component (`Room`, `VoiceLine`, `AdaptiveContextSurface`,
`ContextualChips`, `Composer`, `ContextualOrbit`, `CompanionEmber`,
`TrustIndicator`, `HiddenDock`) has a story that exercises its interaction and
accessibility states, matching the KRIA design-system Definition of Done:
tokens only (no raw colors), focus-visible, reduced-motion honored, and a story
present.

---

## Related Documentation

- `../architecture/presence-homepage-runtime.md` — component map and stores.
- `motion.md` — motion system.
- `accessibility.md` — accessibility contract.
