# Presence Homepage Runtime

> **Last Updated:** 2026-05-27
> **Status:** Shipped behind `home.presence.v2` (default ON; legacy empty state is the rollback path)
> **Spec:** `.kiro/specs/homepage-presence-redesign/`

---

## Executive Summary

The KRIA home surface is a **presence homepage**, not an application dashboard.
At rest it is a light in a dark Room that speaks at most one line. Controls
recede; presence leads. The homepage answers six questions with exactly one
element each — Who is here? (Core), What matters now? (Voice Line + Adaptive
Context Surface), What can I do? (Composer + Chips), What can KRIA do for me?
(Contextual Orbit), Where else can I go? (Hidden Dock / Palette), Can I trust it?
(Trust Indicator). No element answers two questions; none is decorative.

The homepage is a **presentation and read-model layer only**. It consumes
existing Tauri commands, events, and stores. It adds no backend capability, owns
no orchestration, and never sends or executes on the user's behalf. This keeps
the KRIA runtime-authority invariants intact: `TurnGate` remains the sole
top-level planner, and the homepage cannot bypass policy, HITL, or audit.

It ships entirely behind the `home.presence.v2` feature flag. The flag defaults
ON after the Phase-2 exit gate; flipping it OFF restores the legacy Converse
empty state with no rebuild (see `ui/src/featureFlags.ts`).

---

## Boundaries and Invariants

The presence homepage obeys the same non-negotiable boundary the rest of KRIA
does: **the UI proposes; the core decides and executes.** Concretely:

1. **Pure read-model.** The Focus engine (`homeFocusStore`) subscribes to
   existing stores and emits a single `FocusFrame`. It performs no domain
   writes, no tool calls, no navigation, and no send/execute. This is asserted
   by `ui/src/shell/spaces/home/authority.test.ts`, which scans the read-model
   source for forbidden calls.
2. **`coreHint` is advisory.** The Focus engine may hint at a Core mood but
   never writes `coreStore`. The Core state machine remains the single source of
   presence that every surface reads.
3. **Staging, never sending.** Starters, chips, and Orbit points stage a
   reviewable draft in the Composer or route to another surface. They never send
   a message or execute an action (Requirements 4.4, 5.3, 6.4).
4. **Authority order.** Explicit user actions win; rules are deterministic; AI
   outputs are staged. GREEN auto-acts report after the fact; AI never overrides
   navigation. See `ai-rules-user-authority-framework.md` in the spec and
   `ui/src/shell/spaces/home/authority.ts`.
5. **Single instances.** One Adaptive Context Surface, ≤3 chips, one modal host,
   one Inspector host, one overlay manager, one capability-awareness system
   (Orbit). Enforced by the guardrail lint/test (`guardrails.md`,
   `ui/src/shell/spaces/home/guardrails.ts`).
6. **Never blank, never fake.** Every capability tier yields a valid resting
   frame; cold start is truthfully generic with no fabricated personalization.

---

## Component Map

All homepage UI lives under `ui/src/shell/spaces/home/` unless noted. It is
mounted from `ConverseSpace` when `home.presence.v2` is enabled and the surface
is empty.

| Surface | Source | Responsibility |
|---|---|---|
| Home shell | `home/HomeSpace.tsx` | Composes the Room, Core, Focus UI, Composer, and navigation; publishes shared-light. Never blank. |
| Room | `home/Room.tsx` | Pure-presentation environment: base gradient, floor sheen, transform-only particle field (≤30), peripheral darkness. Token-driven; atmosphere, never widgets. |
| Core | `components/CorePresence.tsx` | Identity/presence/emotion/voice/attention. Reuses `coreStore` (16 states, precedence-resolved). Publishes `--core-*` shared-light. Two interactions only: activate and press-hold (push-to-talk). No cursor tracking, no navigation. |
| 3D Core | gated WebGL path in `CorePresence` + `platform/coreRenderMode.ts` | One WebGL surface (shell + one filament + motes + tilted ring + aura + faked static rim). Visually consistent with the 2D path; released on unmount. |
| Voice Line | `home/VoiceLine.tsx` | Single line; live-region announce-once; dwell/crossfade; no consecutive repeat; optional routing-only deep link. |
| Adaptive Context Surface | `home/AdaptiveContextSurface.tsx` | One surface, fixed location; single subject bound to the Voice Line; ≤1 action; recedes/dissolves when empty. |
| Contextual Chips | `home/ContextualChips.tsx` | ≤3 chips from live state; stage-a-draft or route only; omitted when no real action. |
| Composer | `home/Composer.tsx` (adapts `converse/Composer.tsx`) | Unified text/command/voice on the vertical axis; ⌘K hint; focus strengthens light + Core leans. The single home action target. |
| Contextual Orbit | `home/ContextualOrbit.tsx` | Ambient capability awareness from `homeFocusStore.orbit`; routing-only actionable points; static-dot fallback. |
| Companion ember | `home/CompanionEmber.tsx` | Floating ember inheriting Core state; brightens only for needs; click-to-talk; compositor fallback. |
| Trust Indicator | `home/TrustIndicator.tsx` | Muted on-device confirmation; stays lit offline; shows Core→edge reach on desktop action; routes to Settings. |
| Hidden Dock | `shell/HiddenDock.tsx` | Invisible at rest; reveals on edge/Alt/⌘K/pin/AT-focus over a dimmed Room; keyboard/AT reachable while hidden; canonical Space order preserved. |
| Reading Mode | `home/readingMode.ts` | First-send depth-recession (not a page swap); hard-dim Room + near-solid reading backing (AA contrast); reverses on empty. |

### Stores and platform modules

| Module | Source | Responsibility |
|---|---|---|
| Focus engine | `stores/homeFocusStore.ts` | Pure read-model; staged pipeline; emits one `FocusFrame`. |
| Home UI state | `stores/homeStore.ts` | Macro state (rest/engaged/reading/companion), draft, bound Focus subject, render-mode preference. |
| Greeting state | `stores/homeGreetingStore.ts` | Persisted greeting familiarity/session counters feeding the pure greeting derivation. |
| Relationship evolution | `stores/relationshipEvolution.ts` | Content-only scaling (first-launch → long-term); no fake emotion; capped learned facts. |
| Core render gate | `platform/coreRenderMode.ts` | Homepage Core `2d\|3d\|auto` resolver + capability gate + degrade triggers. |
| Lens render gate | `platform/renderMode.ts` | Separate gate for Memory/Capability 3D lenses (not the Core). |
| Motion | `platform/motion.ts`, `styles/motion.css` (budgets guarded by `platform/motionBudget.test.ts`) | Reduced-motion controller and motion budget/shed order. |
| Boot | `platform/boot.ts` | Seeds render-mode gates from live capability detection before surfaces mount. |
| View-mode matrix | `shell/viewModeResponsibilityMatrix.ts` | What shows/hides/persists per Immersive/Standard/Mini/Companion. |
| Feature flag | `featureFlags.ts` | `home.presence.v2` rollout registry (localStorage/env override → default). |

---

## Homepage Intelligence Layer (Focus Engine)

`homeFocusStore` is a deterministic, pure read-model. It fuses signals from the
approval, converse, automation, memory, and notification stores plus the
optional desktop-awareness bridge, and emits a single `FocusFrame` (greeting,
Voice Line, one Adaptive Context Surface subject, ≤3 chips, Orbit points).

It runs a staged pipeline, each stage independently testable:

```text
Signals → Understanding → Confidence → Reasoning → Timing/Interruptibility
        → Personalization → Decision → Presentation → Feedback → Learning
```

Key guarantees (all unit-tested in `stores/homeFocusStore.test.ts`):

- **Determinism + no thrash.** Fixed ranking precedence; conflict resolution by
  precedence → source-trust → recency; incremental, debounced recompute
  (≤1/~250 ms); anti-flicker dwell.
- **Bound subject.** The Voice Line and Adaptive Context Surface bind to the
  same subject; the ACS never shows an empty box.
- **Confidence gating.** Low-confidence subjects get low emphasis only; they
  never take over the surface.
- **Graceful degradation.** Each missing input degrades only its own subjects;
  the engine always yields a valid frame. Tier 0 stays fully usable.
- **Honest personalization.** Greeting familiarity scales full → none with
  no-consecutive-repeat and milestone-only celebration; cold start is generic;
  learned facts are capped; preference learning is bounded and changes no layout.

### Desktop Awareness Subsystem

Desktop awareness is **optional and OFF by default**; the homepage is fully
valuable with zero desktop signals. A bridge (`setAwarenessBridge`) publishes
normalized `AwarenessSignal` events into `homeFocusStore` inputs. It prefers
explicit integrations and portals (calendar, MPRIS, editor plugins, XDG portals)
over scanning, is per-source opt-in with plain-language purpose, processes
locally, keeps signals ephemeral unless opted into memory, and omits unavailable
signals without error. The interruptibility gate keeps a default-silent posture
in blocked contexts (screen-record/share, calls, presentation/fullscreen,
game/DND): only RED approvals surface, calmly and never as audio.

---

## Rendering Split and Degrade Path

**2D is the default and permanent path — never a fallback afterthought.** 3D is
an enhancement gated on device capability. The Core has its own resolver
(`platform/coreRenderMode.ts`) separate from the Memory/Capability lens gate
(`platform/renderMode.ts`).

`resolveCoreRenderMode` reads a stored preference (`2d | 3d | auto`) and a live
capability snapshot, and resolves to a concrete `2d` or `3d`. It auto-degrades
to 2D whenever any documented trigger fires, in a fixed order:

```text
reduced-motion → no-webgl → low-power → failed-gate → frame-drop
```

3D enables only when the preference allows it, the capability gate passes, and
no trigger is active. On degrade the WebGL context is released. Under motion the
budget sheds in order: particles → filament → parallax → breath. At rest there
is no JS animation loop — CSS keyframes only, paused on window blur, idle CPU
≈ 0. See `core-3d-gate-matrix.md` and `performance-matrix.md` in the spec for the
Linux-matrix (GNOME/KDE × Wayland/X11 × NVIDIA/AMD/Intel) gate results; physical
per-device runs are gated/manual on the single-dev laptop and recorded there.

---

## View Modes, Reading Mode, Companion

The canonical window-mode set is **Immersive / Standard / Mini / Companion**
(this supersedes the earlier Compact/Standard/Immersive naming from
`kria-ui-redesign`). The Core is the continuity anchor across transitions;
`shell/viewModeResponsibilityMatrix.ts` fixes exactly what shows, hides, or
persists per mode, and shared state (thread, Core state, draft, Focus subject)
is preserved across every transition.

- **Reading Mode** engages on first send: the homepage recedes into depth (not a
  dock/page swap), the Room hard-dims behind a near-solid AA-contrast reading
  backing, and it reverses when the conversation empties. `ConverseSpace`
  persists across the empty↔reading boundary, so it is a continuous recession,
  never an unmount/remount.
- **Companion** is the detached floating ember; **Mini** is the compact
  companion window. The ember inherits Core state, brightens only for real
  needs, supports click-to-talk, is on by default with opt-out, and falls back
  to in-app/tray when the compositor restricts always-on-top surfaces.

---

## Permission and Trust UX

Permission presentation routes through the existing `approvalStore` and Approval
Center — no new backend:

- **GREEN:** acts, reports via the Voice Line, offers undo.
- **YELLOW:** states intent with a halt window.
- **RED:** the Core steps forward with a single-line allow/deny routed to the
  Approval Center; no modal-on-modal.

The Trust Indicator gives muted on-device confirmation, stays lit offline, shows
the Core→edge reach when KRIA acts on the desktop, and routes to Settings.

---

## Related Documentation

- `overview.md` — platform architecture and runtime-authority invariants.
- `../reference/design-system.md` — homepage tokens and component stories.
- `../reference/motion.md` — motion philosophy, hierarchy, budget, tokens.
- `../reference/accessibility.md` — homepage accessibility contract.
- `../reference/navigation.md` — hybrid navigation + Modal-vs-Page framework.
- `../decisions/adr/004-presence-first-homepage.md` … `008-3d-core-capability-gating.md` — the five decisions behind this redesign.
- `../operations/development.md` — ownership map and testing expectations.
