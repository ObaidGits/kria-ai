# UI Expansion Governance

Status: required architecture contract for every feature introduced after the KRIA UI redesign.

Validates: Requirements 21.1, 21.2, 21.3, 21.4.

## Expansion laws

1. Extend an existing Space. New work ships as a **mode**, **lens**, or **capability** (a segment is acceptable supporting navigation), never as an eighth top-level destination.
2. The Dock remains capped at seven Spaces: Converse, Memory, Automations, Capabilities, Machines, Observatory, Settings. Replacing a Space requires an explicit product/spec decision and removal of the retired Space in the same change.
3. Reuse shared UI language: components from `ui/src/kit`, states from `coreStore`, blocking decisions from Approval Center, and detail views from the shared Inspector. Do not introduce parallel component, state, approval, modal, or detail paradigms.
4. Keep Converse/home Calm. Expansion may add an intentional mode or a quiet palette/search result; it may not add ambient motion, permanent dashboard chrome, unsolicited urgency, or another focal action.
5. Make the feature reachable through Command Palette on its first change. Palette items navigate or submit intent; they never execute a substrate/tool directly.

## Placement decision

Use this order:

1. **Mode** — changes how an existing task is performed (for example, a tool-locked Converse mode).
2. **Lens** — alternate representation of existing Space data (for example, a Memory graph).
3. **Capability** — something KRIA can resolve and execute through policy.
4. **Segment** — supporting organization inside the owning Space, not a new product root.

Choose the Space that owns the user's intent and artifacts, not the provider implementing it. Providers, MCP, n8n, OpenClaw, and remote targets remain execution substrates.

## Required feature package

Put post-governance feature packages under `ui/src/features/<feature-id>/`. Every package must contain `feature.governance.json`:

```json
{
  "id": "research-lens",
  "kind": "lens",
  "space": "memory",
  "entry": "ResearchLens.tsx",
  "paletteSource": "palette.ts",
  "coreStates": ["thinking", "waiting"],
  "componentKit": "shared",
  "approval": "not-required",
  "approvalReason": "Read-only representation; no consequential action.",
  "inspector": "shared-inspector",
  "home": "calm"
}
```

`approval` and `inspector` may be `not-required` only with a non-empty `approvalReason` or `inspectorReason`. This documents why no consequential action or inspectable entity exists; it is not a bypass. `coreStates` must use canonical `CoreState` values.

Feature package source must:

- import visual primitives from shared kit;
- consume `coreStore`/`CoreState`, never create a parallel activity state machine;
- register a `PaletteSource` through `registerSource`;
- use `approvalStore`/Approval Center when consequence requires consent;
- use `registerInspectorRenderer` or the shared Inspector when entities have detail;
- avoid direct execution from palette code.

## Runtime authority invariant

KRIA remains authoritative orchestrator. OpenClaw, MCP, n8n, model/provider adapters, sidecars, remote machines, and other integrations are execution substrates only.

Every consequential action preserves:

`Intent → Capability → Policy → Substrate → Tool → Verification`

UI and palette code may capture intent, navigate, display state/evidence, request cancellation, or submit approval decisions. It must not add prompt-to-tool shortcuts, expose substrate internals as orchestration, let a substrate select policy, bypass verification/confirmation, create recursive autonomous loops, retry without a deterministic bound, or ignore Stop/cancellation. Work remains bounded, deterministic, verifier-aware, interruptible, and owned by KRIA runtime boundaries.

## Review checklist

- [ ] Existing Space owns feature; `kind` is mode/lens/capability.
- [ ] No eighth Dock item, independent global shell, or nested modal system.
- [ ] Shared kit only; token and component-concept lints pass.
- [ ] Canonical Core states only; no spinner/private status language.
- [ ] Consequential actions route to Approval Center; detail routes to Inspector.
- [ ] Palette source exists on introduction and only navigates/submits intent.
- [ ] Converse/home remains Calm; no added ambient loop or permanent clutter.
- [ ] Intent-to-verification chain, bounded retries, Stop, and cancellation remain intact.

## Enforcement

Run from `ui/`:

```bash
npm run lint:expansion
npm run lint:ui-consistency
```

`lint:expansion` fails for Dock growth, unapproved Space files, missing canonical shell/palette seams, malformed feature descriptors, non-canonical Core states, missing kit/Core/palette registration, direct execution in palette source, or undocumented Approval/Inspector non-use.
