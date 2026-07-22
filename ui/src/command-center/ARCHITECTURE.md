# KRIA Presence UI — Architecture

The presence experience is a **frontend-only, static-demo** HUD (deep-indigo base
with cyan + violet accents). It runs behind the `home.command-center` flag and is
composed of three top-level **surfaces** on one axis, orthogonal to the 7-Space
Dock router. No backend, stores, or tool execution — every value is demo data.

```
                 Context Engine  (context.ts)
                        │  activeContext → presence / orbit / composer / deck ops
                        ▼
  HOME (presence)                 COMMAND DECK (ops)            DEVELOPER (eng)
  ─────────────────                ─────────────────            ───────────────
  Core (hero, breathes)            Mission Header               System Monitor
   └ Orbit (adaptive)              Current Activity             LLM Status
  Presence Line                    Running Operations           Memory Insights
  Composer                         Mission Status
  Action Chips                     Upcoming
  Context Surface (1)
  Hidden Dock (overlay)
```

## Surfaces (`app/surface.ts` + `app/SurfaceHost.tsx`)

`Surface = "home" | "command-deck" | "developer"`, a local reactive signal.
`SurfaceHost` renders the active one. These are **not** Dock Spaces (that would
break the 7-Space `expansion-governance-lint` cap) — they are a separate axis.

- **home** → `command-center/CommandCenter.tsx` — KRIA's resting presence.
- **command-deck** → `command-deck/CommandDeck.tsx` — Mission Control (operations).
- **developer** → `developer/DeveloperObservatory.tsx` — diagnostics.

Reachability: home top bar grid icon → Command Deck; Command Deck bar → Developer;
both → Back to Home.

## Context Engine (`command-center/context.ts`)

The one source of "what is the user doing". A single `activeContext` signal drives
everything adaptive; UI only **reads** (`currentContext`, `currentOrbit`,
`currentOperations`). Detection is pluggable — today a demo signal (`setActiveContext`,
cycled from the home status pill / ⌥⇧C). Real sources (active app, open document,
project, calendar, clipboard, AI) can call `setActiveContext` later **without**
touching any component.

Each `ContextDef` supplies: `orbit` (capability ids), `presence`, `placeholder`
(home) and `objective`, `deckFocus`, `operations` (deck).

## Capabilities (`command-center/capabilities.ts`)

Single source of truth for what KRIA can do. Each `Capability` defines its Orbit
presentation (label, icon, preview `description`) and its Context Surface content
(summary, rows, optional actions). Contexts reference these by id — no duplication.

## Home components (`command-center/`)

| Component | Role |
|-----------|------|
| `CommandCenter.tsx` | Composition + global keys (⌘K dock, Alt reveal, ⌥⇧C context, ESC) |
| `Orbit.tsx` | Adaptive capability ring; reveals on Core hover/focus; opens one surface |
| `PresenceLine.tsx` | One context-aware living sentence |
| `HomeComposer.tsx` | Context-aware placeholder; the primary interaction point |
| `ActionChips.tsx` | Low-weight suggestions |
| `ContextSurface.tsx` | The single Adaptive Context Surface (dissolves when null) |
| `ContextPanel.tsx` | The one contextual surface that emerges from the Core |
| `HiddenDock.tsx` | Navigation overlay, invisible until summoned |
| `homeNav.ts` | Shared state: `activeCapability`, dock open/pinned, focus restore |

**One-Surface Rule:** `activeCapability` is a single value — selecting a capability
replaces any open surface; it never stacks. ESC / backdrop dismiss and restore focus
to the triggering Orbit item.

## Command Deck (`command-deck/`)

`CommandDeck.tsx` composes a context-aware `MissionHeader` over a designed
operational flow laid out with CSS grid areas. Panels are registered with a layout
`region` in `registerDeckPanels.tsx`; the shell places them by region.

## Styling & tokens

All HUD colours, radii, glass, borders, shadows and dots come from `--cc-*` tokens
defined on `:root` in `command-center.css` (loaded globally), reused by the Command
Deck so every surface shares one visual language. This code lives **outside** the
token-lint roots (`design-system, kit, shell, palette, prototypes`) by design, so
the HUD carries its own bespoke theme. All motion is gated by reduced-motion (global
`data-reduced-motion` switch + `prefers-reduced-motion`).

## Extension points

- **New context** → add a `ContextDef` to `CONTEXTS` (+ `CONTEXT_ORDER`).
- **New capability** → add a `Capability` to `CAPABILITIES`; reference its id from any context's `orbit`.
- **New deck panel** → register a `SurfacePanelSpec` (id, title, region, render) in `registerDeckPanels`.
- **Real context detection** → call `setActiveContext` from a resolver; no UI changes needed.
