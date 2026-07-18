# Implementation Plan: KRIA UI Redesign

## Overview

Phased, incremental build. Each task is an actionable coding step referencing requirements. Order minimizes risk: foundation → shell → home → global systems → Spaces → modes/Linux → hardening → migration. Every phase ends with tests + a11y + performance gates green before proceeding. Design source: `KRIA_UI_REDESIGN_MASTERPLAN.md`; requirements: `requirements.md`; architecture/tech: `design.md`.

## Tasks

- [x] 0. Foundation: tokens, kit scaffolding, tooling
  - [x] 0.1 Set up the single design-token source and generate CSS custom properties (color/space/type/radius/elevation/motion/z-index/blur) for dark + light; add a CI lint rule that fails on raw hex/color in components. _Requirements: 14.1, 14.2, 14.3_
  - [x] 0.2 Self-host and bundle the display/text/mono fonts (woff2, subset); wire the Lucide icon sprite. _Requirements: 14.6, 18.4_
  - [x] 0.3 Stand up the component-docs workbench (Histoire) and the testing harness (Vitest + Solid testing library + Playwright); add performance-mark utilities and a dev-gated perf HUD. _Requirements: 16, 17_
  - [x] 0.4 Build the base kit primitives on Kobalte with tokens + full interaction states + focus-visible: Button, IconButton, Input, Select, Textarea, Search, Card, Chip, Badge, StatusDot, Row, SegmentBar, Tabs, Tooltip, Popover, Menu, Dialog/Confirm, EmptyState, Progress. Each ships a Histoire story + a11y test. _Requirements: 14.4, 14.5, 17.1, 17.2_
  - [x] 0.5 Run the P0/P1 prototype validation gates (design.md §11.3) on the target Linux matrix (GNOME+KDE × Wayland+X11 × NVIDIA+AMD+Intel): G1 WebKitGTK baseline + G8 blur are P0 Phase-0 exit criteria; G2 3D-graph viability decides per-device whether 3D is enabled or the 2D graph is default; G3–G7 validate Core/charts/palette/a11y/detach. Record pass/fallback per gate. _Requirements: 16.1, 16.3, 5.5, 17.5, 18.5_
  - [x] 0.6 Establish the Linux rendering baseline: default the Memory graph and Capability constellation to their 2D representation on WebKitGTK, expose 3D only as a runtime-capability-gated enhancement, minimize/blur-test aura-glass, and ship NVIDIA/Wayland graphics guidance + safe-mode boot. _Requirements: 16.3, 18.5, 5.5, 7.5_

- [x] 1. Application shell, routing, state & event architecture
  - [x] 1.1 Implement the internal typed router (`space[/segment][/entityId]`) with deep-link + state restoration (Space/thread/selection/scroll). _Requirements: 1.3, 1.5, 1.4_
  - [x] 1.2 Implement modular stores (shell/core/converse/memory/automation/capability/machine/observatory/settings/approval/notification/voice) replacing the god-store, plus a typed event bus with a burst coalescer. _Requirements: 1.1, 13.4, 16.5_
  - [x] 1.3 Build the Tauri bridge that maps existing commands/events into the typed bus; verify graceful degradation when optional services are absent. _Requirements: 20.4_
  - [x] 1.4 Build AppShell: PresenceBar, Dock (7 Spaces), SpaceRouter (lazy), InspectorHost (single shared), StatusLine; enforce one-modal-at-a-time. _Requirements: 1.1, 1.2, 1.6_
  - [x] 1.5 Implement session/state resume on relaunch. _Requirements: 1.4_

- [x] 2. The KRIA Core (presence & state) + Command Palette
  - [x] 2.1 Implement `coreStore` state machine for all 14+ states, fed by domain events. _Requirements: 3.1_
  - [x] 2.2 Build the Core presence (CSS/SVG-first, state-driven via breath/density/temperature/light); reduced-motion renders static; no spinner. Spike a shader layer only if CSS/SVG proves insufficient. _Requirements: 3.2, 3.5, 16.3_
  - [x] 2.3 Wire Core state to the OS tray/menu-bar glyph as an enhancement with in-app fallback. _Requirements: 3.4, 18.2_
  - [x] 2.4 Build the Command Palette (in-house, Kobalte-backed) with Go/Do/Ask/Change modes, fuzzy search over all entity types, keyboard nav, recent ranking, shortcut discovery; instant open. _Requirements: 2.1, 2.2, 2.3, 2.4_
  - [x] 2.5 Implement summon with global-hotkey enhancement + guaranteed in-app/tray/Mini fallback. _Requirements: 2.5, 18.2_

- [x] 3. Converse (home / AI workspace)
  - [x] 3.1 Build the three-lane layout (ConversationLane focal + WorkLane adaptive + ContextRail on-demand) + sticky Composer; enforce conversation-dominance. _Requirements: 4.1, 4.3_
  - [x] 3.2 Implement MessageStream with virtualization, MessageBubble, inline result cards, and per-message actions (copy/retry/explain/remember/branch/feedback via right-click + selection). _Requirements: 4.8, 16.2_
  - [x] 3.3 Implement WorkBlock types (reasoning/tool/plan-compare/gui-cognition/workflow-run) with status, plain-language summary, details disclosure, evidence, independent Stop; auto-reveal WorkLane on activity. _Requirements: 4.2_
  - [x] 3.4 Build the Composer: grow-then-scroll, attachments, mode chip (Assistant/Lab/tool-lock), voice entry, single Send that becomes prominent Stop; per-thread draft persistence. _Requirements: 4.4, 4.5, 4.9_
  - [x] 3.5 Fold slash-commands into the Command Palette (remove separate slash menu). _Requirements: 4.7_
  - [x] 3.6 Implement cold/warm empty states (Core-forward, ≤3 example intents / continue-suggestions). _Requirements: 4.6_
  - [x] 3.7 Revive PlanVisualization as the plan-compare work block. _Requirements: 20.3_

- [x] 4. Approval Center, Notification Center, Inspector
  - [x] 4.1 Build the unified Approval Center + ApprovalCard (what/why/risk-ramp/effects/evidence; deny-one-action, deliberate approve, explicit high-risk confirm). _Requirements: 11.1, 11.2, 11.3, 11.5_
  - [x] 4.2 Route all HITL sources (tool HITL, interaction decisions, gui-cognition approval, workflow resume) into the approvalStore; coordinate the backend approval-event unification + register `workflow_*` commands so controls are no longer inert. _Requirements: 11.1, 11.6, 3.3_
  - [x] 4.3 Build the Notification Center (batched, tiered) + attention rules (one glow + one running-pulse per surface; interruption ladder; place-preservation). _Requirements: 13.1, 13.2, 13.3, 13.4, 13.5_
  - [x] 4.4 Build the single shared Inspector (slide-in; one at a time; content-typed bodies). _Requirements: 1.6, 5.2, 7.2_

- [x] 5. Voice UX (via the Core)
  - [x] 5.1 Build the compact VoiceSurface (Core + one transcript line; not full-screen by default) driven by voiceStore→coreStore. _Requirements: 12.1_
  - [x] 5.2 Implement voice modes (quick/PTT, conversation, hands-free, wake-word, ambient, meeting, coding, research, planning) with in-surface engine/mode switching. _Requirements: 12.2, 12.3_
  - [x] 5.3 Implement a real wake-word test in onboarding; ensure barge-in + stop-phrase always work. _Requirements: 12.4, 12.5_

- [x] 6. Memory Space (2D) + Knowledge Graph lens (3D)
  - [x] 6.1 Build Memory landing + segments (Explorer/Timeline/Goals&Plans/Reasoning&Causal/Library/Cognition/ColdStart). _Requirements: 5.1_
  - [x] 6.2 Build MemoryCard + Inspector detail (content/confidence/worth/truth/staleness/source/conflicts/lineage/version/AI-explanation); actions verify/correct/reinforce/penalize/forget/hard-delete with undo + deliberate confirm; fix silent-failure states. _Requirements: 5.2, 5.3_
  - [x] 6.3 Implement the Cognition controls + result panel (show what changed, not a toast). _Requirements: 5.6_
  - [x] 6.4 Build the 3D Graph lens (Three.js, budgeted): ngraph layout in a Worker, instanced nodes/edges, LOD, frustum culling, damped orbit, focus/expand/pin/hide, predicted-link materialize, community color, centrality size. _Requirements: 5.4, 16.3_
  - [x] 6.5 Implement the mandatory 2D/keyboard fallback list for the graph + auto-degrade under load/reduced-motion/no-WebGL. _Requirements: 5.5, 16.3, 17.5_
  - [x] 6.6 Deep-link from Converse "why did KRIA answer this" into the relevant memory with Inspector open. _Requirements: 5.7_

- [x] 7. Automations Space (2D + 2D node builder)
  - [x] 7.1 Build segments Run/Build/Schedule/History; surface workflows at top level. _Requirements: 6.1, 6.2_
  - [x] 7.2 Build Run (ask-KRIA-to-pick, WorkflowCard, SuggestionCard, PreparedInputPreview, run progress + EvidenceViewer). _Requirements: 6.3, 6.5_
  - [x] 7.3 Build the 2D node builder canvas + node palette + node Inspector (authoring/draft/test/approve). _Requirements: 6.3, 6.4_
  - [x] 7.4 Merge Scheduled tasks + routines + reminders into this Space. _Requirements: 6.6_
  - [x] 7.5 Wire workflow HITL/cancel/continuation to the Approval Center; fold N8nDiagnosticsPanel value into Build/Health; make the advanced registry reachable. _Requirements: 6.5, 11.6, 20.2, 20.3_

- [x] 8. Capabilities Space (2D + Constellation lens)
  - [x] 8.1 Build segments Tools/Skills/Models/Integrations/Governance/Generate + descriptor Inspector. _Requirements: 7.1, 7.2_
  - [x] 8.2 Implement run→permission-gate→Approval-Center (scope once/session/workspace/always/deny); skill install with trust review; provider switch/test; integration connect. _Requirements: 7.3, 7.4_
  - [x] 8.3 Build the 3D Constellation lens (budgeted, same governance as the graph) + 2D catalog fallback. _Requirements: 7.5, 16.3, 17.5_
  - [x] 8.4 Revive QuarantineQueue into Governance; fold orphaned ICP views' value here; delete dead shells. _Requirements: 20.2, 20.3_

- [x] 9. Machines Space (2D + immersive remote canvas)
  - [x] 9.1 Build fleet matrix (DeviceRow table) + TerminalPane + AlertList + device Inspector; enrollment wizard. _Requirements: 8.1_
  - [x] 9.2 Build the remote-desktop canvas + toolbar + keyboard bar with persistent active indicator + one-action kill; honest capability/permission signaling (Wayland/X11). _Requirements: 8.2, 8.3_
  - [x] 9.3 Integrate mobile pairing/devices; deliberate confirm on destructive machine actions. _Requirements: 8.1, 8.4_

- [x] 10. Observatory Space (2D dashboards)
  - [x] 10.1 Build segments Now/Jobs/Analytics/Forensics/Diagnostics; make it the sole telemetry home. _Requirements: 9.1, 9.2_
  - [x] 10.2 Build SystemPulse + ResourceMeter (uPlot) + JobRow (cancel) + ForensicTimeline + AnalyticsTiles + TestConsole; honest shadow-mode states. _Requirements: 9.1, 9.3, 9.4_
  - [x] 10.3 Revive ExecutiveDashboard into Jobs & Cognition (wire the live store). _Requirements: 20.3_

- [x] 11. Settings Space
  - [x] 11.1 Build the searchable Settings Space with groups (You/Voice/Intelligence/Memory&Privacy/Safety&Approvals/Connections/System/Developer) + NL-change bar + change history. _Requirements: 10.1, 10.2_
  - [x] 11.2 Implement per-setting risk/restart/env-lock badges; guard + quarantine the Developer group. _Requirements: 10.3, 10.4_
  - [x] 11.3 Move feature workspaces out (n8n→Automations, skills/providers/MCP→Capabilities, mobile→Machines); remove fake/frontend-only toggles. _Requirements: 10.5, 10.6_

- [x] 12. Window modes, multi-monitor & Linux-native behavior
  - [x] 12.1 Implement the three window modes (Compact/Standard/Immersive) as shell reconfiguration; degrade-by-curation per the Space×mode matrix; preserve state across transitions; keep approvals + global Stop reachable in Immersive. _Requirements: 15.1, 15.2, 15.3, 15.4_
  - [x] 12.2 Implement window geometry memory, host-decoration respect, and the KRIA-owned in-app mode switch + DE-agnostic Immersive exit. _Requirements: 15.5, 18.3, 18.6_
  - [x] 12.3 Implement the capped detachable-surface set (thread, Approval Center, a lens, remote desktop, Observatory Now) via Tauri windows; Core presence per window; approval mirroring to the active window + tray badge. _Requirements: 15.6, 11.4_
  - [x] 12.4 Implement KRIA Mini + "Now" mini companions. _Requirements: 15.7_
  - [x] 12.5 Validate Linux behavior on GNOME+KDE / Wayland+X11: tray/hotkey/always-on-top fallbacks, own theme/fonts (no GTK/Qt inheritance), fractional-scaling/mixed-DPI crispness, honest capture degradation. _Requirements: 18.1, 18.2, 18.4, 18.5, 8.3_

- [x] 13. Adaptive intelligence (predictable)
  - [x] 13.1 Implement promote/demote in clearly-adaptive zones only (quick actions, empty-state suggestions, palette ranking); never move core nav/primary actions. _Requirements: 19.1, 19.2_
  - [x] 13.2 Make adaptive suggestions explainable, dismissible/pinnable, resettable; retire first-run coach hints after use. _Requirements: 19.3, 19.4_

- [x] 14. Cross-cutting hardening (performance, accessibility, consistency)
  - [x] 14.1 Enforce the motion budget + global reduced-motion kill-switch (covers CSS, Core, and 3D freeze). _Requirements: 16.3, 16.4, 17.4_
  - [x] 14.2 Verify virtualization/lazy-loading across chat/memory/logs/timelines/fleet; keep UI interactive under simulated heavy model load; hit the §5.6 perf targets on target Linux hardware. _Requirements: 16.1, 16.2, 16.5, 16.6_
  - [x] 14.3 Full WCAG 2.2 AA pass: keyboard-complete, focus-visible everywhere, landmarks/heading order/labels/real tables/live regions, risk-not-color-only, high-contrast/font-scale mapping, 3D fallbacks, dialog focus traps + DE-agnostic escape. _Requirements: 17.1, 17.2, 17.3, 17.4, 17.5, 17.6_
  - [x] 14.4 Token-lint gate (zero raw color/undefined tokens); one-component-per-concept audit; AI-content provenance cues. _Requirements: 14.2, 14.4, 20.5_

- [x] 15. Migration, parity & cleanup
  - [x] 15.1 Execute the disposition map (masterplan §6.1): confirm every current capability is preserved in its target Space. _Requirements: 20.1_
  - [x] 15.2 Remove orphaned/dead surfaces and the N8nWorkflowBrowser shim + standalone PermissionModal; ensure no inert controls remain. _Requirements: 20.2, 11.6_
  - [x] 15.3 Parity test old→new per capability; verify graceful degradation for optional services. _Requirements: 20.1, 20.4_

- [x] 16. Future-expansion governance (guardrails in code + docs)
  - [x] 16.1 Document + lint the expansion rules: new features as modes/lenses/capabilities within existing Spaces; Dock capped at ~7; new modules must reuse the kit + Core state language + approval/inspector patterns; every new feature reachable via palette. _Requirements: 21.1, 21.2, 21.3, 21.4_

- [x] 17. Final quality gate
  - [x] 17.1 Verify the Definition of Done for every surface and the overall gate; run the full E2E flow-map suite, a11y, and performance gates on target Linux hardware (GNOME+KDE, Wayland+X11). _Requirements: 16, 17, 18, 20_

## Task Dependency Graph

```
0 Foundation
        │
        ▼
1 Shell/routing/state ──► 2 Core + Palette
        │                        │
        ▼                        ▼
3 Converse ◄──────────── 4 Approval/Notification/Inspector
        │                        │
        ├──► 5 Voice
        ▼
6 Memory   7 Automations   8 Capabilities   9 Machines   10 Observatory   11 Settings
   (all Spaces depend on: 0,1,2,4; each independent of the others)
        │
        ▼
12 Window modes / multi-monitor / Linux  ◄─ depends on 1 + all Spaces
13 Adaptive intelligence                 ◄─ depends on 1,2 + Spaces
14 Cross-cutting hardening (perf/a11y/consistency) ◄─ depends on all Spaces
15 Migration/parity/cleanup              ◄─ depends on all Spaces
16 Future-expansion governance           ◄─ depends on 0,1 (docs/lint)
17 Final quality gate                    ◄─ depends on ALL
```
- **Critical path**: 0 → 1 → 2 → 3 → 4 → (Spaces) → 12 → 14 → 15 → 17.
- Spaces 6–11 can proceed in parallel once 0/1/2/4 exist.
- 3D lens sub-tasks (6.4, 8.3) are gated by the WebKitGTK/Linux performance spike (design.md §1.8/§7); if the spike fails, they proceed on the OGL/2D fallback ladder without blocking their Space.

Wave definitions (parallelizable groups; each wave depends on all prior waves):

```json
{
  "waves": [
    { "wave": 1, "name": "Foundation", "tasks": ["0"] },
    { "wave": 2, "name": "Shell & Core", "tasks": ["1", "2"] },
    { "wave": 3, "name": "Home & global systems", "tasks": ["3", "4", "5"] },
    { "wave": 4, "name": "Spaces (parallel)", "tasks": ["6", "7", "8", "9", "10", "11"] },
    { "wave": 5, "name": "Modes, adaptation, hardening, migration, governance", "tasks": ["12", "13", "14", "15", "16"] },
    { "wave": 6, "name": "Final quality gate", "tasks": ["17"] }
  ]
}
```

## Notes

- **Spike first (highest risk, P0):** run the prototype gates (task 0.5, design.md §11.3) before dependent work. **[CORRECTION per design.md §11.2]** Evidence (Tauri/WebKitGTK docs, Mozilla, Tauri issues) shows WebGL/canvas composite poorly on Linux WebKitGTK; therefore the **2D graph/constellation is the DEFAULT on Linux** and 3D is an opt-in, capability-gated enhancement (task 0.6). Tasks 6.4 and 8.3 (3D lenses) are enhancements gated by G2 and never block their Space's 2D delivery.
- **Two backend contract changes only** (coordinate, don't redesign): unified approval-event shape (task 4.2) and registration of `workflow_*` commands (tasks 4.2, 7.5). Everything else consumes existing commands/events unchanged.
- **Migrate Space-by-Space**: token-lint gate (0.1) is enforced from the start so no new hardcoded color enters; legacy surfaces are ported per the disposition map (15.1) as each Space lands.
- **Gates every phase**: token-lint (zero raw color), a11y (keyboard/focus/labels), and the §5.6 performance targets on target Linux hardware must pass before moving on.
- **Definition of Done** per surface and overall is in design.md §9.
