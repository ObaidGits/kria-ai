# Requirements Document

## Introduction

This specification defines the complete rebuild of KRIA's interface as a **native, local-first, AI-native desktop Operating System experience**. It is the single source of truth for implementation and supersedes the ad-hoc current UI.

Design source of truth: `KRIA_UI_REDESIGN_MASTERPLAN.md` (Parts A–D). Current-state evidence: `KRIA_UI_INVENTORY.md` (Parts I–III). This requirements document translates that design into testable, EARS-formatted requirements. Where the masterplan and the current implementation conflict, the masterplan wins; where the masterplan left visual-stage details open, those are flagged as design-time (see design.md).

**Primary platform:** Ubuntu/Linux (GNOME + KDE, Wayland + X11). Secondary: Windows, macOS. **Non-negotiable constraints:** the UI must remain smooth while local AI (LLM/OCR/voice/vision/agents/automation) saturates CPU/GPU; low CPU/GPU/RAM/battery footprint; calm, premium, AI-first, keyboard-first, accessible.

## Glossary
Locked terminology — see masterplan §48.
Space, Dock, Command Palette / Intent bar, Core, Lane, Rail, Inspector, Work block, Approval Center, Notification Center, Lens (3D), Segment, Mini, Window Mode (Compact/Standard/Immersive), Risk ramp, Ink & Aura, Density tiers (Calm/Focused/Dense).

### Product Goals
1. Replace 7 flat routes + 21-tab settings modal with **7 Spaces + Dock + Command Palette**.
2. Unify the 4 approval UIs → one **Approval Center**; the 4–6 telemetry surfaces → one **Observatory**; scattered modals/toasts/detail panes → **Inspector + Notification Center**.
3. Introduce the **KRIA Core** as the single living presence/state indicator.
4. Deliver **three intentional window modes** + multi-monitor detach, Linux-native.
5. Hybrid **2D-default / selective-3D** (3 lenses only), within a strict performance budget.
6. One **design-token system** (dark+light parity), one component kit — eliminate 73%-hardcoded-color drift.
7. Preserve all real capability of the current app; delete dead/orphaned surfaces; wire inert stubs.

### Non-Goals
- No mobile-first/tablet-first design (a separate mobile companion exists, out of this spec's desktop scope except where noted).
- No fully-3D interface.
- No backend/agent redesign (this spec consumes existing commands/events; contract changes are noted, not designed here).
- No new AI capabilities (UI surfaces existing + near-term capabilities only).

---

## Requirements

### Requirement 1: Application Shell & Spaces
**User Story:** As a KRIA user, I want a single adaptive workspace organized by intent, so that I summon contexts instead of navigating a tree of pages.

#### Acceptance Criteria
1. WHEN the app launches and provisioning is complete THE SYSTEM SHALL present the global shell: Core presence, Command/Intent bar, Dock (7 Spaces), optional Inspector, and one status line.
2. THE SYSTEM SHALL provide exactly these Spaces: Converse, Memory, Automations, Capabilities, Machines, Observatory, Settings.
3. WHEN a user selects a Space via Dock, Command Palette, or keyboard THE SYSTEM SHALL switch to it in ≤1 interaction (nav depth ≤2 to any feature).
4. THE SYSTEM SHALL restore the last active Space, thread, selection, and scroll position on relaunch.
5. THE SYSTEM SHALL make every Space addressable/deep-linkable internally so state is never lost on reload.
6. THE SYSTEM SHALL NOT present any modal that spawns another modal, and SHALL allow at most one modal at a time.

### Requirement 2: Command Palette & Intent Bar
**User Story:** As a keyboard-first user, I want one omni bar to go anywhere, run anything, ask, or change a setting, so that I never hunt through menus.

#### Acceptance Criteria
1. WHEN the user invokes the palette (keyboard, click, or voice) THE SYSTEM SHALL open it instantly (<100 ms perceived) with fuzzy search over Spaces, commands, settings, memories, workflows, capabilities, models, threads, and devices.
2. THE SYSTEM SHALL support four modes: Go (navigate), Do (run), Ask (send to KRIA), Change (natural-language setting change).
3. WHEN results are shown THE SYSTEM SHALL group them and support full keyboard navigation (arrows/enter/esc) and recent-item ranking.
4. THE SYSTEM SHALL expose all keyboard shortcuts discoverably via the palette.
5. IF a global system summon hotkey is unavailable (e.g., under Wayland restrictions) THEN THE SYSTEM SHALL still provide in-app palette + tray + Mini summon without breaking.

### Requirement 3: The KRIA Core (presence & state)
**User Story:** As a user, I want one living indicator of KRIA's state, so that I always know what it is doing at a glance without reading text.

#### Acceptance Criteria
1. THE SYSTEM SHALL render a single Core that expresses at least these states: idle, listening, thinking, planning, speaking, acting, running-automation, watching, remembering, reflecting/dreaming, learning, waiting, blocked/needs-permission, error/recovering.
2. THE SYSTEM SHALL express state via breath, density, temperature (within the accent family), and light — never via a generic spinner.
3. WHEN KRIA is blocked or needs permission THE SYSTEM SHALL still (calm) the Core and direct attention to the Approval Center.
4. WHERE the window is closed THE SYSTEM SHALL reflect Core state in the OS tray/menu-bar glyph as an enhancement (with in-app fallback).
5. THE SYSTEM SHALL be the only element permitted ambient (idle) motion; it SHALL honor reduced-motion by rendering a static state.

### Requirement 4: Converse (home / AI workspace)
**User Story:** As a user, I want to think with KRIA in one workspace where its reasoning, actions, and the memory it used are visible beside the conversation, so that I stay in flow and trust its work.

#### Acceptance Criteria
1. THE SYSTEM SHALL present three lanes: Conversation (focal), Work (adaptive), Context rail (on-demand).
2. WHEN KRIA begins acting THE SYSTEM SHALL reveal the Work lane and stream typed work blocks (reasoning step, tool call, plan-compare, GUI-cognition, workflow run), each with status, plain-language summary, a details disclosure, evidence, and an independent Stop.
3. THE SYSTEM SHALL keep the reply text visually dominant over result cards, work blocks, and rails at all times (conversation-dominance rule).
4. THE SYSTEM SHALL provide a sticky composer that grows to a max then scrolls, never covering the last message, with a single primary Send that becomes a prominent Stop while KRIA works.
5. THE SYSTEM SHALL persist a draft per thread and restore thread state on selection.
6. WHEN there are no messages THE SYSTEM SHALL show a Core-forward empty state with ≤3 example intents (cold) or quiet continue-suggestions (warm), never a blank page.
7. THE SYSTEM SHALL fold slash-commands into the Command Palette (no separate slash menu).
8. WHEN a message is selected or right-clicked THE SYSTEM SHALL offer copy, retry, explain, remember, branch, and feedback actions.
9. THE SYSTEM SHALL provide a "Lab" mode (tool-locked) as a mode of a thread, not a hidden environment.

### Requirement 5: Memory Space (+ 3D Knowledge Graph lens)
**User Story:** As a user, I want to see, trust, and correct what KRIA knows, so that I can rely on its memory.

#### Acceptance Criteria
1. THE SYSTEM SHALL provide a Memory landing (overview, recent, gaps, search) and lenses: Explorer, Timeline, Goals & Plans, Reasoning & Causal, Library, Knowledge Graph, Cognition, Cold Start.
2. WHEN a memory is selected THE SYSTEM SHALL show, in the Inspector, its content, confidence, worth, verification/truth state, staleness, source, conflicts/contradictions, lineage (derived-from/superseded-by), version history, and an AI explanation.
3. THE SYSTEM SHALL allow verify, correct, reinforce/penalize, forget, and hard-delete, with undo for reversible actions and deliberate confirmation for irreversible ones.
4. THE SYSTEM SHALL render the Knowledge Graph as an on-demand 3D lens with node focus/expand/pin/hide, community color, centrality sizing, and predicted-link materialization.
5. THE SYSTEM SHALL provide an always-available 2D list/table fallback for the graph (accessibility + low power).
6. WHEN a cognition job (reflect/dream/consolidate/active-learning/self-improvement/entity-extraction) completes THE SYSTEM SHALL show its result (what changed), not merely a toast.
7. WHEN opened from a Converse answer's "why did KRIA answer this" THE SYSTEM SHALL deep-link to the relevant memory with the Inspector open.

### Requirement 6: Automations Space
**User Story:** As a user, I want one place for everything KRIA does on command or schedule, so that automation is discoverable and controllable.

#### Acceptance Criteria
1. THE SYSTEM SHALL provide segments: Run, Build, Schedule, History.
2. THE SYSTEM SHALL surface workflows at the top level (never buried behind a dashboard sub-tab).
3. WHEN the user describes a workflow in natural language THE SYSTEM SHALL support drafting, reviewing, testing, and approving it.
4. THE SYSTEM SHALL render the workflow builder as a 2D node canvas (not 3D).
5. WHEN a run executes THE SYSTEM SHALL show progress, evidence, and route any human-in-the-loop step to the Approval Center.
6. THE SYSTEM SHALL merge scheduled tasks, routines, and reminders into this Space.

### Requirement 7: Capabilities Space (+ 3D Constellation lens)
**User Story:** As a power user, I want one home for what KRIA can do and how to grant/install/trust/evolve abilities, so that its capabilities are legible.

#### Acceptance Criteria
1. THE SYSTEM SHALL provide segments: Tools, Skills, Models, Integrations, Governance, Generate, and a Constellation lens.
2. WHEN a capability is inspected THE SYSTEM SHALL show its descriptor, effects, trust tier, and schema in the Inspector.
3. WHEN a capability requires approval to run THE SYSTEM SHALL route it through the Approval Center with scope options (once/session/workspace/always/deny).
4. THE SYSTEM SHALL present model/provider switching, skill install with trust review, integration connection, evolution proposals, quarantine, and grants within this Space.
5. THE SYSTEM SHALL render the Constellation as an on-demand 3D lens with a 2D catalog fallback.

### Requirement 8: Machines Space
**User Story:** As an operator, I want fleet/VM/remote/mobile control in one place, so that I manage every machine KRIA touches consistently.

#### Acceptance Criteria
1. THE SYSTEM SHALL provide a fleet matrix (health, latency, docker, tests), enrollment wizard, device Inspector, terminal, alerts, mobile pairing/devices, and remote-desktop.
2. WHEN a remote-desktop session is active THE SYSTEM SHALL show a persistent, unmistakable active indicator and a one-action kill control.
3. THE SYSTEM SHALL communicate capture/input capability and permission state honestly and SHALL NOT present controls that silently do nothing (Linux Wayland/X11 differences).
4. THE SYSTEM SHALL confirm destructive machine actions (delete/reset) with deliberate confirmation.

### Requirement 9: Observatory Space
**User Story:** As a user, I want one calm place to understand KRIA's own state and history, so that telemetry is not duplicated across the app.

#### Acceptance Criteria
1. THE SYSTEM SHALL provide segments: Now (system pulse, resources, running jobs, background cognition), Jobs & Cognition (executive controller), Analytics, Forensics & Recovery, Diagnostics (dev-gated).
2. THE SYSTEM SHALL be the only place, besides the Core and status line, that surfaces system health/telemetry.
3. WHEN a running job exists THE SYSTEM SHALL allow cancel; high-risk resets SHALL require deliberate confirmation.
4. THE SYSTEM SHALL present honest "awaiting data / shadow-mode" states where telemetry is advisory.

### Requirement 10: Settings Space
**User Story:** As a user, I want to find and change any preference quickly, so that configuration never feels like a 21-tab maze.

#### Acceptance Criteria
1. THE SYSTEM SHALL present Settings as a searchable Space grouped into: You, Voice, Intelligence, Memory & Privacy, Safety & Approvals, Connections, System, Developer.
2. THE SYSTEM SHALL support search and natural-language change ("change X to Y") as the primary way to find a setting.
3. THE SYSTEM SHALL show per-setting risk/restart/env-lock badges.
4. THE SYSTEM SHALL visually quarantine the Developer group and guard dangerous toggles.
5. THE SYSTEM SHALL NOT host feature workspaces (n8n connection → Automations; skills/providers/MCP → Capabilities; mobile → Machines).
6. THE SYSTEM SHALL NOT ship fake/frontend-only toggles that have no effect.

### Requirement 11: Unified Approval Center
**User Story:** As a supervisor of an autonomous system, I want all approvals in one consistent place, so that I never miss or misjudge a consequential action.

#### Acceptance Criteria
1. THE SYSTEM SHALL route all human-in-the-loop moments (tool HITL, interaction decisions, GUI-cognition approval, workflow resume) into one Approval Center.
2. WHEN an approval is required THE SYSTEM SHALL present an Approval card stating what will happen, why, risk (risk ramp), effects, and evidence.
3. THE SYSTEM SHALL make deny/keep-paused always one action; approve SHALL require deliberate action; high-risk/irreversible SHALL require an explicit confirm.
4. WHERE multiple monitors are used THE SYSTEM SHALL surface pending approvals on the active window and badge the tray so a needed decision is never hidden.
5. THE SYSTEM SHALL be the ONLY true blocking interrupt in the interruption ladder.
6. THE SYSTEM SHALL wire previously-inert workflow HITL/cancel/continuation controls to real backend actions (no dead controls).

### Requirement 12: Voice UX (via the Core)
**User Story:** As a hands-free user, I want voice expressed through the Core with minimal distraction, so that voice feels native and calm.

#### Acceptance Criteria
1. THE SYSTEM SHALL express voice states (idle/wake-listening/listening/transcribing/thinking/speaking/interrupt/blocked) through the Core plus one transcript line, defaulting to compact (not full-screen).
2. THE SYSTEM SHALL support modes: quick/push-to-talk, conversation, hands-free/continuous, wake-word, ambient, meeting, coding, research, planning.
3. THE SYSTEM SHALL make engine/mode switching reachable from the voice surface itself.
4. WHEN wake-word onboarding runs THE SYSTEM SHALL provide a real, functional wake test.
5. THE SYSTEM SHALL always honor barge-in and a "stop" phrase.

### Requirement 13: Notification & Attention Economy
**User Story:** As a focused worker, I want KRIA to protect my attention, so that I am informed without being interrupted.

#### Acceptance Criteria
1. THE SYSTEM SHALL allow at most one glowing primary action and one subtle running-pulse visible per surface.
2. THE SYSTEM SHALL enforce the interruption ladder: only a blocking approval may seize focus; needs-you is non-blocking; notifications are batched and quiet; ambient never intrudes.
3. THE SYSTEM SHALL collect non-blocking notices in a Notification Center and SHALL batch background completions.
4. THE SYSTEM SHALL preserve the user's place (scroll/selection/draft) after any interruption resolves.
5. THE SYSTEM SHALL NOT use decorative/ambient motion (Core excepted) or manufacture urgency.

### Requirement 14: Design System & Tokens
**User Story:** As a contributor, I want one token system and one component kit, so that the UI never drifts into inconsistency again.

#### Acceptance Criteria
1. THE SYSTEM SHALL source all color/spacing/type/radius/elevation/motion/z-index from a single token system with dark+light parity.
2. THE SYSTEM SHALL contain zero hardcoded colors in components and zero undefined tokens.
3. THE SYSTEM SHALL use exactly one accent hue family, one each semantic (success/warning/danger/info), and a risk ramp reserved for autonomy/consequence only.
4. THE SYSTEM SHALL provide one component per concept (Button, Input, Card, Chip, Badge, StatusDot, Row, Segment, Table, Inspector, Approval card, Work block, Graph node/edge, Progress, Empty state, Notification, Modal/Confirm, Wizard).
5. THE SYSTEM SHALL give every interactive element a visible focus state.
6. THE SYSTEM SHALL bundle its own fonts and icon set (no reliance on system theme variables across Linux DEs).

### Requirement 15: Window Modes & Desktop Behavior
**User Story:** As a desktop user, I want KRIA to behave like native professional software across window modes, so that it feels like an OS, not a website.

#### Acceptance Criteria
1. THE SYSTEM SHALL provide three intentional window modes: Compact (~25–35% screen, curated), Standard (default, respecting OS chrome), Immersive (owns the display).
2. WHEN the window shrinks THE SYSTEM SHALL degrade by curation (drop secondary regions, elevate the primary task), never by uniform compression or web-style reflow.
3. THE SYSTEM SHALL preserve current Space/thread/selection/scroll/draft across mode transitions.
4. WHEN in Immersive THE SYSTEM SHALL still surface approvals and a global Stop.
5. THE SYSTEM SHALL remember window geometry and respect the host window manager's decorations (no fixed fake titlebar).
6. THE SYSTEM SHALL support a capped set of detachable surfaces (thread, Approval Center, a lens, remote desktop, Observatory Now) for multi-monitor use, with the primary window remaining the OS.
7. THE SYSTEM SHALL provide KRIA Mini (Core + intent line) and a "Now" mini as optional compact companions.

### Requirement 16: Performance Budget
**User Story:** As a local-AI user, I want the UI to stay smooth while models run, so that the interface is never the bottleneck.

#### Acceptance Criteria
1. THE SYSTEM SHALL keep idle CPU/GPU near zero (only the Core animates; static under reduced-motion).
2. THE SYSTEM SHALL virtualize long lists (chat, memory, logs, timelines) and lazy-load Space content.
3. THE SYSTEM SHALL mount 3D lenses on-demand, freeze them to a static frame when idle/unfocused, unload them on Space exit, and auto-degrade to 2D under heavy model load or reduced-motion.
4. THE SYSTEM SHALL cap UI transition durations at ~200 ms (≤~400 ms only for deliberate Space/mode changes).
5. THE SYSTEM SHALL avoid layout shift and minimize re-rendering (fine-grained reactivity, no full-tree diffs).
6. THE SYSTEM SHALL remain interactive (input, scroll, stop) while agents/tools run.

### Requirement 17: Accessibility
**User Story:** As a user relying on assistive tech or keyboard, I want KRIA to meet WCAG 2.2 AA, so that it is usable by everyone.

#### Acceptance Criteria
1. THE SYSTEM SHALL be fully keyboard-operable (every action reachable; palette-discoverable) with a visible focus state on all interactive elements.
2. THE SYSTEM SHALL use semantic landmarks, correct heading order, labeled controls, real tables for tabular data, and live regions for KRIA state.
3. THE SYSTEM SHALL never convey risk/consequence by color alone (icon + text).
4. THE SYSTEM SHALL honor reduced-motion, high-contrast, and font-scale (mapped, not merely stored).
5. THE SYSTEM SHALL provide a 2D/keyboard fallback for every 3D lens.
6. THE SYSTEM SHALL trap focus in blocking dialogs and provide a DE-agnostic escape.

### Requirement 18: Linux-Native Behavior
**User Story:** As an Ubuntu user, I want KRIA to feel native across GNOME/KDE and Wayland/X11, so that it never relies on Windows/macOS-only patterns.

#### Acceptance Criteria
1. THE SYSTEM SHALL keep all navigation in-app (no dependence on a global menu bar).
2. THE SYSTEM SHALL treat tray glyph, global hotkey, and always-on-top as enhancements with full in-app fallbacks.
3. THE SYSTEM SHALL respect host window decorations and provide its own in-app window-mode switch.
4. THE SYSTEM SHALL render identically across GNOME/KDE using its own bundled theme/fonts (not inherited GTK/Qt variables).
5. THE SYSTEM SHALL remain crisp and correctly proportioned under fractional scaling and mixed-DPI multi-monitor.
6. THE SYSTEM SHALL provide a clear, DE-agnostic exit from Immersive/fullscreen.

### Requirement 19: Adaptive Intelligence (predictable)
**User Story:** As a returning user, I want KRIA to adapt to my expertise without surprising me, so that it grows with me while remaining predictable.

#### Acceptance Criteria
1. THE SYSTEM SHALL promote frequently/recently used items in clearly-adaptive zones (quick actions, empty-state suggestions, palette ranking) and demote unused ones — never deleting them (always reachable via palette/search).
2. THE SYSTEM SHALL NOT move core navigation or primary-action positions as a result of adaptation.
3. THE SYSTEM SHALL make every adaptive suggestion explainable, dismissible/pinnable, and resettable to defaults.
4. THE SYSTEM SHALL retire first-run coach hints once a feature is used and never repeat them unsolicited.

### Requirement 20: Migration, Parity & Cleanup
**User Story:** As a stakeholder, I want the redesign to preserve real capability and remove dead weight, so that nothing valuable is lost and no cruft is carried forward.

#### Acceptance Criteria
1. THE SYSTEM SHALL preserve every currently-reachable capability (mapped per masterplan §6.1 disposition table).
2. THE SYSTEM SHALL remove orphaned/dead surfaces (CapabilityGraph/Manager/ExecutionLogs/PermissionManager views, N8nDiagnosticsPanel, N8nWorkflowBrowser shim, standalone PermissionModal) or fold their value into a Space.
3. THE SYSTEM SHALL revive live-but-unmounted features (ExecutiveDashboard→Observatory, PlanVisualization→Converse/Memory, QuarantineQueue→Capabilities).
4. THE SYSTEM SHALL retain existing backend command/event contracts unless a change is explicitly noted; UI SHALL degrade gracefully when optional services are unavailable.
5. THE SYSTEM SHALL provide AI-content provenance cues distinguishing KRIA-authored content/actions from user content.

### Requirement 21: Future Expansion Governance
**User Story:** As a maintainer, I want rules for adding features, so that KRIA scales to 10× without cluttering navigation.

#### Acceptance Criteria
1. THE SYSTEM SHALL route new capabilities as modes/lenses/capabilities within existing Spaces, not new top-level Spaces, unless one is retired (Dock capped at ~7).
2. THE SYSTEM SHALL require new modules to inherit the component kit, Core state language, and approval/inspector patterns.
3. THE SYSTEM SHALL keep the home Calm regardless of added capability.
4. THE SYSTEM SHALL make every new feature reachable via the Command Palette on introduction.
