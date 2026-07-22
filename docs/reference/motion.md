# Motion System

> **Last Updated:** 2026-05-27
> **Scope:** Homepage-presence motion philosophy, hierarchy, budget, and tokens
> (design.md §11). Implementation: `ui/src/styles/motion.css`,
> `ui/src/platform/motion.ts`, `ui/src/platform/motionBudget.ts`.

---

## Philosophy

Motion in KRIA communicates presence, not decoration. The governing rule is
**importance up → motion down; confidence = stillness; never a spinner.** A more
important state is calmer and stiller, not busier. Ambient animation belongs to
the Core (the single light source); everything else reacts to it.

At rest there is **no JavaScript animation loop** — only CSS keyframes — and all
rendering pauses on window blur. Idle CPU cost is ≈ 0.

---

## Hierarchy

1. **Core breath** — the slow, ever-present sign of life (`--motion-duration-breath`).
2. **Shared-light reaction** — Room, Composer, chips, and Dock react to the
   Core's `--core-*` variables (published ≤1/frame).
3. **Presence transitions** — bloom, lean, step-forward, and depth-recede use
   `--motion-easing-presence` / `--motion-duration-recede`.
4. **Functional micro-motion** — focus rings, crossfades, dwell — short and
   honest, never marketing hero animation.

---

## Budget and Shed Order

Motion is bounded by device capability. Under load or on the 3D Core the budget
sheds in a fixed order (design.md §11; CSS transition budgets are guarded by
`ui/src/platform/motionBudget.test.ts`):

```text
particles → filament → parallax → breath
```

The Core render mode caps at 30–45 fps in 3D, pauses on blur, and auto-degrades
to the 2D path on any documented trigger (see
`../architecture/presence-homepage-runtime.md` → Rendering Split). Parallax
responds to window/scroll only, never per-cursor-frame.

---

## Reduced Motion

A reduced-motion preference (`platform/motion.ts` controller +
`platform/accessibilityPreferences.ts`) renders a **static frame** for every
motion state: the Room is flat, the Core shows a still frame, and 3D is forced
off. No information is conveyed by motion alone — every animated state has a text
equivalent (`coreNarration`) and an `aria-label`.

---

## Tokens

Durations reuse `--motion-duration-fast/base/slow` and add
`--motion-duration-breath` (~5s) and `--motion-duration-recede` (~600ms).
Easings reuse `--motion-easing-standard/entrance/exit` and add
`--motion-easing-breath` (soft ease-in-out) and `--motion-easing-presence` (long
entrance for bloom/recede). See `design-system.md` for the full token list.
