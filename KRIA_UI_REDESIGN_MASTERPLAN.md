# KRIA — UI/UX Redesign Masterplan
### The AI Operating System Design Bible

> Product + UX + Interaction + Visual design planning document. **Not** an implementation/architecture/technology document — contains no framework, code, or backend decisions.
> Primary source of current-state truth: `KRIA_UI_INVENTORY.md` (Parts I–III). This document challenges that reality and defines the target experience.
> Design north star: an **AI Operating System** — calm, premium, spatial, intelligent, productivity-first. Hybrid 2D+3D. Local-first, low-resource.

---

## 0. HOW TO READ THIS DOCUMENT
- **KEEP / MERGE / SPLIT / KILL / NEW** tags mark every surface decision vs the current app.
- **2D / 3D / HYBRID** verdict + rationale on every experience.
- **Why** is stated for every non-obvious decision (the brief demands justification, not preservation).
- Evidence references point back to inventory findings (e.g., "Inv: 73% hardcoded colors").
- Nothing here is a mockup; it is the spec a designer/design-system team executes against.

---

## 1. WHAT KRIA SHOULD BECOME

### 1.1 One-sentence definition
KRIA is a **local-first AI Operating System**: a single calm surface where a person thinks, delegates, and supervises intelligent work — conversation, memory, automation, and machine control — without ever feeling like they opened "an app."

### 1.2 The core reframing (challenge to current reality)
Today KRIA is **7 flat routes + a 21-tab settings mega-modal + a god-store**, with major capability buried (n8n 3 levels deep), duplicated (4 approval UIs, 4–6 telemetry surfaces), and 8 orphaned dead pages (Inv Part I §10). It reads as "a desktop app with a chat tab and a lot of dashboards."

The redesign collapses this into **one adaptive workspace** organized around what the user is *doing*, not around backend subsystems. The mental model shifts from **"navigate to a page"** → **"summon a context."**

### 1.3 What KRIA is NOT
Not a chatbot. Not a dashboard suite. Not a settings tree. Not a fleet console. Not a gaming/cyberpunk HUD. Those are *capabilities inside* KRIA, never the identity.

### 1.4 Emotional target
When KRIA opens the user should feel: **"A calm intelligence is already here, aware and ready."**
- **Calm** (not busy): one focal thing at a time, quiet by default.
- **Alive but not needy**: subtle presence (the AI core), motion only with meaning.
- **In control**: the user always sees what KRIA knows, is doing, and will do — and can stop it instantly.
- **Premium**: restraint, depth, precision typography, generous space — the "expensive software" feeling of Arc/Linear/macOS, not the noise of a trading terminal.

### 1.5 Three design pillars
1. **Presence** — KRIA has a felt center (the Core) that reflects system state (idle/thinking/listening/acting) with cheap, meaningful motion.
2. **Continuity** — memory, reasoning, and action are visible *around* the conversation, not hidden in separate pages. The user never loses the thread.
3. **Supervisable autonomy** — every autonomous action is legible, approvable, reversible, and stoppable from one consistent place (fixes the current 4-approval-UI fragmentation).

---

## 2. VISUAL IDENTITY

### 2.1 Atmosphere (inspired by V1 homepage, then improved)
Take from the V1 concept: spatial depth, subtle holographic feel, AI-first "intelligent room" atmosphere, premium calm. **Improve it by**: removing ambient glow-for-glow's-sake, cutting always-on animation (battery), and anchoring the "futuristic" feeling in *depth + light + typography + one living Core*, not in neon or particle fields. Futurism through **restraint**, not decoration.

### 2.2 Light & depth model
- **Deep, near-black spatial canvas** with soft radial depth (a sense of a dark room with gentle light), not flat black, not busy grids. (Current app already has a faint grid overlay + radial glow — keep the *idea*, reduce the intensity.)
- **Layered glass surfaces**: content sits on translucent panels with subtle blur and a single soft shadow. Depth communicates hierarchy (closer = more important/active).
- **One accent light source**: KRIA's identity color is a **calm teal-green** (the current live `--accent #18a57a` is the right DNA — KEEP the hue family, KILL the competing indigo/blue `#6366f1/#2563eb` that leaked into inline-styled views, Inv Part II §G4). A single accent, used sparingly, for "this is where intelligence/attention is."

### 2.3 Color philosophy
- **Neutral-dominant, accent-rare.** 90% of the UI is graphite/ink neutrals + text; accent teal only marks *AI activity, focus, and primary action*.
- **Semantic set, unified** (fixes the current 4 greens / 5 reds / 4 ambers, Inv Part II §G4): exactly one success, one warning, one danger, one info, each with a soft/solid/text variant, defined once, used everywhere.
- **Risk language** (KRIA-specific, from the safety model GREEN/YELLOW/RED/BLACK): a dedicated, unmistakable risk ramp reserved *only* for approvals/autonomy — never reused for decoration, so "red" always means consequence.
- **Full first-class light theme** — the current light theme is broken on ~8 inline-styled surfaces (Inv Part II §G7). In the redesign, light and dark are equal citizens; nothing is theme-blind.

### 2.4 The KRIA Core (identity motif)
A single, quiet, living **orb/aura** is KRIA's face across the whole OS:
- Idle: slow, minimal breathing.
- Listening: gentle inward pull + live level.
- Thinking/reasoning: soft internal motion (not spinning, not flashing).
- Speaking: outward, calm pulse synced to speech.
- Acting (GUI/automation): a distinct "focused" state.
- Blocked/needs-you: it stills and a risk halo appears.
This replaces the scattered status dots/labels/pills (top bar + status bar + per-panel dots, Inv §E4) with **one legible presence** + supporting text. Cheap to render (see §4).

---

## 3. INTERACTION PHILOSOPHY (AI-first)

### 3.1 Principles
1. **Summon, don't navigate.** A single **Command Palette + intent bar** is the primary way to go anywhere/do anything. (Today there is *no* command palette — Inv Part III §N2. This is the single highest-leverage addition.)
2. **Conversation is the default verb.** Typing/speaking to KRIA is always one keystroke away from any context.
3. **Show the work.** Reasoning, plans, tool calls, and memory used are visible *inline and beside* the conversation, not on separate pages (current chat already streams tool cards + GUI cognition; the redesign elevates this to a first-class "work" lane).
4. **One approval surface.** All human-in-the-loop moments (tool HITL, interaction decisions, GUI cognition approval, n8n resume) funnel into **one consistent Approval experience** (merges 4 current UIs, Inv Part II §H3/§I-6).
5. **Reversibility over confirmation.** Prefer undo + clear "what will happen" over modal nag. Reserve hard confirmations for irreversible/destructive/high-risk only (KRIA already trends this way per dev-context).
6. **Progressive disclosure.** Layman surface by default; a single, consistent "details" affordance reveals depth (KRIA already does this well in the GUI Cognition panel's layman/developer split — generalize it everywhere).
7. **Calm by default, dense on demand.** The resting UI is minimal; density (tables, logs, graphs) appears when the task needs it.
8. **Motion only with meaning** (see §16 + §4 budget).

### 3.2 Friction to remove (from inventory pain points)
- Buried features → surfaced via palette + workspace switcher (n8n was 3 levels deep, Inv §I-5).
- 21-tab settings mega-modal → restructured (see §7.9).
- Duplicate telemetry → one observability space (Inv §H3).
- Dead/orphaned pages → removed (Inv §10; 8 components).
- No deep links / lost state on reload → every context addressable (Inv Part II §C "key gaps").
- Hidden hover-only session actions → persistent/keyboard-reachable (Inv Part III §B2).

---

## 4. THE 2D + 3D HYBRID DOCTRINE (+ hardware budget)

### 4.1 Rule of thumb
**2D is the default. 3D must earn its place by making a *spatial relationship* understandable that a 2D layout cannot.** A fully-3D OS is explicitly rejected (per brief). 3D is a *lens*, never the room.

### 4.2 The single test for 3D
Use 3D **only** when the data is a **graph/space/field whose topology is the insight** — where seeing clusters, distance, density, and connection *is* the value. Everything text-, form-, list-, or time-linear is 2D.

### 4.3 Verdicts (challenged, not blindly accepted from the brief)
**Earns 3D (spatial topology = insight):**
- **Memory Knowledge Graph** — YES. Entities/relationships/communities are inherently spatial; clustering + link density is the point. (Today it's a 2D SVG force layout — the 3D upgrade is genuinely additive.) *Constrained*: capped nodes, 2D fallback, static unless interacted.
- **Capability Constellation** (CPP ecosystem) — YES, light 3D. Providers/capabilities/trust tiers as a navigable field communicates "what KRIA can do" far better than 10 tabs of rows.
- **Reasoning / Plan graph** (structured branching planner's 3 paths + steps) — HYBRID. A shallow 2.5D branching layout shows path comparison + winner; not a full 3D world.

**Stays 2D (3D would harm usability) — challenging the brief's suggestions:**
- **Workflow / n8n builder** — **2D canvas**, not 3D. Node-graph editing is a precision 2D task (Figma/n8n/Blueprints are all 2D for good reason). 3D nodes hurt editing. *Reject 3D here.*
- **Agent thinking / execution flow** — **2D timeline/stream**. Reasoning is sequential and text-heavy; a live 2D "work lane" beats a 3D flow. *Reject 3D.*
- **Task dependency graph** — 2D graph (small, precise). 3D adds nothing.
- **Code architecture / tool ecosystem explorer** — mostly 2D; the *tool ecosystem* may share the Capability Constellation lens, but code = 2D.
- **Voice** — **2.5D at most**: the Core orb has depth/light but is not a 3D scene. Cheap.
- **AI core visualization** — the orb (2.5D shader-lite), not a heavy 3D scene.

**Always 2D (per brief, confirmed):** Settings, forms, tables, editors, chat history/typing, docs, logs, config, search, all productivity/text-heavy views.

### 4.4 3D is a "lens," invoked, not ambient
3D surfaces are **entered deliberately** (open the Memory graph, open the Capability constellation) and **paused when unfocused**. No 3D renders in the background. No always-on 3D on the home surface (only the lightweight Core).

### 4.5 Hardware / resource budget (design constraints, non-negotiable)
KRIA is local-first and shares the machine with the models. Design so the UI is nearly free:
- **Idle cost ≈ zero**: when nothing is happening, nothing animates except the Core's slow breath (low frame-rate, GPU-cheap).
- **3D lenses are on-demand, single-scene, and throttled**: render only while focused/interacting; freeze to a static frame when idle; hard node/element caps with graceful 2D fallback; no particle fields, no post-processing stacks, no continuous physics once settled.
- **Respect reduced-motion globally** (KRIA already enforces this well, Inv Part III §L4 — keep it, and extend it to the JS-driven graph/Core loops which currently escape it).
- **Motion is event-driven, not ambient**: pulses fire on state change, then rest.
- **No cost without meaning**: every effect must encode information (state, progress, attention). Decorative motion is banned.
- Target: KRIA UI should be invisible in CPU/GPU/battery graphs during normal reading/typing.

---

## 5. INFORMATION ARCHITECTURE — THE NEW MODEL

### 5.1 From "pages" to "Spaces"
Replace the 7 flat routes + modal-tabs with a small set of **Spaces** — persistent, addressable contexts the user switches between. Each Space is a purpose, not a subsystem.

**The Spaces (target set):**
1. **Converse** (home) — the AI workspace: chat + live work + memory/context rail. (Absorbs current Home/Chat + Prompt Lab + inline GUI cognition/workflow panels.)
2. **Memory** — the mind of KRIA: explorer, timeline, goals/plans, reasoning, and the 3D Knowledge Graph lens. (Keeps current Memory route, restructured.)
3. **Automations** — everything KRIA can *do on a schedule or on command*: n8n workflows, scheduled tasks, macros, reminders. (SURFACES n8n from its buried Dashboard sub-tab; merges Tasks + Automation.)
4. **Capabilities** — what KRIA *can do now*: tools, skills (OpenClaw), model providers, MCP, integrations, and the Capability Constellation lens. (Merges CPP + OpenClaw marketplace + providers + MCP into one "abilities" home.)
5. **Machines** — fleet/VM/remote control + mobile pairing + remote desktop. (Current VM Management + Mobile/Remote, unified.)
6. **Observatory** — one calm system-status space: health, resources (HRA), executive/running jobs, forensics, analytics, test runner. (MERGES the 4–6 current telemetry surfaces, Inv §H3.)
7. **Settings** — restructured, not a 21-tab dump (see §7.9).

Plus **global, space-independent layers**: Command Palette, the KRIA Core/Voice, the unified Approval Center, Notifications, and the Inspector.

### 5.2 Why 7 Spaces (not 7 pages)
- It matches the count users can hold in mind, and each maps to an *intent* ("talk / remember / automate / empower / control machines / observe / configure").
- It eliminates orphan/duplicate surfaces by giving every current feature exactly one home.
- Spaces are **addressable** (deep-linkable) and **restore on reload** — fixing the current lost-state problem (Inv Part II §C).

### 5.3 The shell (global chrome around every Space)
```
┌──────────────────────────────────────────────────────────────────────┐
│ ● KRIA Core (state)      ⌘ Command / Intent bar        ◇ Approvals  ⚙  │  ← top presence bar (thin)
├────┬─────────────────────────────────────────────────────┬───────────┤
│ D  │                                                       │  Context  │
│ o  │            SPACE CANVAS (the active Space)            │  / Inspector│
│ c  │                                                       │  (optional)│
│ k  │                                                       │           │
├────┴─────────────────────────────────────────────────────┴───────────┤
│  quiet status line: what KRIA is doing right now (1 line, calm)        │
└──────────────────────────────────────────────────────────────────────┘
```
- **Dock (left, thin)**: the 7 Spaces as glyphs; active Space glows. Collapsible to pure icons. Replaces the current flat top-nav bar. Keyboard-switchable.
- **Top presence bar**: the Core (left, identity+state), the **Command/Intent bar** (center — type or speak to go anywhere / do anything), Approvals bell + Settings (right). Replaces the current chip-cluster top bar.
- **Context/Inspector (right, on-demand)**: a single, reusable slide-in panel for details (a memory node, a tool descriptor, a run's evidence, a device). Replaces today's scattered modals/toasts/detail panes with one consistent inspector.
- **Status line (bottom, one line, calm)**: "what KRIA is doing now" — replaces the noisy bottom status bar + duplicate chips.

### 5.4 Navigation layers (complete)
- **Primary**: Dock (Spaces).
- **Secondary**: within-Space tabs/segments (kept minimal; e.g., Memory's lenses).
- **Global/omni**: Command Palette (⌘K style) — jump to any Space, run any command, ask KRIA, switch model, toggle voice, open a memory, run a workflow. This is the backbone.
- **Quick actions**: a small contextual action cluster near the Core / intent bar (new chat, voice, screenshot-to-KRIA, new automation).
- **Floating AI actions**: select text/'object' anywhere → a compact "Ask/Do with KRIA" affordance (AI-first, removes navigation).
- **Context menus / right-click**: introduced where today there are none (Inv §N2: 0 context menus) — on messages, memory nodes, files, devices, workflow nodes.
- **Keyboard**: full model (see §15.x per journey); every Space + palette + approvals reachable without mouse.
- **Workspace/view switching**: Command Palette + Dock + keyboard cycle; Spaces preserve their internal state.
- **Notifications**: a single quiet notification center (approvals, job completions, alerts) — replaces ad-hoc toasts scattered today.
---

## 6. FULL APPLICATION MAP (target)

```
KRIA (AI Operating System)
│
├── GLOBAL LAYERS (present in every Space)
│   ├── KRIA Core / Voice presence (top-left) — one living state indicator + voice entry
│   ├── Command / Intent Bar (top-center) — omni nav + ask + do  [PRIMARY navigation]
│   ├── Approval Center (top-right bell) — ALL HITL/decisions/automation approvals (unified)
│   ├── Notification Center — job done / alerts / needs-you (calm, batched)
│   ├── Context Inspector (right slide-in) — single reusable detail surface
│   ├── Quick Actions cluster — new chat, voice, capture, new automation
│   └── Status line (bottom) — "what KRIA is doing now"
│
├── SPACE 1 · CONVERSE  (home)                         [2D + inline work lane]
│   ├── Conversation lane (chat/notebook hybrid)
│   ├── Work lane (live reasoning · tool calls · plans · GUI-cognition · workflow runs)
│   ├── Context rail (memory used · sources · active tools · model)  [toggle]
│   └── Modes: Assistant · Prompt Lab (as a mode, not a hidden env)
│
├── SPACE 2 · MEMORY                                   [2D shell + 3D Graph lens]
│   ├── Explorer (facts/records + detail)         2D
│   ├── Timeline                                  2D
│   ├── Goals & Plans                             2D (2.5D plan-compare optional)
│   ├── Reasoning & Causal                        2D / 2.5D
│   ├── Library (ingested docs)                   2D
│   ├── Knowledge Graph                           3D lens (on-demand)
│   ├── Cognition (dream/reflect/consolidate…)    2D control + result surface (fix: results now shown)
│   └── Cold Start onboarding                     2D wizard
│
├── SPACE 3 · AUTOMATIONS                              [2D + 2D node canvas]
│   ├── Command workflows (n8n) — browse/run/monitor      2D cards + runs timeline
│   ├── Workflow builder/authoring                        2D node canvas (NOT 3D)
│   ├── Scheduled tasks & routines                        2D
│   ├── Reminders                                         2D
│   └── Run history & evidence                            2D timeline + inspector
│
├── SPACE 4 · CAPABILITIES                             [2D + 3D Constellation lens]
│   ├── Abilities overview (what KRIA can do now)         2D
│   ├── Tools / native capabilities (CPP browser)         2D + inspector
│   ├── Skills marketplace (OpenClaw install/trust)       2D
│   ├── Model providers & runtime                         2D
│   ├── Integrations (Google/Colab/Telegram/MCP)          2D
│   ├── Capability Constellation                          3D lens (on-demand)
│   ├── Evolution / proposals / quarantine / grants       2D
│   └── Generate (synthesis) + Discovery + Execution monitor 2D
│
├── SPACE 5 · MACHINES                                 [2D + immersive remote canvas]
│   ├── Fleet matrix (targets, health, terminal)          2D table + live
│   ├── Enrollment                                        2D form/wizard
│   ├── Mobile pairing & devices                          2D
│   └── Remote desktop (view/control)                     immersive canvas (video)
│
├── SPACE 6 · OBSERVATORY                              [2D dashboards]
│   ├── Now (live system pulse: health, resources, running jobs)   2D HUD
│   ├── Jobs & Cognition (executive controller, background work)   2D
│   ├── Analytics (usage/telemetry)                                2D
│   ├── Forensics & recovery (Ironclad)                            2D
│   └── Diagnostics / Test runner (developer)                      2D (dev-gated)
│
├── SPACE 7 · SETTINGS                                 [2D, restructured]
│   └── (see §7.9 — grouped, searchable, progressive, NOT 21 flat tabs)
│
└── MODE: MOBILE (companion)                           [separate responsive surface]
    ├── Converse (mobile chat)
    ├── Remote desktop
    └── Pair / settings
```

### 6.1 Disposition of every current surface (KEEP / MERGE / SPLIT / KILL / NEW)
| Current (inventory) | Decision | Target home |
|---|---|---|
| Home/Chat | KEEP+ELEVATE | Space 1 Converse (conversation lane) |
| Prompt Lab (hidden env) | MERGE | Space 1 as a "Lab/tool-lock" mode (not a hidden sidebar env) |
| Dashboard (Ironclad strip) | SPLIT+MERGE | Space 6 Observatory (Now/Forensics) |
| — Analytics toggle | MERGE | Space 6 Analytics |
| — n8n sub-tab | MOVE+PROMOTE | Space 3 Automations (top-level) |
| — Tests toggle | MOVE | Space 6 Diagnostics (dev-gated) |
| VM Management + DeviceMatrix | KEEP | Space 5 Machines |
| Tasks | MERGE | Space 3 Automations |
| Capabilities (CPP 10 tabs) | KEEP+RESTRUCTURE | Space 4 Capabilities |
| Memory (13 tabs) | KEEP+RESTRUCTURE | Space 2 Memory |
| Settings (21 tabs) | RESTRUCTURE | Space 7 Settings |
| MCP / Telegram / Google / Colab (settings tabs) | MERGE | Space 4 Integrations |
| OpenClaw settings + SubstrateStatus + SkillMarketplace | MERGE | Space 4 Skills |
| Mobile & Remote panel | MOVE | Space 5 Machines |
| HitlModal | MERGE | Global Approval Center |
| DecisionActionCenter | MERGE | Global Approval Center |
| GUI Cognition HITL | MERGE | Global Approval Center |
| n8n HITL resume | MERGE | Global Approval Center |
| VoiceOverlay/Onboarding | KEEP+ELEVATE | Global Core/Voice |
| Toasts (scattered) | MERGE | Global Notification Center |
| Top-bar chips + bottom status bar | MERGE | Core + status line |
| Descriptor/Result/detail modals & panes | MERGE | Global Context Inspector |
| ExecutiveDashboard (orphan) | REVIVE→MERGE | Space 6 Jobs & Cognition (wire the live store that exists) |
| PlanVisualization (orphan) | REVIVE→MERGE | Space 1 work lane / Space 2 plans (store is live) |
| QuarantineQueue (orphan) | REVIVE→MERGE | Space 4 (safety) |
| CapabilityGraph/Manager/ExecutionLogs/PermissionManager views (orphan) | KILL or FOLD | fold useful ideas into Space 4; delete dead shells |
| N8nDiagnosticsPanel (orphan) | REVIVE→MERGE | Space 3 diagnostics |
| N8nWorkflowBrowser shim / standalone PermissionModal | KILL | remove |
| workflowSession HITL/cancel/continuation (inert stubs) | FIX | wire into Approval Center + work lane |
| NEW: Command Palette / Intent bar | NEW | global |
| NEW: unified Approval Center | NEW | global |
| NEW: Context Inspector | NEW | global |
| NEW: Observatory "Now" | NEW | Space 6 |
| NEW: Capability Constellation | NEW | Space 4 lens |

---

## 7. SPACE-BY-SPACE DESIGN

> Each Space uses the required template: Purpose · Why · Users · Primary/secondary workflow · Entry · Info hierarchy · Layout · Sections/widgets · Interactions · Empty/Loading/Error · AI/Voice/Memory integration · Relationships · Extensibility · 2D/3D verdict + why.

### 7.1 SPACE 1 — CONVERSE (home / AI workspace)
- **Purpose**: the place you think *with* KRIA. Conversation + the live "work" it produces + the context it used.
- **Why**: chat is the default verb (§3). Current chat already streams tool calls, GUI cognition, workflow progress, images — but they crowd the message column. Elevate them into structure.
- **Users**: everyone, every session.
- **Primary workflow**: user asks → KRIA responds, streaming; tool calls/plans/reasoning appear in the **work lane**; memory used appears in the **context rail**; approvals surface globally.
- **Secondary workflow**: Lab mode (tool-locked testing — replaces hidden Prompt Lab env); attach files/images/audio; export transcript.
- **Entry**: default Space; Command Palette "new chat"; Ctrl/⌘+N; voice.
- **Information hierarchy**: (1) the conversation, (2) what KRIA is doing now, (3) why (context/memory), (4) history.
- **Layout**: center **conversation lane**; optional right **work lane** (live reasoning steps, tool cards, plan compare, GUI-cognition, workflow runs — collapsible, auto-opens when work starts); optional far-right **context rail** (memory grounding, active model, active tools). Sidebar = session/thread list (persistent, not hover-hidden — fix Inv §B2).
- **Sections/widgets**: message bubbles (user/assistant/system/tool), inline tool-result cards (news/web/image/google — KEEP, they're good), plan-compare card, GUI-cognition panel (layman + details — KEEP the two-layer pattern), memory-feedback ("why did KRIA answer this"), export.
- **Interactions**: type/voice, slash-commands → **fold into the Command Palette** (no separate slash menu), attach, stop (always one action, prominent), per-message actions via right-click/hover (copy, retry, feedback, "explain", "remember this").
- **Empty**: a calm welcome centered on the Core + 3–4 example intents (not a blank card).
- **Loading**: the Core enters "thinking"; work lane shows live steps (replaces bare dots).
- **Error**: inline, with recovery options + retry (KEEP current recovery-options pattern) and a "what went wrong" plain-language line.
- **AI/Voice/Memory**: this is the fusion point — reasoning live, memory beside, voice as an alternate input to the same lane.
- **Relationships**: opens Inspector for any tool result/memory; deep-links into Memory/Automations/Capabilities when a result references them.
- **Extensibility**: work-lane is a stream of typed "work blocks" — new agent capabilities appear as new block types without new pages.
- **2D verdict**: **2D** (text + linear work). 3D would harm reading/typing. The only 3D nearby is the Core (2.5D).

### 7.2 SPACE 2 — MEMORY
- **Purpose**: see, trust, and shape what KRIA knows.
- **Why**: memory is a pillar (§1.5). Today it's 13 sibling tabs with no landing (Inv §H1). Give it a home + lenses.
- **Users**: power users, researchers, anyone auditing "why KRIA said X."
- **Primary workflow**: search/browse a memory → inspect (confidence/worth/source/conflicts/version) → verify/correct/forget.
- **Secondary**: explore the graph; review reasoning/causal history; manage goals/plans; ingest a document (Library); run cognition (dream/reflect/consolidate) **and see results** (fix: current results are discarded to a toast, Inv Part I §5).
- **Entry**: Dock; Command Palette ("memory: …"); from a chat answer's "why did KRIA answer this" → jumps here with the node open.
- **Info hierarchy**: (1) what do you want to recall/inspect, (2) the record + its trust, (3) its relationships, (4) its history.
- **Layout**: a **Memory landing** (overview: counts, health, recent, gaps, quick search) → lenses as segments: Explorer · Timeline · Goals/Plans · Reasoning/Causal · Library · **Graph (3D)**. Detail always in the Context Inspector.
- **Widgets**: memory card (content, type, confidence bar, worth, staleness, source), relationship chips, conflict/contradiction flag, version history, AI explanation ("why this is here"), goal tree, plan-compare.
- **Interactions**: search/filter/sort; edit/verify/forget/hard-delete (with the risk language, not nag); reinforce/penalize; create relationship; grant/deny cold-start sources.
- **Empty**: "KRIA is still learning about you" + cold-start entry.
- **Loading/Error**: skeletoned lists; explicit error (fix: current explain fails silently, Inv Part I §5).
- **AI/Voice/Memory**: ask "what do you know about X?" from anywhere → opens the relevant memory lens.
- **Knowledge Graph — 3D lens (verdict: 3D, justified)**: nodes = entities, edges = relationships, color = community, size = centrality; navigate/orbit/focus; click node → Inspector; predicted links shown distinctly; materialize a link. **Why 3D**: relationship topology + clustering is the insight; a 3D field reveals structure a 2D list/force-graph flattens. **Constraints**: node cap, on-demand render, freeze when idle, 2D fallback list always available (accessibility + low-power).

### 7.3 SPACE 3 — AUTOMATIONS
- **Purpose**: everything KRIA does on command or schedule — the "hands" of the OS.
- **Why**: n8n is a flagship capability buried 3 levels deep today (Inv §I-5); Tasks/Automation/Reminders are split. Unify into one "get things done automatically" home.
- **Users**: automation builders, power users, daily "remind/plan me" users.
- **Primary workflow**: find or describe a workflow → prepare input → run → watch it → see evidence. (KEEP the current strong routing/suggestion/prepare-input/evidence flow — it's well-built, just unreachable.)
- **Secondary**: author/edit a workflow (2D node canvas); schedule tasks/routines; set reminders; review run history.
- **Entry**: Dock; Command Palette ("run …", "automate …"); from chat ("KRIA, every morning…").
- **Info hierarchy**: (1) what can run, (2) run it / its status, (3) evidence/result, (4) build/edit.
- **Layout**: **Automations landing** (ready-to-run cards + "ask KRIA to pick"), Runs timeline, Builder (2D canvas), Schedules, Reminders. Connection/setup lives here too (from N8nSettings), not in global Settings.
- **Widgets**: workflow card (risk/trigger/last-run), suggestion card, prepared-input preview, run progress + evidence viewer, schedule row, reminder row.
- **Interactions**: run/run-now, prepare input, approve draft, archive, schedule, snooze; HITL resume → Approval Center.
- **Empty/Loading/Error**: "No automations yet — describe one" (AI-first authoring); friendly connection-repair (KEEP current repair steps).
- **AI/Voice/Memory**: describe an automation in natural language → KRIA drafts it; memory informs suggestions.
- **2D verdict**: **2D**, incl. the builder (node editing is a precision 2D task — explicitly reject 3D here, §4.3). Runs = 2D timeline.

### 7.4 SPACE 4 — CAPABILITIES
- **Purpose**: what KRIA *can do* — and how to grant, install, trust, and evolve those abilities.
- **Why**: today this is fragmented across Capabilities(CPP, 10 tabs) + OpenClaw marketplace(in Settings) + Providers(in Settings) + MCP/integrations(in Settings). One coherent "abilities" home.
- **Users**: power users, developers, anyone extending KRIA.
- **Primary workflow**: discover a capability → inspect → run/approve (permission gate) OR install a skill (trust review).
- **Secondary**: switch model/provider; connect integrations; review evolution proposals, quarantine, grants; generate/synthesize a capability.
- **Entry**: Dock; Command Palette ("what can you do about …", "install …", "switch model").
- **Info hierarchy**: (1) abilities overview, (2) a specific ability + its trust/effects, (3) grant/approve, (4) evolve/govern.
- **Layout**: **Capabilities landing** (overview + search), segments: Tools · Skills · Models · Integrations · Governance (evolution/quarantine/grants) · **Constellation (3D)**. Detail → Inspector (descriptor, effects, trust, schema).
- **Widgets**: capability row + descriptor inspector (KEEP — good), skill card + trust badge + permission review, provider card + test/apply, integration status, proposal card, grant row.
- **Interactions**: run→permission gate→approve scope (KEEP the once/session/workspace/always model, route through Approval Center); install→capability review; set autonomy level.
- **Empty/Loading/Error**: honest states (KEEP CPP's honest "no data yet").
- **Capability Constellation — 3D lens (verdict: light 3D, justified)**: a navigable field of everything KRIA can do, clustered by domain/provider, sized by usage/health, dimmed if quarantined. **Why 3D**: communicates the *breadth and relationships* of an ability ecosystem at a glance far better than 10 tabs; supports the "AI OS with capabilities" identity. **Constraints**: on-demand, capped, 2D catalog fallback.
- **2D verdict**: shell + all governance/config = 2D; Constellation = 3D lens only.

### 7.5 SPACE 5 — MACHINES
- **Purpose**: KRIA's reach beyond this device — fleet, VMs, remote desktop, phone companion.
- **Why**: current VM Management + Mobile/Remote are separate; both are "other machines KRIA touches."
- **Users**: operators, developers, remote users.
- **Primary workflow**: see machine health → act (terminal, docker eval, remote view) / enroll a new one.
- **Secondary**: pair a phone; manage devices; kill remote session.
- **Entry**: Dock; Command Palette ("connect …", "remote …").
- **Layout**: **Machines landing** (fleet matrix + this-device + phones), enrollment wizard, per-machine Inspector (state/health/terminal/docker/tests), remote-desktop immersive canvas.
- **Widgets**: device row (health bar, latency, docker, test), terminal pane, alerts, pairing card, remote toolbar.
- **Interactions**: enroll (wizard), edit, delete (confirm), run docker evals, focus terminal, start/kill remote; reset controls (Ironclad) live in Observatory, not here.
- **Empty/Loading/Error**: "No machines connected" + enroll; live stream states (online/connecting/offline) as calm pills.
- **AI/Voice/Memory**: "KRIA, run the smoke test on the staging VM."
- **2D verdict**: **2D** for matrix/terminal/forms; **immersive canvas** only for the live remote-desktop video (already is).

### 7.6 SPACE 6 — OBSERVATORY
- **Purpose**: one calm place to understand KRIA's own state and history.
- **Why**: today the same telemetry appears 4–6 ways (top bar, status bar, Ironclad strip, Analytics, ExecutiveDashboard-orphan, ResourceDashboard) — Inv §H3. Merge into one.
- **Users**: everyone (Now) → developers (Diagnostics/Forensics).
- **Primary workflow**: glance at "is KRIA healthy / what is it doing" → drill into a job/metric/incident.
- **Layout**: **Now** (live pulse: health, resources/HRA, running jobs, background cognition), **Jobs & Cognition** (executive controller — revive the orphaned ExecutiveDashboard, its store is live), **Analytics**, **Forensics & Recovery** (Ironclad + reset controls), **Diagnostics/Test Runner** (dev-gated).
- **Widgets**: system pulse cards, resource bars, job list (priority/state/cancel), forensic timeline, analytics tiles, test console.
- **Interactions**: cancel a job, soft/hard reset (high-risk, KEEP typed confirm), export diagnostics.
- **Empty/Loading/Error**: honest shadow-mode labels (KEEP HRA's honesty).
- **2D verdict**: **2D dashboards/HUD** throughout. No 3D (metrics ≠ spatial topology).

### 7.7 SPACE 7 — SETTINGS
- See §7.9.

### 7.8 MOBILE companion
- **Purpose**: talk to KRIA + control the desktop from a phone.
- **Layout**: bottom-tab (Converse / Remote / Settings) — KEEP, it's clean and responsive.
- **Fixes**: show QR for pairing (backend already produces it, Inv Part II mobile), add in-flight cancel, surface tool activity, add chat reconnect.
- **2D verdict**: 2D + immersive remote canvas.

### 7.9 SETTINGS — restructure (challenge the 21-tab modal)
- **Kill the mega-modal.** Settings becomes a **searchable Space** with a small number of groups, progressive disclosure, and inline search ("type what you want to change"). The current NL config-prompt is a great seed — make **search + natural-language config the primary way to find a setting**.
- **Groups (not 21 flat tabs)**: **You** (appearance, language, assistant persona), **Voice**, **Intelligence** (models/providers/routing — cross-links Capabilities), **Memory & Privacy**, **Safety & Approvals**, **Connections** (Google/Colab/Telegram/MCP — cross-links Capabilities), **System** (hardware/GPU, advanced), **Developer** (dev mode, Ironclad, readiness-bypass, diagnostics — one clearly-marked dangerous area).
- **Move out**: n8n connection → Automations; skills/providers/MCP → Capabilities; mobile → Machines; briefing → Automations. Settings holds *preferences*, not *feature workspaces*.
- **Field intelligence**: keep KRIA's schema-driven risk/restart/env-lock badges (a genuinely good current feature, Inv Part I §4.1) — but present them calmly.
- **Kill frontend-only mock tabs** (Labs mock catalog, Assistant frontend-only prefs) or make them real; don't ship fake toggles.

---

## 8. MODAL DOCTRINE (drawer / popover / inline / wizard / remove)

Principle: **modals interrupt; prefer the Inspector (slide-in) or inline.** Reserve true modals for a *decision that must block* (destructive confirm, approval).

| Current modal/dialog | Redesign form | Why |
|---|---|---|
| SettingsModal | **Space** (not modal) | too big to interrupt; needs deep-linking |
| Descriptor / Result / detail panes | **Context Inspector** (slide-in) | non-blocking, consistent, reusable |
| HitlModal + DecisionActionCenter + GUI approval + n8n resume | **Approval Center** (one surface; blocking only for high-risk) | one mental model; §3.1-4 |
| Add/Edit Target | **Inline wizard** in Machines | enrollment is multi-step, belongs in context |
| Tool-choice (low confidence) | **Inline** in work lane | don't interrupt; offer choice in place |
| Setup Wizard | **Full-screen wizard** | KEEP (first-run is legitimately blocking) |
| Voice onboarding | **Inline coach** in the Core/voice surface | KEEP short; fix fake wake-test |
| Memory onboarding (cold start) | **Wizard within Memory** | KEEP |
| Shortcuts overlay | **Command Palette help** | fold into palette |
| Toasts | **Notification Center** + transient inline confirms | de-scatter |
| Image full-screen preview | **Lightbox** (kept) | appropriate |
| Standalone PermissionModal (dead) | **Remove** | duplicate |

---

## 9. VOICE UX

Voice is a first-class input to the *same* workspace, expressed through the **KRIA Core**. Minimal, calm, never a separate app.

### 9.1 Shared voice state language (the Core)
Idle → Wake-listening → Listening (live level) → Transcribing → Thinking → Speaking → (Interrupt) → Blocked/needs-you. One consistent visual across every mode; supporting text is secondary. Barge-in always available; "KRIA stop" always works.

### 9.2 Modes (each: UI · feedback · when)
- **Quick voice** (push-to-talk): hold to talk, release to send. UI: Core pulls inward + level meter; minimal. Best for one-shot commands.
- **Conversation mode**: back-and-forth, hands mostly free; Core alternates listen/speak; live transcript optional.
- **Hands-free / Continuous assistant**: always listening for turns; clear "I'm listening" affordance + easy mute.
- **Wake-word mode**: "Hey KRIA" starts a turn even when unfocused; a subtle wake flash; privacy-clear (only wake phrase monitored) — and **actually testable** in onboarding (fix current fake test).
- **Ambient mode**: KRIA present but silent; only the Core breathes; speak to engage. Lowest-distraction, lowest-cost.
- **Meeting mode**: KRIA listens/notes without speaking; visible "listening, not responding" state; summarize on request.
- **Coding mode**: voice dictation + command ("run tests", "explain this") with a compact, non-covering indicator so the editor/chat stays visible.
- **Research mode**: voice queries that spawn work-lane research with sources; Core shows "gathering."
- **Planning mode**: voice → plan appears in work lane; approve steps by voice or click.

### 9.3 Voice UI rules
- Never full-screen unless the user chooses immersive; default is the **compact Core + one transcript line** so the workspace stays usable (challenge to today's full-screen overlay).
- Interruptions, playback health, latency (TTFA) surfaced subtly, only when relevant.
- Engine/mode switching reachable **from the voice surface itself** (fix: today it's only in Settings, Inv Part II §I-11).
- Motion: state-change pulses only; no continuous heavy animation (battery).

---

## 10. MEMORY UX (complete)

- **Memory landing**: overview (counts, health, gaps, recent, search) — the missing "home" for today's 13 tabs.
- **Explorer**: search → result list → **memory card**. Card shows content, type, **confidence** (bar), **worth**, **truth/verification** state, **staleness**, **source event**, access count. Actions: verify, correct (inline), reinforce/penalize, forget, hard-delete (risk language).
- **Memory detail (Inspector)**: full record + **AI explanation** ("why KRIA believes this / where it came from"), **conflicts/contradictions** (flagged, with the competing memory), **superseded-by / derived-from** lineage, **version history**.
- **Timeline**: chronological memory formation; scrub; filter by type.
- **Goals & Plans**: goal tree (status, priority, confidence); plans per task with worth/success; plan-compare (revive PlanVisualization here).
- **Reasoning & Causal**: reasoning traces (chains/hypotheses/counterexamples), causal effects/causes/chains; replay a session.
- **Library**: ingested docs (chunks, version), ingest by path/drop, delete.
- **Knowledge Graph (3D lens)**: §7.2 — entities/relationships/communities/centrality/predicted links; node interactions (focus/pin/hide/expand), materialize predictions; **2D fallback always present**.
- **Cognition**: trigger reflect/dream/consolidate/active-learning/self-improvement/entity-extraction **and show the result** (what changed, how many, what was learned) — fix the current toast-and-discard.
- **Search/filter/edit/verify/deletion/confidence/worth/truth/conflicts/version history**: all first-class per above.
- **Emotional goal**: the user *trusts* KRIA's memory because they can always see and correct it.

---

## 11. CHAT → AI WORKSPACE (rethink)

Chat is **not** just a message list; it is **Mission Control for a single line of thought**.
- **Three lanes** (§7.1): Conversation · Work · Context — each independently collapsible; Work auto-opens when KRIA acts.
- **Live reasoning**: the work lane shows KRIA's steps as they happen (plan → tool → verify), plain-language by default, "details" on demand (generalize the GUI-cognition two-layer pattern).
- **Live execution**: tool calls, workflow runs, GUI-cognition, image gen appear as typed **work blocks** with status, evidence, and a stop.
- **Live plans**: multi-path plans render as a compact compare block; the chosen path streams its steps.
- **Memory beside chat**: the context rail shows exactly which memories/sources grounded the current answer, one click to inspect/correct.
- **Notebook/canvas affordances**: pin a result, branch a thread, turn a result into a memory or an automation ("remember this", "make this a routine") — AI-first shortcuts that remove navigation.
- **Threads, not just sessions**: persistent, searchable, groupable (KEEP recency grouping), with content search (new).
- **Verdict**: 2D. The intelligence is in *structure + live legibility*, not 3D.

---

## 12. COMMAND CENTER (Observatory "Now" + Command Palette)

Two complementary "control" surfaces:

### 12.1 Command Palette (the verb layer, global)
- Invoked anywhere (keyboard + click + voice). Fuzzy over: Spaces, commands, settings, memories, workflows, capabilities, models, sessions, devices.
- Modes: **Go** (navigate), **Do** (run a command/tool/workflow), **Ask** (send to KRIA), **Change** (a setting, natural language).
- This is the primary navigation and the single biggest UX upgrade (today: none).

### 12.2 Observatory "Now" (the state layer)
A single calm control room answering "what is KRIA right now": System health · Models (active/runtime) · Agents/Executive jobs (running/queued, cancel) · Voice state · Tasks/automations in flight · Capabilities health · Performance (CPU/GPU/RAM/VRAM, resource authority) · Background cognition (dream/reflect running) · Recent incidents. Everything drillable into its Space. Replaces the scattered telemetry.

---

## 13. DESIGN SYSTEM (high-level language)

> Direction, not tokens/pixels (implementation defines exact values). Fixes the current dual-token, 73%-hardcoded, 7-dangling-token, 4-greens reality (Inv Part II §G, Part III §C).

- **Foundation: ONE token system.** A single source of truth for color/space/type/radius/elevation/motion/z-index, with full **dark + light** parity. No hardcoded colors in components. No competing palettes. No undefined tokens.
- **Color philosophy**: neutral-dominant graphite/ink canvas; **one accent** (KRIA teal) for intelligence/attention/primary; **one** each semantic (success/warning/danger/info) with soft/solid/text variants; a dedicated **risk ramp** (green→black) reserved for autonomy/approvals only.
- **Typography philosophy**: one expressive display face for identity/headers + one highly-legible text face + one mono for code/logs/hashes. A real type scale (not ad-hoc 10–28px). Generous line-height for calm reading. **Bundle the fonts** (today they're declared but possibly unbundled, Inv Part III §C5).
- **Spacing philosophy**: a strict spacing scale (e.g., 4-based) applied everywhere; generous negative space is the premium signal.
- **Elevation & depth**: a small, meaningful elevation ladder (canvas → panel → floating → modal). Depth = importance/activity. Consistent, soft shadows; no random `0 8px 24px`.
- **Materials / glass**: translucent panels with restrained blur for floating layers (inspector, palette, approvals); opaque for dense work (tables, editors) to keep legibility + performance.
- **Cards**: one card system (header/body/meta/actions) replacing ≥4 current card styles; consistent radius/padding.
- **Buttons/inputs**: one button family (primary/secondary/ghost/danger) + one input family with real focus-visible rings (today most buttons lack focus rings, Inv Part III §E1/§F2). One status-dot/badge component (replaces ≥5).
- **Motion**: one motion system — durations/easings as tokens; motion is event-driven, purposeful, reduced-motion-safe, cheap (§16).
- **Icons**: a single coherent icon set (replace ad-hoc emoji, Inv §C11) — line icons with consistent weight; emoji only where user content.
- **Illustration**: minimal; the Core + soft geometry, not mascots.
- **Charts**: a consistent, low-ink chart style (bars/lines/sparklines) — currently CSS bars everywhere; standardize.
- **Node/graph styles**: a shared visual grammar for 2D node canvas (Automations) and 3D graphs (Memory/Constellation) — same node/edge/label/selection language so graphs feel like one family.
- **Consistency rules**: every surface uses tokens; no inline color; one component per concept; light+dark verified; focus-visible mandatory; risk color never decorative.
- **Visual hierarchy**: size + weight + space + accent (in that order); accent is scarce so it always means "attention here."

---

## 14. USER JOURNEYS (target experience)

- **First launch**: Core greets; a 60-second calm onboarding (name, voice check that actually tests, optional memory cold-start, backend/model pick) → lands in Converse with example intents. Fewer clicks, no dead-end stepper feeling.
- **Daily usage**: open → Core "ready" → type/speak → answer + live work + memory beside → done. Zero navigation for the 90% case.
- **Power user**: ⌘K for everything; manual tool-lock as a mode; multi-thread; pin/branch; "make this a routine."
- **Developer**: one Developer area (Observatory Diagnostics + Settings Developer) instead of scattered dev surfaces; readiness-bypass clearly quarantined as dangerous.
- **Researcher**: Memory Space + graph lens + reasoning history + "why did KRIA answer this" round-trips from chat.
- **Coding session**: compact voice/command; work lane shows tool runs; results become memories/automations.
- **Voice session**: pick a mode from the Core; compact by default; immersive on request.
- **Memory exploration**: landing → search → card → inspector → correct/verify; graph for structure.
- **Automation**: describe in natural language → KRIA drafts → review → run → evidence; schedule by voice.
- **Learning/onboarding**: contextual coach marks the first time a Space/lens is opened; never blocking twice.
- **Debugging**: Observatory Now → drill to failing job/forensic → recovery.
- **Recovery**: KRIA surfaces "I hit a problem, here's why + options" inline; global "stop" always present.
- **Settings**: search or say what to change; grouped; risk-badged; dangerous area clearly separated.
Each journey optimizes: fewer clicks, shallower depth, fewer context switches, higher discoverability (via palette), and always-visible system status + stop.

---

## 15. MOTION SYSTEM

- **Purpose-only motion**: state change, progress, attention, spatial continuity (Space/lens transitions). Nothing ambient except the Core's slow breath.
- **The Core** is the primary animated element and carries most "aliveness" at near-zero cost.
- **Transitions**: Spaces cross-fade/slide with subtle depth; the Inspector slides; the palette scales in — all fast (≈120–200ms), all interruptible.
- **Work lane**: steps appear with a gentle reveal; running items pulse subtly; done items settle.
- **3D lenses**: animate only during interaction; settle to a static frame; never loop in background.
- **Reduced motion**: honored globally *including* the Core and graph loops (extend today's global rule which JS loops currently escape, Inv Part III §L4).
- **Tokens**: durations/easings standardized; no bespoke timings per component.

---

## 16. ACCESSIBILITY & RESPONSIVE (targets)

- **A11y**: WCAG 2.2 AA as the bar. Semantic landmarks (nav/main/aside), heading order, real focus-visible everywhere, full keyboard model (palette makes this natural), labeled controls (fix unlabeled selects), live regions for KRIA state (build on current `role=status`/`aria-live`), real tables for tabular data (fix div-grids), risk never color-only (icon+text), contrast verified in both themes. Reduced-motion + high-contrast + font-scale first-class and actually mapped.
- **Responsive**: desktop-first but not desktop-only. Define real breakpoints (today ~1 meaningful desktop bp, Inv Part III §M). The shell reflows: Dock → bottom bar on narrow; rails collapse; Inspector becomes an overlay. Mobile companion stays its own optimized surface. Set sensible min-widths; no silent overflow.

---

## 17. MIGRATION MAP (current → target, at a glance)

| From (current) | To (target) |
|---|---|
| 7 flat routes + top-nav | 7 Spaces + Dock + Command Palette |
| 21-tab Settings modal | Settings Space (grouped, searchable) |
| Buried n8n | Automations Space (top-level) |
| Tasks + Automation + Reminders (split) | Automations Space (unified) |
| CPP + OpenClaw + Providers + MCP (split) | Capabilities Space (unified) |
| Dashboard + Analytics + Executive + Resource + Ironclad (4–6 telemetry) | Observatory Space (one) |
| VM + Mobile/Remote (split) | Machines Space (unified) |
| Hidden Prompt Lab env | Converse "Lab" mode |
| 4 approval UIs | one Approval Center |
| scattered modals/toasts/detail panes | Context Inspector + Notification Center |
| top chips + status bar + status dots | Core + one status line |
| no command palette | Command Palette (primary nav) |
| 8 orphaned dead pages | removed or revived into a Space |
| 2 token systems, 73% hardcoded, broken light | one token system, dark+light parity |
| 2D SVG memory graph | 3D Knowledge Graph lens (2D fallback) |
| no capability overview | Capability Constellation lens |
| full-screen voice only | Core-centric voice, compact by default |

---

## 18. SELF-REVIEW PASSES (challenging this plan)

- **Pass 1 — Too many Spaces?** 7 is the ceiling; each maps to a distinct intent and absorbs multiple current surfaces. Fewer would overload a Space; more would fragment. Kept at 7. ✔
- **Pass 2 — Is 3D justified or decoration?** Restricted to 3 lenses (Memory graph, Capability constellation, plan-compare 2.5D) — each passes the "topology-is-the-insight" test. Explicitly rejected 3D for the n8n builder, execution flow, tasks, code (would harm usability). ✔
- **Pass 3 — Does the Core replace real information?** No — it *summarizes* state; text/detail remain. It removes redundant dots/pills, not data. ✔
- **Pass 4 — Command Palette overreliance?** Palette is primary but not exclusive: Dock + in-Space nav + context menus + floating AI actions all coexist for discoverability. ✔
- **Pass 5 — Approval Center as single point?** Unifying 4 UIs risks a bottleneck; mitigated by inline context (approvals show *where* they arose) + the Core signaling "needs you." ✔
- **Pass 6 — Low-power vs "alive"?** Aliveness concentrated in one cheap Core; 3D on-demand + frozen-when-idle; motion event-driven. Meets the hardware budget. ✔
- **Pass 7 — Did we preserve what's good?** Explicitly KEEP: tool-result cards, GUI-cognition two-layer disclosure, recovery-options, honest empty states, schema-driven setting badges, risk-phrase confirms, mobile companion, n8n prepare-input/evidence flow. Not change-for-change's-sake. ✔
- **Pass 8 — Anything still fragmented?** Integrations appear in both Capabilities (connect) and Settings (prefs) — resolved by rule: *workspaces live in Capabilities, preferences live in Settings, cross-linked*. ✔
- **Pass 9 — Accessibility not an afterthought?** A11y + responsive are targets (§16), and the palette/keyboard model is core, not bolted on. ✔
- **Pass 10 — Could a designer build from this?** Yes for IA, navigation, Spaces, voice, memory, chat, design-language direction, motion, journeys. Remaining depends on visual comps + exact tokens (implementation stage). ✔

---

## 19. OPEN QUESTIONS (for the design phase, not blockers)
- Exact token values, type scale, and comps (visual design stage).
- Precise Core visual language (needs motion/visual exploration + performance test).
- 3D lens fidelity vs the low-power budget (needs a spike to tune node caps / frozen-frame strategy).
- Whether Prompt-Lab "mode" and manual tool-lock should be one control or two.
- Approval Center density model for bursts of many simultaneous approvals.
- Mobile companion scope (how much of each Space is worth mirroring).

---

*KRIA UI/UX Redesign Masterplan — product/UX/interaction design bible. Grounded in `KRIA_UI_INVENTORY.md` (current state), challenges it where warranted, and defines the target AI-Operating-System experience: calm, premium, spatial, hybrid 2D+3D, local-first, supervisable. No implementation/technology content by design.*

---
---

# PART B — THE KRIA VISUAL DESIGN BIBLE

> Part A (§0–19) defined **what** KRIA becomes (identity, IA, Spaces, philosophy). Part B defines **how** it looks, feels, moves, composes, and behaves — to the point where a design + engineering team can execute without inventing major decisions. Design/UX/visual/interaction/IA only — no technology.

## 20. SELF-AUDIT OF PART A (critique before expansion)

Treating Part A as if a peer wrote it, the honest gaps:
1. **Too much WHAT, not enough HOW.** It names "calm premium spatial" but never specifies composition, proportion, reading path, material behavior, or how the eye is guided. → §22–§26.
2. **No layout proportions.** Spaces are described but not sized (rail widths, canvas ratios, what's sticky vs scrollable, how it transforms). → §25.
3. **Component model is shallow.** Names components but no hierarchy tree, no importance weighting, no reuse contract, no per-state interaction spec. → §26–§29.
4. **The Core is asserted, not designed.** "One living orb" — but its full state language (14+ states), body/movement/emotional grammar is undefined. → §30.
5. **3D says WHERE, not HOW.** No camera/light/material/motion/fallback/freeze rules. → §31.
6. **No attention model / storytelling.** What the eye notices at 1s/5s/30s/5min/months is unspecified. → §32.
7. **Premium is invoked, not distilled.** References Apple/Linear/Arc but doesn't extract the *principles* and reinterpret them for KRIA. → §21.
8. **No immutable design laws.** Nothing caps accent colors, floating layers, nesting depth, density, motion length → drift risk (the exact disease of the current app). → §24.
9. **Flows are listed, not mapped.** Journeys named but not drawn end-to-end with decision/approval/execution nodes. → §33.
10. **No delight/wow definition, no emotional arc.** Premium ≠ merely clean; the "alive intelligence" feeling needs explicit moments. → §30/§32/§36.
11. **Missing future-proofing rules.** Where does feature #200 go? Undefined. → §35.
12. **Ranking not tied to visual weight.** Importance is listed but not converted into a visual-emphasis rule. → §34.
13. **Reuse/consistency enforcement absent.** No mechanism preventing the re-emergence of "4 greens / 5 button styles." → §24/§28.

Part B fixes all thirteen.

---

## 21. PREMIUM DESIGN LANGUAGE — PRINCIPLES DISTILLED → KRIA

We don't copy Apple/Linear/Arc; we extract *why* they feel premium and reinterpret.

| Reference | Why it feels premium (principle) | KRIA reinterpretation |
|---|---|---|
| **Apple (macOS/visionOS)** | Deference — UI recedes so content leads; depth via light + translucency, never lines; motion explains spatial change | KRIA canvas is deep and quiet; panels float on soft light; every transition explains *where you went*, never decorates |
| **Linear** | Ruthless restraint; one accent; keyboard-first; speed as luxury; perfect alignment | KRIA: one teal accent, Command Palette as spine, instant response, strict grid |
| **Arc** | Playful calm; spaces/profiles; the browser "gets out of the way"; delightful micro-moments earned, not constant | KRIA: Spaces model, a get-out-of-the-way home, delight reserved for meaningful moments (a completed plan, a learned memory) |
| **Notion** | Content as blocks; infinite composability from few primitives; calm typography | KRIA work-lane = typed "work blocks"; few primitives, many compositions |
| **Cursor** | AI woven into the workspace, not bolted on; the model's work is *legible inline* | KRIA: reasoning/tools/plans live beside the conversation |
| **Anthropic/OpenAI product** | Trust through legibility + restraint; the model shows its reasoning and limits | KRIA: "show the work," honest states, visible memory grounding, supervisable autonomy |
| **Tesla/industrial UI** | Calm authority; big legible state; one glance = full situational awareness | KRIA Core + one status line = instant "what is KRIA doing" |

**The five KRIA premium laws (derived):**
1. **Deference** — the interface serves the thought; chrome is thin, content and Core lead.
2. **Restraint** — scarcity of color/motion/borders makes each one meaningful.
3. **Depth over lines** — hierarchy from light, elevation, and translucency, not boxes and dividers.
4. **Speed is luxury** — instant, interruptible, keyboard-first; latency is hidden by the Core, not by spinners.
5. **Earned delight** — moments of beauty attach to meaningful events (learning, completing, connecting), never to idle decoration.

---

## 22. KRIA VISUAL IDENTITY SYSTEM ("Ink & Aura")

The signature look, named so the team has one target: **Ink & Aura** — deep ink space, precise typographic content, and a single living aura (the Core).

### 22.1 The canvas (the "room")
- A deep, dark, dimensional ground — near-black graphite with a **soft radial vignette of light** as if a calm source sits behind the Core. Not flat black (cold), not busy (grids/particles). In light theme: a soft paper-white with the same gentle depth.
- The canvas is **still**. It never animates on its own. Depth is static; life comes only from the Core and purposeful transitions.
- **Why**: stillness + depth reads as "premium room," not "screensaver." It also costs nothing.

### 22.2 Surfaces & material
- **Three material tiers**, each with a job:
  1. **Content surfaces** (work canvas, tables, editors, chat): near-opaque, calm, high legibility — no blur (reading + performance).
  2. **Floating surfaces** (Command Palette, Context Inspector, Approval Center, popovers): translucent "aura glass" with restrained blur + a single soft shadow + a hairline light edge — they feel *above* the room.
  3. **The Core**: the only luminous element — an aura that emits, not a panel.
- **Rule**: blur is reserved for *floating* layers only. Never blur content (legibility + cost). This prevents "glass overload."

### 22.3 Light & elevation
- One implied light source (behind/above the Core). Elevation is expressed by **how much light a surface catches** (edge highlight) + shadow softness — a 4-step ladder: Canvas (0) → Panel (1) → Floating (2) → Modal/Approval (3). Nothing exceeds tier 3.

### 22.4 Color in practice
- **Neutrals carry the UI** (graphite ink scale, ~6 steps). **Accent teal is scarce** — it marks: active Space in the Dock, the Core, the single primary action on a surface, live AI activity, and focus. If accent appears more than a few times on one screen, the composition is wrong.
- **Semantic + risk**: one success/warning/danger/info (soft/solid/text) + a reserved **risk ramp** (Green→Yellow→Red→Black) used *only* for autonomy/approval consequence. Risk color never decorates.
- **Emotion via temperature**: KRIA "thinking" leans cool-calm; "success/learned" gives a brief warm-teal bloom; "blocked/risk" desaturates the room slightly and raises the risk halo. Emotion is conveyed by *restraint shifts*, not new palettes.

### 22.5 Typography in practice
- **Display face** (identity/greeting/section titles): a distinctive geometric-humanist face with presence — used sparingly and large.
- **Text face**: a highly legible neutral face for all reading (chat, memory, settings) — generous line-height (calm), comfortable measure (~60–75 chars in chat).
- **Mono**: code, logs, hashes, evidence, terminal.
- **Type scale**: a real ratio-based scale (display / title / heading / body / caption / micro) — six roles, no ad-hoc sizes. Weight does hierarchy work before size does.
- **Why**: typography is 80% of a premium calm feel; the current app's ad-hoc 10–28px sizing is the opposite of this.

### 22.6 Iconography & illustration
- One coherent **line-icon set**, single stroke weight, geometric, calm. Emoji only inside user content, never as system iconography (fixes current emoji-as-icon).
- Illustration is minimal and geometric: the Core, subtle constellation motifs, soft light — no mascots, no stock 3D.

---

## 23. VISUAL COMPOSITION RULES

### 23.1 Global composition doctrine
- **One focal point per view.** Every screen has a single primary thing (in Converse: the answer forming; in Memory: the graph or the card; in Approval: the decision). Everything else is quieter.
- **Reading path = Z or F, guided by light + accent.** The eye enters at the Core (top-left presence), reaches the intent bar (top-center), drops into the canvas focal point, then to actions (bottom/right). Accent + elevation place the "next step" exactly where the eye lands.
- **Whitespace is the primary premium signal.** Default to generous negative space; density is *earned* by the task (tables, graphs). Never fill space just because it exists.
- **Grouping by proximity + shared surface**, not by borders/dividers. Related things sit together on one calm surface; unrelated things get space between them.
- **Alignment**: one strict grid; everything aligns to it; optical alignment for icons/type. Misalignment is the fastest way to look cheap.
- **Density tiers**: Calm (home/idle) → Focused (a task) → Dense (tables/logs/graphs). A Space shifts tier with the task; it never opens dense.
- **Balance**: asymmetric-but-weighted — the Core anchors left, content mass centers, actions/inspector balance right.

### 23.2 Per-Space composition (focal + reading path + density)
- **Converse**: focal = the forming answer/Core. Path: Core → intent bar → latest message → work lane (only if active). Density: Calm → Focused when work runs. Whitespace protects readability; work lane is visually secondary (quieter surface) so reading isn't disrupted.
- **Memory**: focal = the graph (in graph lens) or the selected card. Path: search → results → card → relationships. Density: Focused; graph lens is Dense but centered and calm-bordered.
- **Automations**: focal = "what can run" / the active run. Path: landing cards → run → evidence. Builder is Dense (precision) but framed in calm.
- **Capabilities**: focal = the ability in question / the constellation. Path: overview → item → inspector → grant. Constellation lens is the "wow" moment, centered.
- **Machines**: focal = fleet health / remote canvas. Path: matrix → machine → action. Remote = immersive (content fills).
- **Observatory**: focal = the system pulse (Now). Path: pulse → drill. Dashboards are Dense but calm; one accent per card max.
- **Settings**: focal = search + the group you chose. Path: search/group → setting → change. Calm density throughout.

---

## 24. UNIVERSAL DESIGN LAWS (immutable)

These are permanent constraints. Violating them is a design bug. They exist to prevent the current app's drift (73% hardcoded color, 5 button styles, 6 telemetry surfaces).

**Color & material**
- Max **1** accent hue family (teal) across the entire OS.
- Max **4** neutral surface tiers + **4** elevation steps.
- Semantic colors: exactly **1** each (success/warning/danger/info). Risk ramp used ONLY for autonomy/approval.
- Blur allowed ONLY on floating layers. Never on content.
- **0** hardcoded colors in any surface — everything from one token system, dark+light parity.

**Layout & navigation**
- Max navigation depth **2** to reach any feature (Dock → Space, or Palette → anything). Nothing is 3+ levels deep (kills today's buried-n8n problem).
- Max **1** modal at a time (a modal may not open a modal). Approvals queue; they don't stack.
- Max **1** persistent right Inspector (details never open new windows).
- Max **3** primary panels visible at once in a Space (e.g., Converse: conversation + work + context — the third is optional/collapsible).
- Max **1** primary action per surface (one accent button); everything else is secondary/ghost.

**Density & information**
- A Space must be able to open in **Calm** density (never dense-by-default).
- Max card density: content must never require reading faster than comfortable; if it would, paginate/segment.
- Minimum whitespace: content never touches container edges; consistent breathing room from the spacing scale.

**Motion**
- Max transition duration **~200ms** for UI moves; **~400ms** only for a deliberate spatial Space change. Nothing longer.
- Ambient motion allowed on the Core ONLY. Everything else is event-driven and settles.
- All motion respects reduced-motion, including the Core and 3D lenses.

**Consistency**
- One component per concept (one Button family, one Card, one StatusDot, one Chip, one Input, one Table, one Graph-node grammar). No bespoke re-implementations.
- Every interactive element has a visible focus state.
- Risk/consequence is never communicated by color alone (icon + text always).

---

## 25. REAL PAGE LAYOUT BLUEPRINTS (proportions + regions + behavior)

Proportions are design targets (relative), not pixels. Regions labeled: **[P]** persistent · **[A]** adaptive · **[C]** collapsible · **[S]** sticky · **[scroll]** scrollable.

### 25.1 Global shell
```
[S] Presence Bar  (thin, ~48–56 tall)                                     [P]
 ● Core+state (left)      ⌘ Intent/Command bar (center, ~40% width)   ◇Approvals ⚙ (right)
────────────────────────────────────────────────────────────────────────────
[P]Dock │            SPACE CANVAS  [A]                          │ Context Inspector [C]
~64 w  │            (fills; internal layout per Space)          │  ~360–420 w, slides
(icons)│                                                        │  over on narrow [A]
────────────────────────────────────────────────────────────────────────────
[S] Status line (one line, ~28 tall): "what KRIA is doing now"            [P]
```
- Dock collapses to pure icons always (never wider than ~64); labels on hover/focus.
- Inspector is one shared panel; slides in from right; becomes a full overlay under ~1000-wide.
- Presence bar + status line are always present; everything else adapts.

### 25.2 Converse (home)
```
Sidebar[C]     │ CONVERSATION lane [A][scroll]      │ WORK lane [C][A]   │ CONTEXT rail [C]
threads ~260   │ ~ fills (max measure ~720 center)  │ ~380 (auto-opens   │ ~300 (memory/
(collapsible)  │  messages + inline cards           │  when KRIA acts)   │  model/tools)
               │ ────────────────────────────────── │  live steps/tools/ │
               │ [S] Composer (bottom, sticky)       │  plans/gui/runs    │
```
- Resting state: sidebar + conversation only (Calm). Work lane appears on activity; context rail on demand. On narrow widths, work/context become bottom sheets or Inspector.
- Composer is sticky-bottom, grows with input, never covers the last message.

### 25.3 Memory
```
[S] Memory header: search + lens segments (Explorer·Timeline·Goals·Reasoning·Library·Graph)
Landing (Calm): overview tiles + recent + gaps + big search
Lens = Explorer:  results list [scroll ~40%]  |  (Inspector holds the card detail)
Lens = Graph:     3D graph fills center; slim controls top-right; Inspector = node detail
```
- Graph lens: canvas-dominant, controls minimal and edge-docked; 2D fallback list toggle always present.

### 25.4 Automations
```
[S] header: segments (Run · Build · Schedule · History)
Run:    "ask KRIA to pick" bar + ready-to-run cards grid [scroll]; run status inline
Build:  2D node canvas [A] fills; node inspector = right Inspector; palette of nodes left [C]
History: runs timeline (left) + selected run evidence (Inspector)
```

### 25.5 Capabilities
```
[S] header: segments (Tools · Skills · Models · Integrations · Governance · Constellation)
List segments: searchable rows/cards [scroll]; detail = Inspector (descriptor/effects/trust)
Constellation: 3D field fills center; filters edge-docked; node = Inspector
```

### 25.6 Machines
```
[S] header: fleet summary chips
Matrix: device table [scroll] (left ~60%) | focused terminal + alerts (right ~40%) [C]
Remote: immersive canvas fills; floating toolbar (auto-hide); keyboard bar on demand
```

### 25.7 Observatory
```
[S] header: segments (Now · Jobs · Analytics · Forensics · Diagnostics)
Now: system pulse (Core-linked) hero + a calm grid of state cards [scroll]
drill: any card → Inspector or its Space
```

### 25.8 Settings
```
[S] search ("change what?") + group rail [P ~240]
group body: sections of setting rows [scroll]; risk/restart/env badges inline
Developer group visually quarantined (distinct, guarded)
```

### 25.9 Layout transformation rules
- **Wide → narrow**: context rail collapses first, then work lane → bottom sheet, then sidebar → overlay, then Dock → bottom bar. Presence bar + status line always survive.
- **Focus mode**: any Space can hide all rails (F-key) leaving canvas + Core — for deep reading/work.
- **Persistent everywhere**: Core, Intent bar, Approvals, status line. Everything else is adaptive/collapsible.
---

## 26. COMPONENT HIERARCHY (per surface)

Trees: Section → Container → Widget → Component → Subcomponent.

### 26.1 Converse
```
Converse
├── Sidebar (Threads)
│   ├── New/Voice quick actions
│   ├── Search
│   └── Thread list → Thread row (title · pin · badge · actions[persistent])
├── Conversation lane
│   ├── Message stream
│   │   ├── Message (user/assistant/system)
│   │   │   ├── Content (markdown/text)
│   │   │   ├── Inline AI cards (search/web/image/google result)
│   │   │   ├── Message actions (copy·retry·explain·remember·feedback)
│   │   │   └── Timestamp/role
│   │   └── Empty/Welcome (Core + example intents)
│   └── Composer [sticky]
│       ├── Input (grow)
│       ├── Attachments (chips/preview)
│       ├── Mode chip (Assistant/Lab/tool-lock)
│       ├── Voice entry
│       └── Send / Stop (single primary)
├── Work lane [adaptive]
│   ├── Work block: Reasoning step
│   ├── Work block: Tool call (status·args·evidence·stop)
│   ├── Work block: Plan compare (paths·winner·steps)
│   ├── Work block: GUI-cognition (layman + details)
│   └── Work block: Workflow run (progress·evidence)
└── Context rail [collapsible]
    ├── Memory-used cards
    ├── Active model
    └── Active tools/capabilities
```

### 26.2 Memory
```
Memory
├── Header (search + lens segments + live indicator)
├── Landing (overview tiles · recent · gaps)
├── Explorer (results list → Memory card)
├── Timeline (time rows)
├── Goals & Plans (goal tree · plan-compare)
├── Reasoning & Causal (trace list · chains)
├── Library (doc cards)
├── Graph lens (3D field · controls · fallback list)
└── Inspector: Memory detail (content·confidence·worth·truth·source·conflicts·lineage·version·AI explanation·actions)
```

### 26.3 Automations
```
Automations
├── Header (segments)
├── Run (ask-KRIA bar · workflow cards · prepared-input preview · run status)
├── Build (node palette · canvas · node inspector)
├── Schedule (task rows · routine editor)
├── Reminders (reminder rows)
└── History (runs timeline · evidence viewer)
```

### 26.4 Capabilities
```
Capabilities
├── Header (segments + search)
├── Tools (capability rows → descriptor inspector)
├── Skills (skill cards · trust · install review)
├── Models (provider cards · test/apply · runtime)
├── Integrations (connection cards)
├── Governance (proposals · quarantine · grants · autonomy)
├── Generate (goal → preview → synthesize)
└── Constellation lens (3D field · filters · node inspector)
```

### 26.5 Machines / Observatory / Settings
```
Machines: Header · Matrix(device rows·terminal·alerts) · Enroll wizard · Remote canvas(toolbar·keyboard bar) · device Inspector
Observatory: Header · Now(pulse hero·state cards) · Jobs(job rows·cancel) · Analytics(tiles) · Forensics(timeline) · Diagnostics(dev)
Settings: Search · Group rail · Setting rows(control + risk/restart/env badge) · Developer(guarded)
```

### 26.6 Global layers
```
Presence bar: Core · Intent/Command bar · Approvals · Settings
Command Palette: input · mode(Go/Do/Ask/Change) · result groups · result row
Approval Center: queue · Approval card(what·why·risk·effects·evidence·actions) · scope options
Notification Center: notification row(type·summary·action)
Context Inspector: header · body(context-typed) · actions
Core/Voice: aura · state label · live transcript line · mode chooser
```

---

## 27. COMPONENT IMPORTANCE CLASSIFICATION

Weight = visual prominence a component may claim. Drives size/contrast/placement.

| Class | Meaning | Examples | Visual weight |
|---|---|---|---|
| **Primary-persistent** | Always present, defines the OS | Core, Intent/Command bar, Dock, status line | Highest presence, but calm |
| **Primary-contextual** | The focal point of the active Space | forming answer, memory graph, approval card, remote canvas | Dominant while active |
| **Secondary** | Supports the focal task | work lane, context rail, filters, segments | Quieter surface, lower contrast |
| **Contextual/on-demand** | Appears for a task then leaves | Inspector, palette, prepared-input preview, tool cards | Elevated, transient |
| **Temporary** | Brief, self-dismissing | toasts→notifications, inline confirms, streaming step | Light, non-blocking |
| **Rare/critical** | Infrequent but must dominate when present | Approval, Hard-reset confirm, kill switch | Interrupts, high contrast, risk color |
| **Hidden/expandable** | Progressive disclosure | "details" accordions, raw output, dev panels | Zero weight until opened |
| **Developer-only** | Gated | Diagnostics, readiness-bypass, Ironclad config | Visually quarantined |
| **AI-generated** | KRIA-authored content | work blocks, suggestions, drafts, memories | Marked with a subtle AI provenance cue |
| **User-generated** | User content | messages, notes, uploads | Neutral, primary in reading |

**Rule**: a component may never claim weight above its class. AI-generated content always carries a quiet provenance cue so the user can always distinguish KRIA's words/actions from their own (trust requirement).

---

## 28. COMPONENT REUSE MAP (the design-system kit)

One kit, reused everywhere. "Never changes" = invariant contract; "changes" = allowed variants.

| Component | Reused in | Never changes | May change (variants) |
|---|---|---|---|
| **Button** | everywhere | one family, focus ring, one primary/surface | primary/secondary/ghost/danger; size s/m |
| **Input / Select / Textarea** | composer, forms, search | focus-visible ring, label pattern | size, inline vs stacked |
| **Card** | memory, capability, workflow, device, result | header/body/meta/actions structure, radius, padding scale | accent edge (status), media slot |
| **Chip / Badge** | status, tags, risk, filters | shape, size, one semantic meaning | tone (neutral/success/warn/danger/risk) |
| **StatusDot** | Core-linked states, devices, services | single dot grammar + label | color = one semantic only |
| **Row (list item)** | threads, memories, jobs, devices, settings, results | height rhythm, hover, selection, keyboard | leading media, trailing action |
| **Segment/Tab bar** | Space sub-nav | underline-active, keyboard | count badges |
| **Table** | fleet, analytics, tests | real tabular semantics, one header/row style | column density |
| **Inspector panel** | all detail views | slide-in, header/body/actions, one-at-a-time | body content type |
| **Approval card** | all HITL/decisions | what·why·risk·effects·evidence·actions | risk tone, scope options |
| **Work block** | Converse work lane | typed block w/ status + stop + details | block kind (reason/tool/plan/gui/run) |
| **Graph node/edge** | Memory 3D, Constellation 3D, task/plan 2D | shared node/edge/label/selection grammar | 2D vs 3D render, cluster color |
| **Progress/meter** | image gen, runs, resources, confidence | one bar grammar | determinate/indeterminate |
| **Empty state** | every list/space | icon+headline+one action, honest tone | copy |
| **Toast/Notification** | global | one row grammar, non-blocking | type |
| **Modal/Confirm** | destructive/high-risk only | one modal shell, one-at-a-time | risk level |
| **Wizard** | setup, enroll, cold-start | stepper + back/next + skip rules | steps |
| **The Core** | global | one aura, one state map | state (see §30) |

**Duplication prevention rule**: no surface may invent a card/button/dot/graph style; if a need isn't covered, the *kit* is extended (a new variant, reviewed), never a one-off. This is the structural cure for the current app's fragmentation.

---

## 29. INTERACTION BIBLE (per-state behavior)

Applies to every interactive component; specifics noted where they differ. Micro-motions are ≤200ms, purposeful, reduced-motion-safe.

### 29.1 Universal states
- **Default**: calm; no accent unless it is the surface's single primary.
- **Hover**: subtle surface lift (light, not movement); reveals row actions *without* layout shift (actions are always allotted space — no hover-only hidden affordances like today).
- **Focus (keyboard)**: a clear, consistent focus ring on **every** interactive element (fixes current gap). Focus is never invisible.
- **Pressed**: brief inset/dim; immediate (speed = luxury).
- **Selected**: accent edge + raised surface; persists; keyboard-navigable.
- **Disabled**: reduced contrast + not focusable + a reason on hover/inspect (never a mystery).
- **Loading**: the component shows local progress; global waiting is carried by the Core, not spinners scattered everywhere.
- **Streaming** (AI): content/work blocks reveal progressively with a gentle typing/step cadence; a stop is always present.
- **Success**: brief warm-teal confirmation bloom, then rest; no lingering banners.
- **Warning/Failure**: inline, plain-language, with recovery action; risk color + icon + text (never color alone).
- **Recovery**: failed actions always offer a next step (retry/alternative/explain) — never a dead end.

### 29.2 Input methods
- **Keyboard**: full model — ⌘K palette; Space/Dock switching; arrow/enter in lists; Esc closes top-most transient layer (one level at a time); Enter=send, Shift+Enter=newline in composer; type-ahead in palette and search.
- **Mouse**: hover reveals, click selects, double-click opens/renames, right-click = context menu (new: message/memory/file/device/node menus).
- **Drag/drop**: files into composer; nodes on the 2D builder canvas; memory node reposition in graph; reorder where meaningful. Drag has a clear ghost + drop target highlight.
- **Touch** (mobile companion + touchscreens): ≥44px targets, swipe between mobile tabs, long-press = context menu, pinch/pan in canvases.
- **Voice**: any primary action is voice-addressable; voice never requires visual focus; the Core shows voice state.

### 29.3 Key component specifics
- **Composer**: grows to a max then scrolls; Send is the single primary; while KRIA works, Send becomes **Stop** (prominent, always reachable). Draft persists per thread.
- **Message**: hover/right-click → copy/retry/explain/remember/branch/feedback; selection enables "ask about selection."
- **Work block**: expandable details (layman↔technical); running blocks pulse subtly; each has an independent stop; completed blocks settle and can become a memory/automation.
- **Approval card**: primary = the *safe* default is visually secondary is NOT assumed — the card states the risk; Approve requires deliberate action; Deny/keep-paused is always one click; high-risk requires the typed/deliberate confirm; every approval shows what will happen, effects, and evidence.
- **Graph node**: hover = highlight neighborhood; click = select + Inspector; double-click = focus/expand; drag = reposition (pins); keyboard = list-based alternative (accessibility).
- **Inspector**: opens without moving the canvas; Esc or click-away closes; only one open.
- **Command Palette**: opens instantly, remembers recent, fuzzy, grouped results, arrow+enter, Esc closes; typing a question routes to "Ask."
- **Undo/Redo**: destructive-but-reversible actions (delete memory, archive workflow, remove device) show a brief **Undo** in the notification, not a pre-confirm; irreversible actions use the deliberate confirm. Config changes are undoable via the settings history.

### 29.4 Discoverability mechanisms
- First entry to any Space/lens shows a one-time, dismissible coach hint (never twice).
- The Command Palette surfaces "you can also…" contextual actions.
- Empty states teach the primary action.
- Hover tooltips (delayed) on all icon-only controls.

---

## 30. THE KRIA CORE — AI PERSONALITY & PRESENCE

The Core is KRIA's face: one luminous aura that expresses system state through **light, breath, density, and temperature** — never text-only, never a cartoon. It is the emotional and trust anchor of the OS.

### 30.1 Design grammar (the four dials)
- **Breath** (scale/opacity rhythm): slow = calm/idle; quicker = engaged; still = blocked/attention.
- **Density** (internal structure): sparse = idle; gathering/converging = thinking; radiating = speaking; concentrated = focused acting.
- **Temperature** (hue within the teal family): cool = neutral/thinking; warm bloom = success/learned; desaturated + risk halo = blocked/permission.
- **Light** (glow reach): soft = ambient; brief brighten = event; dimmed = idle/low-power.
All four are cheap to render and combine into a readable vocabulary.

### 30.2 State language (complete)
| State | Breath | Density | Temp | Light | Supporting cue |
|---|---|---|---|---|---|
| **Idle** | slow | sparse | cool | soft/dim | "Ready" (subtle) |
| **Listening** | inward pull | gathering | cool | responsive to voice level | live level ring |
| **Thinking / Reasoning** | medium | converging swirl (calm) | cool | soft | "Thinking" + work lane opens |
| **Planning** | medium | branching internal motion | cool | soft | plan-compare block appears |
| **Speaking** | outward | radiating | slightly warm | pulse with cadence | transcript line |
| **Acting (GUI/automation)** | steady | concentrated, directed | neutral | focused beam | "Acting on <target>" |
| **Running automation (bg)** | slow | small orbiting satellite | cool | dim | status line + Observatory |
| **Watching (meeting/ambient)** | very slow | quiet | cool | very dim | "Listening, not responding" |
| **Remembering (write)** | brief inward | quick absorb | warm flick | brief brighten | memory toast |
| **Reflecting / Dreaming (bg cognition)** | very slow, dreamlike | soft internal drift | cool-violet hint | very dim | Observatory badge |
| **Learning** | brief | crystallize | warm bloom | brighten then settle | "Learned: …" |
| **Waiting (on user)** | paused | held | neutral | steady | inline prompt |
| **Blocked / Needs permission** | still | held + risk halo | desaturated | risk-tinted edge | Approval surfaces + Core points to it |
| **Error / Recovering** | irregular→settling | unsettled→calm | brief risk then neutral | dim→soft | plain-language recovery |

### 30.3 Body & movement language
- The Core has a **home** (top-left presence) but can **project** into a Space for a moment when relevant (e.g., blooms toward a just-learned memory, or leans toward the approval that needs the user). These are rare, brief, meaningful "glances" — the OS's body language. Never wandering, never distracting.
- Movement is **eased and organic** (breath-like), never mechanical/spinning. Spinning = generic loader = banned.

### 30.4 Emotional & trust rules
- The Core never fakes confidence: uncertainty reads as calmer/cooler, not louder.
- Risk/blocked states **reduce** stimulation (still, desaturated) so the user's attention goes to the decision, not to animation.
- The Core is honest and legible at a glance — a user across the room knows KRIA's state from the aura alone.
- **Personality**: competent, calm, unobtrusive, warm-on-success. Not chatty, not cute, not theatrical. "A brilliant, composed colleague," not "an assistant character."

### 30.5 Voice = Core
Voice states are Core states (§9 + §30.2). Voice never spawns a separate full-screen unless the user chooses immersive; default is Core + one transcript line so the workspace stays usable.

---

## 31. 3D PHILOSOPHY — HOW (per lens)

3D exists in exactly three places (§4.3): **Memory Knowledge Graph**, **Capability Constellation**, and **Plan-compare (2.5D)**. Rules below make 3D calm, legible, and cheap.

### 31.1 Universal 3D rules
- **Camera**: gentle perspective (near-orthographic) so structure is readable and nodes don't distort; slow, damped orbit; constrained tilt (no disorienting free-fly); a "reset view" is always one action.
- **Lighting**: single soft key light + ambient fill matching the Ink & Aura room; no harsh speculars, no lens flares. Selected/active nodes self-illuminate (accent).
- **Depth cues**: size, focus/blur (subtle), and brightness by distance — enough to read topology, not enough to hide labels.
- **Materials**: matte, calm, translucent for clusters; the accent reserved for selection/activity. No glossy sci-fi chrome, no neon.
- **Motion/parallax**: subtle parallax on interaction only; the field is **static when idle** (frozen frame). Layout physics run only until settled, then stop.
- **Interaction**: hover = highlight node + its edges, dim the rest (focus+context); click = select + open Inspector; double-click = focus/zoom to neighborhood; drag = orbit (empty space) or move node (on node); scroll = zoom (bounded).
- **Selection/focus**: selected node rises in brightness; neighborhood stays lit; unrelated field dims — never fully hidden (keeps context).
- **Transparency/occlusion**: distant/unrelated nodes fade rather than clip; labels always legible for focused set; occluded labels resolve on focus.
- **Transitions (2D↔3D)**: entering a lens is a smooth "pull into depth" (~300–400ms); exiting flattens back to the 2D landing. The transition explains the spatial move.
- **Entry/exit**: 3D wakes on lens open/interaction; **freezes to a static image when unfocused or after ~a few seconds idle**; fully unloads when the Space is left.
- **Fallback**: every 3D lens has an always-available **2D list/table** of the same data (accessibility, low-power, keyboard). 3D is a lens, never the only way.
- **Performance budget**: hard node/element caps with graceful "showing top N by relevance"; no continuous simulation; no background rendering; honors reduced-motion (renders static). If the device is under model load, the lens degrades to 2D automatically.

### 31.2 Per-lens specifics
- **Memory Knowledge Graph**: clusters = communities (calm cluster tint), node size = centrality, edges = relationships (predicted edges dashed/dim), focus a node to see its neighborhood + Inspector; time filter; search re-centers. Purpose: understand structure of what KRIA knows.
- **Capability Constellation**: grouped by domain/provider, brightness = health, dim = quarantined, accent = active; select to inspect/grant. Purpose: grasp the breadth + relationships of KRIA's abilities at a glance.
- **Plan-compare (2.5D)**: the 3 planner paths as shallow layered lanes with the winner raised/accented; steps stream on the active path. Barely 3D — depth only to separate paths. Purpose: compare paths without a flat wall of text.

### 31.3 3D anti-rules (never)
- Never render 3D on the home/idle canvas.
- Never use 3D for editing precision tasks (the Automations node builder is 2D).
- Never use 3D for decoration, transitions between non-spatial pages, or "because it looks cool."
- Never trap the user in 3D without a one-action escape to 2D.

---

## 32. VISUAL STORYTELLING & ATTENTION MODEL

### 32.1 What the eye notices (ranked, by design)
1. **First**: the Core (presence — "something intelligent is here").
2. **Second**: the single focal point of the Space (the forming answer / the graph / the decision).
3. **Third**: the primary action or the intent bar (where to go next).
4. Then: secondary supports (work lane, rail, lists) — intentionally quieter.
Accent + light + elevation enforce this order; nothing competes with the focal point.

### 32.2 Emotional arc over time
- **@1 second**: "This is calm and alive. I'm not overwhelmed." (Core breathing, quiet room, clear one focal thing.)
- **@5 seconds**: "I know exactly what to do and where I am." (Intent bar + focal point + Dock legible.)
- **@30 seconds**: "It's showing me its thinking and what it knows — I trust it." (work lane + context rail + honest states.)
- **@5 minutes**: "I'm in flow; it's fast and gets out of the way." (keyboard/palette speed, no friction, no clutter.)
- **@months**: "KRIA feels like *my* intelligent system — it remembers, adapts, and I supervise it effortlessly." (memory legibility, adaptive surfacing, trusted autonomy.)

### 32.3 Delight moments (earned, rare)
- A plan completes → the Core blooms warm, the completed plan settles with a quiet flourish.
- KRIA learns something → a brief "Learned: …" with a warm flick + it appears in Memory.
- A capability is connected → the Constellation gains a new lit node.
- First successful autonomous task → a calm, confident confirmation.
These are the "wow" — tied to meaning, never to idle animation.

### 32.4 Trust storytelling
Every autonomous act tells a three-beat story: **Intent** (what KRIA will do + why) → **Consequence** (risk/effects, one approval surface) → **Evidence** (what it did, verifiable). This narrative is the spine of supervisable autonomy and the core of KRIA's premium-trust feeling.

---

## 33. COMPLETE USER FLOW MAPS

### 33.1 Launch → work (daily)
```
Open → Core "waking→ready" → Converse (Calm)
  → type OR ⌘K OR speak
    → KRIA thinks (Core: thinking, work lane opens)
      → [needs tool?] → tool block runs (evidence)
        → [risk?] → Approval Center (Intent→Consequence→Evidence) → approve/deny
      → answer streams → memory-used shown in rail
  → [remember this?] one-click → Memory
  → done (Core: rest)
```

### 33.2 Voice → reasoning → approval → execution → memory
```
"Hey KRIA / hold PTT" → Core: listening (level)
  → transcribe → thinking (plan-compare block)
  → planning → acting (Core: acting; work blocks per step)
  → high-risk step → Core stills + Approval card → approve(scope)
  → execute → verify (evidence) → learn (Core: warm bloom, memory written)
  → speak result (Core: speaking) → rest
```

### 33.3 Automation authoring (natural language)
```
Automations → "every morning summarize my email"
  → KRIA drafts (work) → review draft (2D canvas + summary)
  → test → approve draft → schedule → confirmation
  → later: run fires → Observatory shows job → result + evidence → notification
```

### 33.4 Memory exploration → correction
```
Memory landing → search → results → select card (Inspector)
  → see confidence/source/conflicts → [wrong?] correct/verify/forget (undo available)
  → open Graph lens → focus node → see neighborhood → materialize a relationship → exit to 2D
```

### 33.5 Capability install (trust)
```
Capabilities → Skills → search → skill card → review (trust + requested effects)
  → install → permission review (Approval Center) → granted
  → Constellation gains node → usable in Converse
```

### 33.6 Failure recovery
```
KRIA hits error → work block: failure (plain language) + Core: recovering
  → recovery options inline (retry / alternative / explain)
  → user picks → resumes → success OR escalates to a decision
```

### 33.7 First launch / onboarding
```
First open → Core greeting (full-attention, brief)
  → name + how-to-talk → voice check (REAL test)
  → optional memory cold-start (consent per source)
  → backend/model pick (simple) → lands in Converse with 3 example intents
```

### 33.8 Model switch / settings change
```
⌘K "switch model to X" → applied (Core brief) → confirmed in status line
⌘K "change theme to light" → applied instantly
Settings Space → search "gpu" → GPU group → toggle (risk badge) → saved (undo via history)
```

---

## 34. IMPORTANCE RANKING → VISUAL WEIGHT

Ranking (from inventory + product logic) that dictates emphasis. **Higher rank = more presence, better placement, first in nav, more polish budget.**

### 34.1 Spaces
1. Converse (Critical) 2. Memory (Critical) 3. Capabilities (High) 4. Automations (High) 5. Observatory (High) 6. Machines (Medium) 7. Settings (Medium, utility).
Dock order + onboarding emphasis follow this.

### 34.2 Global layers
Core (Critical) ≈ Command Palette (Critical) ≈ Approval Center (Critical, when active dominates) > Context Inspector (High) > Notifications (Medium) > Status line (ambient).

### 34.3 Features (visual-weight tiers)
- **Tier 1 (define the OS)**: conversation, live work/reasoning, memory, approvals, voice/Core, command palette.
- **Tier 2 (power)**: automations/n8n, capabilities/tools/skills/models, knowledge graph, fleet.
- **Tier 3 (support)**: analytics, tasks/reminders, integrations, test runner, remote desktop.
- **Tier 4 (rare/dev)**: forensics, ironclad config, readiness-bypass, diagnostics — present but visually quiet/quarantined.

### 34.4 Rule
Visual emphasis must match tier. A Tier-4 control may never out-shout a Tier-1 element. This prevents the current app's problem where dev/telemetry surfaces are as loud as chat.

---

## 35. FUTURE-PROOF DESIGN RULES

### 35.1 Where new things go (decision tree)
- New way to *talk/think with KRIA* → a mode or work-block in **Converse** (not a new Space).
- New *thing KRIA can do* (tool/skill/model/integration) → **Capabilities** (appears in Constellation automatically).
- New *automation/schedule* → **Automations**.
- New *knowledge/analysis* view → a **Memory** lens.
- New *system/telemetry* → an **Observatory** segment.
- New *device/surface KRIA controls* → **Machines**.
- New *preference* → a **Settings** group.
- **Never** create an 8th top-level Space without retiring one. The Dock is capped.

### 35.2 Evolution horizons
- **~2 years**: richer work-blocks, more capabilities in the Constellation, deeper memory reasoning views, multi-thread workflows — all inside the 7 Spaces. Nav unchanged.
- **~5 years**: multi-agent supervision (many KRIA workers) → represented as satellites of the Core + an Observatory "workforce" view, not new top-level nav. Spatial memory/capability lenses mature.
- **~10 years / spatial computing**: the Ink & Aura room and the Core translate to a real spatial environment (headset/ambient) — the Core becomes a presence in the room, Spaces become zones, the graph/constellation become walk-through fields. The 2D productivity surfaces remain flat panels within it. The design is *born* compatible because depth, presence, and lenses are already the model.

### 35.3 Never-violate laws (permanent)
- The Universal Design Laws (§24) are immutable.
- One accent, one Core, one approval surface, one inspector, one token system — forever.
- 3D stays a lens; 2D stays the productivity default.
- Every autonomous action stays legible, approvable, reversible, stoppable.
- Calm-by-default survives every feature addition (new features may not make the home louder).

---

## 36. KRIA NORTH STAR (the dream)

### 36.1 The vision (unconstrained)
You sit down. The room is calm and dark, a single warm-cool intelligence breathing at the edge of your attention. You don't "open an app" — you *arrive somewhere aware*. You speak, or type, or just glance. KRIA already knows the context of your day (its memory), and it thinks *visibly* — you watch it reason, plan, and act, and you can stop or steer it at any instant. Its knowledge is a living field you can walk into; its abilities are a constellation you can survey; its work is a calm stream, not a wall of logs. When it acts on your behalf, it tells you a clear story — what, why, what happened — and you supervise with a nod. Nothing shouts. Everything has purpose. It feels less like software and more like a **composed, trustworthy intelligence sharing your space** — powerful, calm, and unmistakably yours.

### 36.2 The feelings
Presence without intrusion. Power without complexity. Autonomy without loss of control. Intelligence without noise. Beauty without cost.

### 36.3 The bridge (dream → realistic today)
- **Today**: 2D Spaces + Core + palette + 3 on-demand 3D lenses + unified approvals — achievable now, cheap, calm.
- **Next**: the Core gains richer body language and background-cognition presence; memory/capability lenses deepen; multi-thread flow.
- **Later**: multi-agent supervision as Core satellites; adaptive surfacing (KRIA arranges the room to your task).
- **Eventually**: the same model lifts into spatial computing without redesign, because presence + depth + lenses are already the foundation.
Each step is incremental, never a rewrite, and never sacrifices calm/performance for spectacle.

---

## 37. FINAL SELF-REVIEW PASSES (Part B)

- **"What would Apple improve?"** — Even more deference: ensure chrome (Dock/bars) can fully recede in focus mode; make the Core the quietest-loudest element (present but never demanding). Added focus mode (§25.9) + Core honesty rules (§30.4). ✔
- **"What would Linear simplify?"** — Guarantee keyboard-completeness and one-primary-action-per-surface as *laws* (§24), not suggestions. ✔
- **"What would Arc rethink?"** — Spaces + earned delight (§32.3) instead of constant flourish; the room gets out of the way. ✔
- **"What would OpenAI remove?"** — Remove every duplicate/orphan (Part A §6.1), remove spinners in favor of the Core, remove hover-only hidden actions. ✔
- **"What makes KRIA unmistakably KRIA?"** — The **Ink & Aura** language + the **Core as a legible living presence** + **supervisable autonomy storytelling** (Intent→Consequence→Evidence) + **lenses not worlds**. No other product combines calm local-first OS presence with visible, steerable machine cognition. ✔
- **Cognitive-load check** — one focal point per view, ≤3 panels, Calm-by-default, progressive disclosure, capped nav depth. ✔
- **Performance check** — still canvas, on-demand frozen 3D, event-driven motion, Core is the only ambient element, auto-degrade under model load. ✔
- **Trust/agency check** — provenance cue on AI content, one approval surface, reversibility-first, always-present stop, honest uncertainty. ✔
- **Remaining to invent at design stage** (small): exact token values + comps, the Core's precise motion studies (needs visual/motion exploration + a perf spike), 3D lens fidelity tuning, approval-burst density. Everything structural/experiential is now specified.

---

*End of Part B. The Masterplan now defines KRIA's identity, information architecture, Spaces, visual language (Ink & Aura), composition rules, layout blueprints, component kit + hierarchy + importance + reuse, full interaction states, the Core's personality/state grammar, the 3D "how," attention model, flows, ranking→weight mapping, future-proofing, universal laws, and the North Star — the definitive product/UX/visual design bible for KRIA as a premium, calm, local-first AI Operating System.*

---
---

# PART C — DESIGN BIBLE: DESKTOP DEPTH, HOME MASTER-DESIGN, ATTENTION, ADAPTATION & GOVERNANCE

> Final convergence pass. Part A = what. Part B = how it looks/feels. Part C = **desktop-native depth, the definitive Home design, attention governance, adaptive intelligence, delight, anti-patterns, expansion law, and a terminology lock** so the document stops leaving decisions open. Design/UX/visual/IA only.

## 38. SELF-REVIEW PASS 3 (converging critique — what Parts A/B still miss)

Read as a peer would, the remaining real gaps:
1. **Not desktop-native enough.** The doc reads like a single-window web-ish canvas. A desktop AI OS needs: native window behavior, multi-window/detach, multi-monitor, a menu bar / tray presence, always-on-top mini modes, and long-session ergonomics. → §39.
2. **Home is under-designed for its importance.** Converse is the identity; it deserves a full master-design, not a paragraph. → §41.
3. **Attention rules are implied, not codified** (how many things may glow at once, interruption priority, notification tiers). → §42.
4. **"AI OS that evolves with you" is claimed but not designed** (beginner→expert, adaptive surfacing without surprise). → §43.
5. **Delight is mentioned, not catalogued** (which moments, how, restraint bounds). → §44.
6. **No explicit anti-pattern firewall** — the single best defense against re-drifting into the current app's problems. → §45.
7. **Expansion philosophy is thin** for big future modules (Vision Studio, Robotics, Multi-Agent, Marketplace, Enterprise). → §46.
8. **Terminology drift risk** — "panel/rail/lane/inspector/Space" used loosely; needs a locked glossary. → §48.
9. **Long-session / cognitive-fatigue / trust-explainability** underspecified. → §47.
Part C closes all nine, then converges (§49).

**Consistency guardrail for this pass**: every addition must obey the Universal Design Laws (§24), stay Calm/local-first/AI-first, and not add nav depth or ambient cost.

---

## 39. DESKTOP-FIRST DOCTRINE (KRIA is an OS app, not a web page)

KRIA must feel like **premium native desktop software** (macOS apps, Linear, Raycast, Cursor), never a browser tab. This section defines the desktop behaviors Parts A/B assumed but didn't specify.

### 39.1 Window model
- **Primary window**: the full KRIA OS (shell + Spaces). Resizable, remembers size/position/last Space per launch. Has a real minimum size below which it enters a graceful compact layout (§25.9), never breaks.
- **Focus/Zen window state**: a keystroke collapses all chrome (Dock, bars, rails) to Core + canvas only — for deep work/reading. One key restores.
- **Detachable surfaces (multi-window)**: a small, deliberate set may pop out into their own window for multitasking on large/multi-monitor setups:
  - **A conversation thread** (keep chatting while working elsewhere).
  - **The Approval Center** (supervise autonomy on a second monitor).
  - **A 3D lens** (Memory graph / Constellation) as a focused window.
  - **Remote desktop** (its own window is natural).
  - **Observatory "Now"** (a glanceable system monitor window).
  Rule: **detach is opt-in and capped** — not every panel detaches (prevents window sprawl). Detached windows are secondary; the primary window remains the OS.
- **Mini / companion modes**: a compact **"KRIA Mini"** — a small always-available bar/orb (the Core + intent line) that floats for quick ask/voice without opening the full OS. And a **"Now" mini** for at-a-glance system state. Both are optional, dismissible, and cheap.

### 39.2 System-level presence
- **Menu bar / tray**: KRIA lives in the OS menu bar/tray with the Core as its glyph, reflecting state (idle/thinking/acting/needs-you) even when the window is closed. Quick actions from the tray: new ask, toggle voice, pending approvals, pause autonomy.
- **Global summon hotkey**: a system-wide shortcut raises KRIA Mini (Raycast-style) from anywhere — the fastest path to "ask/do."
- **Notifications**: use native OS notifications for out-of-app moments (job done, approval needed) that deep-link into the right Space; in-app they route to the Notification Center (§42).
- **Dock/taskbar badge**: pending-approval / needs-you count.

### 39.3 Multi-monitor & ultra-wide
- **Ultra-wide**: the canvas does not stretch text infinitely — content holds a comfortable measure and the freed space goes to *optional* rails (context, work lane, inspector) shown side-by-side rather than stacked. Never a full-width wall of text.
- **Multi-monitor**: detached surfaces (§39.1) are the multi-monitor story — e.g., Converse on the main display, Approval Center + Observatory on a second. The Core presence appears on the active window.
- **Laptop / small**: rails collapse per §25.9; Dock → compact; one panel at a time; Mini mode especially valuable.

### 39.4 Desktop ergonomics & long sessions
- **Keyboard-first** (Linear/Raycast standard): every action reachable without the mouse; the Command Palette is the spine; shortcuts are consistent and discoverable (see §40).
- **Pointer ergonomics**: primary actions sit where the hand rests (bottom composer, right actions); destructive actions are never adjacent to common ones.
- **Long-session comfort**: dark-default reduces eye strain; calm motion avoids fatigue; no flashing/looping; density stays Calm until invoked; the Core's slow breath is restful, not attention-grabbing. Auto "quiet hours" dim the room and soften the Core late at night (respect ambient light if available).
- **Resume**: KRIA restores the exact Space, thread, and scroll on relaunch (no lost context — fixes the current reload-loses-state problem).
- **No modal traps**: work is never blocked except by a true decision; background work continues while the user reads/does other things (multitasking within the OS).

### 39.5 Why this matters
Desktop-native behaviors are the difference between "a website in a window" and "an operating system." They cost almost nothing in attention/hardware and are decisive for the premium feeling.

---

## 40. NAVIGATION PHILOSOPHY (consolidated & locked)

- **Two ways to move, always**: (1) **spatial** — the Dock (Spaces) for orientation and mouse users; (2) **verbal** — the Command Palette + Intent bar for speed and everything-else. Both always available. Nothing important is reachable *only* one way.
- **Depth cap = 2** (law §24): Dock→Space, or Palette→anything. Sub-navigation inside a Space is flat segments, never nested trees.
- **Discoverability ladder**: Dock (always see the 7 Spaces) → segments (see a Space's lenses) → Palette ("you can also…") → contextual coach on first visit → empty-state teaching → delayed tooltips on icons. A new user is never stranded; an expert is never slowed.
- **Progressive disclosure**: every surface opens Calm; depth (details, technical, dev) is one consistent affordance away and remembered per user preference. KRIA never shows everything at once.
- **Keyboard model** (consistent OS-wide): global summon (system hotkey) · Palette (⌘K-class) · Space switching (numbered/cycle) · new/stop/voice · Esc peels one transient layer · arrows/enter in lists · focus mode toggle. The keyboard map is itself discoverable via the Palette ("keyboard shortcuts").
- **No navigation loops / dead ends** (fixes current issues): every view has a clear exit; back is always meaningful; a failed action always offers a next step.

---

## 41. HOME / CONVERSE — THE DEFINITIVE MASTER-DESIGN

Converse is KRIA's identity and the 90% surface. It gets the deepest design in this bible.

### 41.1 The feeling on open (first 1–5 seconds)
- The room is dark, calm, dimensional. Slightly left-of-center, the **Core breathes** — the first thing the eye meets. The message: *"An intelligence is already here, at rest, ready."*
- The center holds a **calm invitation**, not a blank page and not a busy dashboard: KRIA's presence + a single, quiet prompt to begin (typed or spoken) + at most 3 example intents that teach capability without clutter.
- No panels shout. The Dock is a thin column of quiet glyphs. The intent bar sits ready. Nothing animates except the Core.
- Emotional target: **relief + confidence** — "this is calm, I know what to do, it's powerful but not overwhelming."

### 41.2 Visual composition & hierarchy
- **Focal point**: the conversation column (and the forming answer within it). Everything else is deliberately quieter (lower contrast, quieter surfaces).
- **Reading flow**: Core (presence) → intent/composer (agency) → latest exchange (content) → work lane (only if KRIA is acting) → context rail (only on demand). A clean vertical-then-rightward flow.
- **Breathing room**: the conversation holds a comfortable measure (~60–75 chars); generous vertical rhythm between turns; the composer never crowds the last message. Whitespace is the premium signal — resist filling it.
- **Balance**: asymmetric — Core anchors upper-left, conversation mass centers, optional rails balance right. The layout feels weighted, not centered-and-empty.
- **Density**: opens **Calm** (conversation only). Becomes **Focused** when work runs (work lane fades in). Only reaches **Dense** if the user opens something dense (a table, a graph) — never by default.

### 41.3 The three-lane system (definitive)
```
 Threads[C]      │  CONVERSATION (focal, centered)     │ WORK lane[C/A]   │ CONTEXT rail[C]
 (quiet, ~260)   │  ── the answer forming ──           │ live cognition   │ memory used ·
 collapsible     │  messages + inline result cards     │ (auto-appears)   │ model · tools
                 │  ─────────────────────────────      │ reason·tool·plan │ (on demand)
                 │  [ Composer — sticky bottom ]        │ ·gui·run blocks  │
```
- **Conversation lane (Primary-contextual)**: the star. Highest legibility, most whitespace, warmest reading. Holds user + KRIA turns and *inline* result cards (search/web/image/doc/google) that are rich but subordinate to the reply text.
- **Work lane (Secondary, adaptive)**: KRIA's *visible thinking and doing*. Hidden when idle; **fades in from the right the instant KRIA acts**; a quieter surface so it informs without stealing reading focus. Holds typed **work blocks** (reasoning step · tool call · plan-compare · GUI-cognition · workflow run), each with status, plain-language summary, "details" disclosure, evidence, and an independent **Stop**. Collapsible to a slim "KRIA is working…" spine.
- **Context rail (Contextual, on-demand)**: *why* KRIA answered — the memories/sources it used, the active model, the active tools. One click from any memory chip opens it in the Inspector (round-trip to Memory Space). Off by default; the curious/skeptical user summons it.

### 41.4 Conversation-dominance rule
The reply text always wins. Result cards, work blocks, and rails are visually subordinate (quieter surfaces, lower contrast, tighter type). If the composition ever makes the "machinery" louder than KRIA's answer, it's wrong. This is what keeps Converse a *conversation*, not a dashboard.

### 41.5 The Composer (the most-used object in KRIA)
- **Placement**: sticky bottom of the conversation lane; grows with input to a max then scrolls internally; never covers the last message.
- **Structure (hierarchy)**: input field (primary) · attachment affordance · mode chip (Assistant / Lab / tool-lock) · voice entry · single primary **Send** — which becomes a prominent **Stop** whenever KRIA is working (stop is never more than one action away).
- **States**: empty (quiet placeholder that rotates gentle example intents), typing (calm, no jitter), sending (input clears, Core→thinking), working (Stop), error (inline, recoverable), draft (persists per thread).
- **Voice parity**: the mic is a peer to typing; speaking routes to the same lanes; the Core carries voice state so the composer stays uncluttered.
- **AI-first affordances**: slash/command needs fold into the Palette (no separate slash menu); paste of a file/image is understood; selecting KRIA's text offers "explain / remember / turn into automation."

### 41.6 Empty & first-run states
- **Cold empty (first ever)**: Core-forward greeting + one-line "what can I help with?" + 3 example intents spanning KRIA's range (ask · automate · remember) — teaching by example, not a feature tour.
- **Warm empty (new thread, returning user)**: quieter — Core at rest + composer + optional "continue where you left off" suggestions drawn from recent context (adaptive, never surprising).
- **No results / dead ends**: never blank; always a next step or example.

### 41.7 Interaction density & attention inside Converse
- One focal exchange at a time; older turns recede (subtle de-emphasis with distance) so the *current* thought dominates.
- At most one glowing primary (Send/Stop or a single inline CTA). Work-lane running indicators are quiet pulses, not competing highlights (obeys §42).
- Approvals never render inline as loud modals here — the Core stills, points, and the **Approval Center** surfaces; the conversation stays calm.

### 41.8 Threads (left, quiet, powerful)
- Persistent thread list (not hover-hidden actions — fixes current app). Grouped by recency (Pinned/Today/Yesterday/Earlier). Search across **content**, not just titles. Pin/rename/branch/delete with undo. Selecting restores full thread state.
- "Lab" is a **mode of a thread** (tool-locked testing), not a hidden separate environment.

### 41.9 Emotional arc within Converse
- Ask → Core leans to *thinking* (calm convergence) → work lane quietly reveals the steps → answer streams with composed cadence → memory-used available in the rail → on a meaningful result, a restrained warm bloom. The user *feels* KRIA think, act, and know — legibly and calmly. That felt sequence is KRIA's signature moment and must be protected in every layout decision.

### 41.10 Converse edge cases (designed, not left open)
- **Very long answer / big table / code**: contained with comfortable measure; wide content scrolls within its card, never breaks the lane; a "focus this result" option opens it in the Inspector/detached window.
- **Many simultaneous work blocks**: work lane stacks newest-active on top, older-collapsed below; a single lane-level "Stop all."
- **Rapid-fire messages**: queued visibly; KRIA answers in order; the user is never confused about what's pending.
- **Interrupted/stopped mid-work**: clean, calm "stopped" state with resume/retry; never a frozen spinner.
- **Offline model / degraded**: a quiet, honest banner in the rail + Core in a "limited" state; typing still allowed and queued.
- **2D verdict**: Converse is 2D. Its intelligence is legibility + composition + the Core, not depth.

---

## 42. ATTENTION ECONOMY (governance of the user's focus)

KRIA's scarcest resource is the user's attention. These are enforceable rules.

- **One primary focus per view** (law): exactly one focal point; exactly one glowing primary action. If two things want attention, one is wrong.
- **Highlight budget**: at most **one** accent-glowing element + **one** subtle running-pulse visible at once in a surface. No competing glows.
- **Interruption ladder** (only these may interrupt, in order): (1) **Blocking approval** for a high-risk/irreversible autonomous act — the only true interrupt. (2) **Needs-you** (KRIA is stuck, waiting) — the Core stills + a calm inline prompt, non-blocking. (3) **Notification** (job done, alert) — batched, quiet, never steals focus. (4) **Ambient** (status line, Core) — always non-intrusive. Nothing below tier 1 may take focus by force.
- **Notification tiers**: Critical (approval/needs-you) → Informational (job complete) → Ambient (background cognition). Only Critical may badge/sound; the rest collect calmly in the Notification Center.
- **No decorative motion / no visual noise** (law): every moving/glowing pixel encodes state, progress, or attention. Idle = still (except the Core's breath).
- **Focus recovery**: after any interruption resolves, the room returns to its prior calm and the user's place is preserved (scroll, selection, draft).
- **Urgency hierarchy honesty**: red/risk only for real consequence; KRIA never manufactures urgency to drive engagement (anti-dark-pattern — trust requirement).
- **Batching**: KRIA groups background completions and surfaces them together rather than a stream of pings.
- **Quiet by default**: sound off unless the user opts in; even then, reserved for Critical.

---

## 43. ADAPTIVE INTELLIGENCE (an OS that grows with the user — predictably)

KRIA adapts to expertise **without ever surprising or hiding**. Adaptation is additive and reversible, never disorienting.

### 43.1 Expertise tiers (implicit, never a setting the user must manage)
- **Beginner**: more guidance visible — example intents, coach hints, labeled Dock, "what can KRIA do" prompts, gentler defaults. Nothing dangerous exposed.
- **Intermediate**: coach hints retire as features are used; shortcuts start appearing beside actions the user performs; the Palette surfaces their common commands first.
- **Expert**: chrome recedes; keyboard/Palette lead; frequently-used tools/workflows are promoted to quick actions; rarely-used surfaces demote (but remain reachable via Palette/search).

### 43.2 Adaptive surfacing (rules)
- **Promote by real use**: recently/frequently used tools, workflows, threads, and capabilities surface higher — based on the user's own behavior, explained ("Because you use this often").
- **Demote, never delete**: unused features fade from prominence but are **always** reachable via Palette/search. Nothing the user learned to find ever disappears.
- **Predictability contract** (critical): the *structure never rearranges under the user's hands*. Adaptation changes *suggestions and ordering in clearly-adaptive zones* (quick actions, empty-state suggestions, Palette ranking) — never the position of core navigation or primary actions. Muscle memory is sacred.
- **Explainable & reversible**: any adaptive suggestion says why and can be dismissed/pinned; the user can reset to defaults. KRIA proposes; the user disposes.
- **Personalized quick actions**: a small, user-editable cluster (near the Core/intent bar) that KRIA seeds from behavior and the user can pin/unpin.
- **AI-suggested organization**: KRIA may *offer* to group threads, archive stale automations, or pin a workflow — as a suggestion in the Notification Center, never an automatic rearrangement.

### 43.3 Anti-surprise laws
- No feature moves position without user action.
- No primary action is ever hidden by adaptation.
- Every adaptive change is visible, explained, and reversible.
- Defaults are always one action away.

---

## 44. DELIGHT MOMENTS (elegant, earned, catalogued)

Premium software has memorable moments. KRIA's are calm and meaningful — never childish, never confetti-for-everything.

| Moment | Experience (restrained) |
|---|---|
| **First launch complete** | The Core "awakens" once — a single graceful bloom from dim to ready — then settles. A quiet "I'm ready." Happens once. |
| **First conversation** | The first answer streams with slightly more presence; a subtle acknowledgment that a relationship began. |
| **First voice interaction** | The Core's first listen→speak cycle is a touch more expressive, teaching the voice language. |
| **First learned memory** | A warm-teal flick from the Core toward Memory + "Learned: <thing>." The user *sees* KRIA get smarter. |
| **First automation created** | The new automation settles into place with a quiet flourish; Core gives a confident pulse. |
| **First autonomous execution** | After approval, a composed "done" with visible evidence — building trust, not celebration. |
| **First workflow completed** | The run timeline resolves with a calm success bloom + evidence ready. |
| **Onboarding complete** | The room "opens" — chrome settles into place, Core breathes calm — a threshold-crossing feeling. |
| **Milestones** (100th memory, first month) | A single, tasteful acknowledgment in the Notification Center — opt-in, never intrusive. |

**Delight laws**: at most one delight beat per event; ≤ the motion budget (§24); never blocks; never repeats for the same class of event beyond its "first"; always tied to *meaning* (learning, completing, connecting, trusting). Delight is the reward of *progress*, not decoration.

---

## 45. DESIGN ANTI-PATTERNS (the firewall against drift)

Explicitly forbidden. These exist because the current app already fell into most of them (see inventory). Any of these is a design defect.

**Structure & navigation**
- ❌ Never build another **mega-modal** (the 21-tab settings modal). Big configuration is a Space.
- ❌ Never **duplicate navigation** or create two ways that behave differently.
- ❌ Never exceed **nav depth 2**; never bury a feature 3+ levels deep (the buried-n8n mistake).
- ❌ Never create **duplicate telemetry/dashboard** surfaces; system state lives only in Observatory + Core + status line.
- ❌ Never open a **modal from a modal**; never stack modals.
- ❌ Never create **orphan pages** (built, unreachable) or ship **inert controls** (buttons that do nothing).

**Interaction**
- ❌ Never **hide primary actions** behind hover-only affordances.
- ❌ Never leave an interactive element **without a focus state**.
- ❌ Never present a **dead end** — always a next step/recovery.
- ❌ Never use a generic **spinner** where the Core can carry the wait.
- ❌ Never require the **mouse** for anything essential.

**Visual & motion**
- ❌ Never introduce a **second accent hue** or hardcode a color outside the token system.
- ❌ Never let **risk color decorate** (red = consequence, always).
- ❌ Never add **decorative/ambient animation** (only the Core breathes at idle).
- ❌ Never **blur content** surfaces; blur is for floating layers only.
- ❌ Never break the **elevation/z-order** ladder or invent one-off card/button/dot styles.
- ❌ Never fill **whitespace** just because it exists.

**AI & trust**
- ❌ Never let the **machinery out-shout the answer** in Converse.
- ❌ Never take an **autonomous action** without a legible, approvable, reversible, stoppable path.
- ❌ Never **manufacture urgency** or use dark patterns to drive engagement.
- ❌ Never **rearrange** core layout under the user (adaptation ≠ surprise).
- ❌ Never render **3D as decoration** or trap the user in 3D without a one-action 2D escape.
- ❌ Never hide **AI provenance** — the user must always know what KRIA authored/did.

---

## 46. FUTURE EXPANSION RULES (how KRIA grows to 10× without clutter)

The 7-Space Dock is capped. Big future modules slot into the existing model rather than adding top-level nav.

| Future module | Where it lives | How it integrates | Nav impact |
|---|---|---|---|
| **AI Coding Studio** | a **mode of Converse** + a Capability | code work appears as work-blocks; a focused editor is a detachable window; the Coding "mode" tool-locks the composer | none (mode, not Space) |
| **Vision Studio** (image/video gen) | **Capabilities** (a capability) + results in Converse | generation runs as a work-block with progress; a focused canvas detaches | none |
| **Robotics / device control** | **Machines** (new machine class) | robots are "machines" with health/telemetry/remote; control surfaces reuse the fleet grammar | none |
| **Multi-Agent Teams** | **Observatory "workforce"** + Core satellites | multiple KRIA workers = satellites orbiting the Core; supervise/assign in Observatory; approvals unified | none (extends Observatory + Core) |
| **AI/Skill Marketplace** | **Capabilities → Skills** (expanded) | browse/install/trust reuses existing skill grammar; Constellation gains nodes | none |
| **Cloud Sync** | **Settings → Memory & Privacy** + subtle status in Core | opt-in; sync state shown honestly; local-first remains the default identity | none |
| **Enterprise Fleet / Teams** | **Machines** + a scoped view; org policy in **Settings** | scales the fleet grammar; role/permission surfaces reuse governance patterns | none (may add segments, not Spaces) |
| **Research Lab** | **Memory** (deep reasoning/experiment lenses) or a Converse mode | experiments are memory+reasoning artifacts; heavy analysis = a Memory lens | none |

**Expansion laws**: (1) prefer a **mode** or a **lens** or a **capability** over a new Space. (2) A new Space requires retiring one — the Dock never exceeds ~7. (3) New modules inherit the component kit, the Core state language, the approval/inspector patterns — no new paradigms. (4) The home stays Calm regardless of how much KRIA can do. (5) Everything new is reachable via the Palette from day one.

---

## 47. DEEP-REVIEW RESOLUTIONS (fatigue, trust, accessibility, consistency)

- **Cognitive fatigue / long sessions**: Calm-default density, dark room, restful Core, no looping motion, quiet hours dimming, one-focal-point rule, and progressive disclosure keep hours-long use comfortable. The work lane's *quietness* is deliberate anti-fatigue design.
- **Trust & explainability**: the Intent→Consequence→Evidence story (§32.4) on every autonomous act; visible memory grounding in Converse; honest uncertainty in the Core; provenance cues on AI content; the "why did KRIA answer this / do this" affordance everywhere. Trust is a designed, recurring experience, not a one-time promise.
- **Accessibility (reaffirmed as non-negotiable)**: keyboard-complete (Palette makes it natural), visible focus everywhere, semantic structure + landmarks, labeled controls, live regions for KRIA state, real tables for tabular data, risk never color-only, reduced-motion/high-contrast/font-scale first-class, 3D always has a 2D/keyboard fallback. A11y is a launch gate, not a backlog.
- **Consistency enforcement**: one component kit (§28), one token system, the anti-pattern firewall (§45), and the terminology lock (§48). Reviews check surfaces against these, not against taste.
- **Interaction speed**: everything responds instantly and is interruptible; latency is carried by the Core, not spinners; the Palette makes frequent actions near-zero-friction.

---

## 48. TERMINOLOGY LOCK (glossary — prevents drift & misinterpretation)

One word per concept, used everywhere in design, copy, and specs.

| Term | Definition (locked) |
|---|---|
| **Space** | A top-level context in the Dock (Converse, Memory, Automations, Capabilities, Machines, Observatory, Settings). Not "page/tab/screen." |
| **Dock** | The thin left column of Space glyphs. Primary spatial nav. |
| **Command Palette / Intent bar** | The omni verbal nav+ask+do surface (top-center + summonable). |
| **Core** | KRIA's single living aura/presence and state indicator. Not "orb/avatar/mascot." |
| **Lane** | A vertical region inside Converse: Conversation lane, Work lane, Context rail. |
| **Rail** | A slim collapsible side region (Context rail, thread sidebar). |
| **Inspector** | The single right-side slide-in detail panel (one at a time). Not "drawer/modal." |
| **Work block** | A typed unit of KRIA's visible work in the Work lane (reason/tool/plan/gui/run). |
| **Approval Center** | The single unified surface for all HITL/decisions/consequential approvals. |
| **Notification Center** | The single batched surface for non-blocking informational/ambient notices. |
| **Lens** | An on-demand 3D view of graph/field data (Memory Graph, Capability Constellation). Not "3D page." |
| **Segment** | Flat sub-navigation within a Space. Not "tab tree." |
| **Mini** | A compact floating companion (KRIA Mini, Now mini). |
| **Card / Chip / Row / Badge / StatusDot / Button / Table** | The single kit component per concept (§28) — no synonyms, no variants outside the kit. |
| **Calm / Focused / Dense** | The three density tiers a Space may occupy. |
| **Risk ramp** | The Green→Yellow→Red→Black scale reserved for autonomy/consequence only. |
| **Ink & Aura** | KRIA's visual language (deep ink room + luminous Core + precise content). |

Rule: specs and UI copy use these exact terms. Deviations are review failures.

---

## 49. FINAL CONVERGENCE REVIEW (Part C)

- **Desktop-native now covered?** Windows, detach, mini/tray, multi-monitor, ultra-wide, long-session, resume — yes (§39). KRIA reads as an OS app, not a web page. ✔
- **Home worthy of its importance?** Full master-design: feeling, composition, three lanes, composer, empties, edge cases, emotional arc — yes (§41). ✔
- **Attention protected?** Codified interruption ladder, highlight budget, notification tiers, no-noise laws (§42). ✔
- **Adapts without surprising?** Expertise tiers + promote/demote + predictability contract + reversibility (§43). ✔
- **Delight without childishness?** Catalogued, restrained, meaning-tied (§44). ✔
- **Drift firewall?** Explicit anti-pattern list mapped to the current app's real failures (§45). ✔
- **Scales to 10×?** Modes/lenses/capabilities over new Spaces; Dock capped; kit inherited (§46). ✔
- **Interpretation risk reduced?** Terminology lock + one-component-per-concept + laws (§48/§24/§28). ✔
- **Still Calm / local-first / low-cost / hybrid-2D+selective-3D?** Every Part C addition obeys the laws; nothing adds ambient cost or nav depth. ✔
- **Consistency across new + old sections?** Re-checked: detach set aligns with the "one Inspector / one modal" laws (detached windows are secondary surfaces, not extra modals); Mini aligns with the Core-as-presence model; adaptive surfacing respects the anti-surprise + attention laws; expansion respects depth cap. No new contradictions introduced. ✔

**Remaining for the visual-design stage only** (not decisions, executions): exact token values + comps; the Core's precise motion/visual studies (+ a performance spike); 3D lens fidelity tuning; the exact detach-set final list; icon set selection. Everything structural, experiential, behavioral, and governing is now specified.

---

*End of Part C. The Masterplan (Parts A + B + C) now defines KRIA's identity, IA, Spaces, desktop-native window/system behavior, the definitive Home/Converse design, visual language (Ink & Aura), composition + layout blueprints, the component kit/hierarchy/importance/reuse, full interaction states, the Core's living personality, the 3D "how," the attention economy, adaptive intelligence, delight, the anti-pattern firewall, future-expansion law, terminology lock, and the North Star — the authoritative, drift-resistant design bible for KRIA as a premium, calm, local-first, AI-native desktop operating system.*

---
---

# PART D — DESKTOP EXPERIENCE, WINDOW MODES & PLATFORM DOCTRINE (Core Principle)

> This is not a feature — it is a **core design principle** ranked beside Calm, AI-first, and Local-first. KRIA is a **native desktop AI Operating System**, primarily targeting **Ubuntu/Linux**, then Windows, then macOS. Part D deepens and supersedes §39 where they overlap. Design/UX only — no implementation, no OS-specific code.

## 50. DESKTOP-FIRST AS A CORE PRINCIPLE

### 50.1 The principle
Every KRIA decision prioritizes **desktop ergonomics over web conventions**. KRIA must feel like it belongs beside VS Code, Cursor, Blender, IntelliJ, and Unreal — not beside browser tabs. The test: *if a user alt-tabs from Cursor to KRIA, the interaction quality, polish, keyboard fluency, and window behavior must feel equally native and intentional.*

### 50.2 What "desktop-first" concretely means for KRIA
- **Windowed, not paged.** KRIA is a window that behaves like professional software (remembers geometry, respects the WM, resizes gracefully into *modes*, not into "mobile breakpoints").
- **Keyboard-first, mouse-excellent.** Every action has a shortcut; the Palette is the spine; pointer targets are generous and ergonomically placed.
- **Multitasking-native.** Background work continues while the user does other things; surfaces can detach to other monitors; nothing blocks except a true decision.
- **Long-session comfort** is a first-class goal (dark room, calm Core, quiet hours, no looping motion).
- **Never responsive-web behavior.** KRIA never "reflows like a website." It transitions between **deliberate window modes** (§51), each intentionally composed. Shrinking the window changes *mode*, not just scale.

### 50.3 Platform priority & the platform-neutral design language
- **Primary: Ubuntu/Linux.** Secondary: Windows. Tertiary: macOS.
- KRIA defines its **own platform-neutral desktop language** that feels native everywhere and never depends on an OS-specific pattern:
  - KRIA carries its **own in-app navigation and controls** (Dock, Palette, Core, in-app window actions) — it does **not** depend on a macOS-style global top menu bar, nor on a Windows-style ribbon, nor assume a specific system tray exists.
  - KRIA **respects** the host window manager's decorations and controls rather than replacing or faking them; its own chrome sits *inside* the content area.
  - System integrations (tray glyph, native notifications, global hotkey) are treated as **enhancements with in-app fallbacks**, never as the only path — because their availability varies across Linux desktops (see §54).
- **Design rule**: if a pattern only feels right on Windows or macOS, it is rejected in favor of a neutral one that also feels right on GNOME/KDE.

### 50.4 Elevation in the principle hierarchy
Add "**Desktop-native**" to KRIA's core pillars (§1.5 originally: Presence, Continuity, Supervisable autonomy) → now four pillars: **Presence · Continuity · Supervisable autonomy · Desktop-native**. All four are co-equal and immutable.

---

## 51. THE THREE WINDOW MODES

KRIA has three intentional modes. Each is a **distinct composition**, not a scaled version of another. Transitions between modes are smooth and explained (a brief, eased spatial change ≤ the motion budget), never abrupt.

### 51.1 Mode overview
| Mode | Footprint | Intent | Chrome | Default? |
|---|---|---|---|---|
| **Compact Workspace** | ~¼ screen (corner/side) | quick ask/command/voice, monitoring, small tasks | minimal; secondary panels auto-collapse | no |
| **Standard Desktop** | maximized/normal window | the main working mode | full KRIA chrome, respects OS chrome | **yes** |
| **Immersive Focus** | entire display | deep work, big lenses, long sessions | KRIA owns the display; OS chrome hidden | no |

### 51.2 Compact Workspace Mode (~¼ screen)
- **Purpose**: a persistent, intelligent corner companion — quick conversations, quick commands, voice, notifications, small automations, and glanceable monitoring of ongoing work. The everyday "talk to KRIA while I work in another app" surface.
- **Feel**: **intentional, not compressed.** It is a *curated* subset, composed for the small footprint — never the Standard layout shrunk.
- **Composition**: Core (top, presence + state) → intent/composer (center, dominant) → a single adaptive stream below that shows *the one most important thing right now*: the live answer, or the running work summary, or a pending approval, or the latest notification. One thing at a time, chosen by priority.
- **What collapses (auto)**: Dock → a slim glyph strip or hides behind the Core/palette; thread sidebar → hidden (Palette + recent); Work lane → a one-line "KRIA is working… (tap to expand)" spine; Context rail → hidden (available via Inspector-as-overlay); segments → hidden.
- **Priority logic**: Compact **prioritizes, it does not shrink**. If an approval is pending, it surfaces (the one true interrupt). Otherwise: active answer > running work > notifications > idle composer.
- **Interactions**: type/voice primary; Palette for anything else; tapping the work spine expands it as a temporary overlay; approvals appear as a compact but complete Approval card (never truncated — a decision must stay legible).
- **Relationship to Mini**: "KRIA Mini" (§39.1) is the *smallest* presence (Core + one line). Compact Mode is a step up — a usable quarter-screen workspace. Mini → Compact → Standard → Immersive is one continuous ladder.
- **3D**: no 3D in Compact (lenses require room); a lens invoked in Compact offers to open in Standard/Immersive or a detached window.

### 51.3 Standard Desktop Mode (default)
- **Purpose**: the primary working mode for most users, most of the time. Behaves exactly like professional desktop apps (VS Code/IntelliJ/Cursor/Blender).
- **Respects the OS**: the window sits within the desktop workspace and **never hides** the host Dock/taskbar/top panel/window-manager controls/desktop-environment UI. KRIA is a good desktop citizen here.
- **Composition**: the full shell (§25.1) — Presence bar, Dock, Space canvas, optional Inspector, status line. All three Converse lanes available; rails collapsible; density Calm-by-default, Focused/Dense on task.
- **This is the default** on first launch and the mode all Space blueprints (§25) are authored against.

### 51.4 Immersive Focus Mode (full display)
- **Purpose**: distraction-free deep work — long conversations, deep research, coding, planning, memory exploration, large knowledge graphs, complex workflows, multi-agent monitoring.
- **Feel**: KRIA **owns the entire display**; the outside world (OS Dock/taskbar/top panel/window decorations) recedes. Even KRIA's own chrome thins: Dock auto-hides to an edge reveal, bars minimize, headers soften where appropriate. The Core and the content remain.
- **Composition**: canvas-maximal. The active Space gets the whole display; rails become edge-reveal or Inspector-overlay; the Palette (summonable) becomes the primary nav so the Dock can hide. 3D lenses get their full, calm stage here.
- **Transition**: entering/leaving Focus is a smooth, intentional "the room opens / the room returns" motion — the user always knows they crossed a threshold, and exit is always one obvious key (Esc-class) so they never feel trapped.
- **Guardrail**: even in Immersive, **approvals and the global Stop are always reachable** (safety > immersion) — the Core surfaces them; immersion never hides consequence.

### 51.5 Mode ladder & continuity
Mini → Compact → Standard → Immersive is a single continuum of *presence and space*, not four separate UIs. Switching modes preserves the current Space, thread, selection, scroll, and draft. The user's context is sacred across every transition.

---

## 52. ADAPTIVE LAYOUT MATRIX (every Space × every mode)

For each Space: how it composes in Compact / Standard / Immersive. Legend: **show · collapse · contextual(on-demand) · hidden · docked · floating**.

### 52.1 Global elements across modes
| Element | Compact | Standard | Immersive |
|---|---|---|---|
| Core | show (top, prominent) | show (presence bar) | show (floats, primary nav anchor) |
| Command Palette | primary nav (summon) | show + summon | primary nav (summon) |
| Dock | collapsed glyph strip / hidden | show | auto-hide (edge reveal) |
| Presence bar | condensed (Core+intent only) | full | thinned |
| Inspector | overlay (full-width sheet) | slide-in rail | slide-in / overlay |
| Approvals | compact full card (never hidden) | Approval Center | surfaces over immersion |
| Notifications | one-at-a-time, quiet | Notification Center | edge, batched |
| Status line | one line (essential) | show | thinned / on-reveal |

### 52.2 Converse
- **Compact**: Conversation lane only + composer; Work lane → one-line spine (expand as overlay); Context rail → hidden (Inspector overlay); threads → Palette/recent. Focal = the answer.
- **Standard**: three lanes (Conversation focal, Work adaptive, Context on-demand); threads sidebar collapsible.
- **Immersive**: Conversation centered with maximal breathing room; Work lane docks right (fuller); Context rail available; Dock hidden — pure "think with KRIA." Best for long conversations/research.

### 52.3 Memory
- **Compact**: quick memory *search + result peek* only; graph unavailable (offer to open larger); detail = overlay. Read-only-ish quick lookups.
- **Standard**: landing + lens segments; Explorer list + Inspector; graph opens as a focused center.
- **Immersive**: the **Knowledge Graph lens** gets the whole display — the flagship spatial experience; controls edge-docked; Inspector overlay; 2D fallback still one action away.

### 52.4 Automations
- **Compact**: "run" only — ask-KRIA-to-pick + a short ready-to-run list + run status; builder/history hidden (offer to open larger). Great for firing a workflow mid-work.
- **Standard**: full segments (Run/Build/Schedule/History); builder canvas + node inspector.
- **Immersive**: the **2D node builder** gets full room for complex workflows; palette + inspector edge-docked.

### 52.5 Capabilities
- **Compact**: search + quick "what can you do about X" + run/approve; constellation hidden.
- **Standard**: segments + rows + Inspector.
- **Immersive**: the **Constellation lens** full-display; filters edge-docked.

### 52.6 Machines
- **Compact**: glanceable fleet health + this-device + pending alerts; terminal/remote hidden (open larger). Monitoring role.
- **Standard**: matrix + terminal + alerts.
- **Immersive**: remote-desktop canvas full-display (natural); or the fleet matrix as a monitoring wall.

### 52.7 Observatory
- **Compact**: the **"Now" mini** — system pulse + running jobs + Core state. The ideal always-visible monitor.
- **Standard**: full segments (Now/Jobs/Analytics/Forensics/Diagnostics).
- **Immersive**: a full monitoring wall (multi-agent monitoring, big dashboards) — for supervising autonomy at scale.

### 52.8 Settings
- **Compact**: search-only ("change what?") + the single result; groups collapsed. Quick toggles.
- **Standard**: group rail + sections.
- **Immersive**: rarely used; behaves as Standard centered (settings don't need immersion).

### 52.9 Universal degradation rule
Every Space **degrades by curation, not compression**: in smaller modes it *drops* secondary regions and elevates the one primary task, rather than shrinking everything until it's unusable. A Space must always be genuinely usable for its *core* action in Compact, fully capable in Standard, and expansive in Immersive.

---

## 53. MULTI-MONITOR EXPERIENCE

KRIA rewards professional workstation setups without requiring them. The mechanism is **detachable surfaces** (§39.1), now specified for multi-display:

- **Dedicated Knowledge Graph** — the Memory lens as its own window on a second monitor; explore structure while working in Converse on the main.
- **Persistent Observatory / "Now"** — a always-visible system + jobs dashboard on a side monitor for supervising KRIA.
- **Detached Voice Console** — the Core + voice state + live transcript as a small dedicated window (great for hands-free/meeting/coding modes).
- **Floating Agent Monitor** — for multi-agent futures, a window showing worker satellites + their tasks + approvals.
- **Separate Knowledge Explorer** — a Memory Explorer window for research beside a document/editor.
- **Independent AI execution window** — a detached Work lane / run monitor to watch long automations execute.

**Rules**: (1) the **primary window remains the OS**; detached surfaces are secondary and clearly subordinate. (2) The **Core presence** appears on the active window (and as a subtle anchor on detached ones) so the user always knows KRIA's state on whichever screen they're looking at. (3) **Approvals mirror** to wherever attention is — a detached-window setup must never let a needed approval hide on an unfocused screen (surfaces on the active window + badges the tray). (4) Detach is **opt-in and capped** to the set above — no arbitrary panel-sprawl. (5) Single-monitor users lose nothing: every detachable surface is fully reachable in-window via Spaces.

---

## 54. LINUX DESKTOP CONSIDERATIONS (primary platform)

Because Ubuntu/Linux is primary, the interaction model must feel natural across GNOME, KDE Plasma, and both Wayland and X11 — **without depending on behaviors that vary between them.** Design-level guidance (no implementation):

### 54.1 Window decorations & controls
- Linux desktops disagree on window controls (GNOME often close-only, KDE typically min/max/close, positions vary, client-side vs server-side decorations differ). **KRIA must not fake a fixed titlebar with hardcoded control positions.** It **respects the host's window decorations** and keeps its own chrome (Dock/Palette/Core/mode switch) *inside* the content area, so KRIA looks correct whether the WM draws decorations or not.
- KRIA provides its **own in-app window-mode switch** (Compact/Standard/Immersive) rather than relying on OS-specific maximize/fullscreen affordances that behave differently per DE.

### 54.2 Global menu bar assumption — rejected
- KRIA never depends on a macOS-style global top menu bar or a Windows-style menu ribbon. All navigation lives in-app (Dock + Palette). This is already KRIA's model — reaffirmed as the Linux-safe choice.

### 54.3 System tray / indicators
- Tray/indicator support is inconsistent on Linux (varies by GNOME extensions vs KDE). The tray glyph + quick actions are an **enhancement with a full in-app fallback**: everything reachable from the tray is also reachable inside KRIA (pending approvals, toggle voice, pause autonomy, new ask). KRIA never *requires* a working tray.

### 54.4 Global hotkey (summon)
- A system-wide summon hotkey may be restricted (notably under Wayland). Design a **graceful fallback**: if the global hotkey isn't available, the in-app Palette + the tray + KRIA Mini still provide fast summon. The "summon from anywhere" promise degrades to "summon instantly once KRIA is focused," never breaks.

### 54.5 Always-on-top / floating windows (Mini, Compact, detached)
- Wayland restricts always-on-top and precise window positioning. KRIA's Mini/Compact/detached windows are designed to be **useful even if the WM controls their stacking/placement** — the user can position them via their WM; KRIA doesn't assume it can force placement. The design communicates state well regardless of stacking.

### 54.6 Remote desktop / screen capture (Machines)
- Screen capture + input control differ significantly (Wayland's portal-based capture + input constraints vs X11's freer model). At the design level: the remote-desktop experience must **degrade honestly** — clearly communicate capability/permission state, never present controls that silently don't work, and always show what KRIA can and cannot do on the current session. (KRIA's current app already reflects this reality; the redesign must preserve honest capability signaling.)

### 54.7 Theming & fonts across DEs
- KRIA ships its **own complete visual language (Ink & Aura)** and does **not** inherit unpredictable GTK/Qt theme variables that would fracture its look across GNOME/KDE. It looks identical everywhere (with dark/light as KRIA's own choice, optionally following the OS preference as an enhancement). Fonts are KRIA's own, bundled, so typography is consistent regardless of system fonts.

### 54.8 Fractional scaling / HiDPI / mixed-DPI
- Linux fractional scaling and mixed-DPI multi-monitor are common pain points. KRIA's layout is built on a relative spacing/type scale (§22.5) and mode-based composition (not pixel breakpoints), so it stays crisp and correctly proportioned across scales and when a window moves between differently-scaled monitors.

### 54.9 Linux-usability review of existing sections (findings + fixes)
- **§39.1 detach / always-on-top** → reframed: detached/Mini windows must be useful even if the WM governs stacking (fixed above).
- **§39.2 global hotkey / tray** → reframed as enhancements-with-fallback (fixed above).
- **Any implied macOS "traffic light" or Windows "snap" reliance** → none should exist; window-mode switching is KRIA's own in-app control, WM snapping is a bonus, not required.
- **Immersive Focus hiding OS chrome** → must use the standard fullscreen concept the DE provides and always offer a clear, DE-agnostic exit (Esc-class), since fullscreen escape affordances differ across GNOME/KDE.
- **Result**: no section now assumes a Windows/macOS-only pattern; all desktop behaviors are platform-neutral with Linux as the design baseline.

---

## 55. DESKTOP PARITY BAR & WEB-ISM PURGE

### 55.1 The parity bar (what "premium desktop" requires)
When switching between Cursor/VS Code/Blender/Unreal and KRIA, these must feel equal:
- Instant, interruptible response (no web-latency feel).
- Keyboard fluency (everything reachable, discoverable via Palette).
- Real window behavior (geometry memory, modes, multi-monitor, resume).
- Native-feeling density control (Calm↔Dense) without "responsive reflow."
- Professional restraint (no marketing-site motion, no web-card shadows-for-decoration).

### 55.2 Web-isms explicitly purged (design review of the whole document)
- ❌ No "responsive breakpoints" language → replaced by **window modes** (§51) + curated degradation (§52.9).
- ❌ No full-bleed hero/marketing composition → Home is a *workspace*, not a landing page (§41).
- ❌ No infinite content stretch on ultra-wide → measure held, space → optional rails (§39.3/§53).
- ❌ No hamburger-menu / mobile-nav thinking on desktop → Dock + Palette.
- ❌ No modal-heavy web flow → Inspector + inline + one-modal law (§8, §24).
- ❌ No decorative web animation → motion budget + Core-only ambient (§24, §42).
- ❌ No dependence on browser-tab metaphors → Spaces + threads, not tabs-as-navigation.
The mobile companion (Part A §7.8) remains the *only* surface that is legitimately responsive/touch — and it is explicitly a separate companion, not the desktop OS.

---

## 56. CONVERGENCE REVIEW (Part D)

- **Desktop-first elevated to a core pillar?** Yes — now four co-equal pillars (§50.4). ✔
- **Three window modes fully designed, not scaled?** Compact (curated ¼-screen), Standard (default, OS-respecting), Immersive (owns display) — each a distinct composition with smooth transitions (§51). ✔
- **Every Space × every mode specified?** Adaptive matrix + universal degrade-by-curation rule (§52). ✔
- **Multi-monitor?** Detachable surfaces, Core presence per window, approval mirroring, capped, single-monitor loses nothing (§53). ✔
- **Linux-primary, platform-neutral?** In-app nav (no global menu bar), respect WM decorations, tray/hotkey/always-on-top as enhancements-with-fallback, honest capture degradation, own theming/fonts, HiDPI-safe; existing sections reviewed and reframed (§54). ✔
- **Feels like pro desktop software, not web?** Parity bar + explicit web-ism purge (§55). ✔
- **Consistency with prior laws?** Modes obey the Universal Design Laws (one focal point, one modal, calm-default, motion budget); Compact's "prioritize not shrink" = the attention economy applied to space; detach set aligns with the one-Inspector/one-modal rules; nothing adds nav depth or ambient cost. No contradictions. ✔
- **Remaining for visual stage only**: exact mode dimensions/thresholds, transition motion studies, the final detachable-surface list, per-DE fullscreen exit affordance choices. All executions, not open design decisions.

---

*End of Part D. KRIA is now specified as a true native desktop AI Operating System — Linux-primary and platform-neutral — with three intentional window modes, a complete per-Space adaptive layout matrix, a professional multi-monitor model, and a Linux-desktop-safe interaction language, sitting confidently beside Cursor, VS Code, Blender, and Unreal. The Masterplan (Parts A–D) is the authoritative, drift-resistant design bible for KRIA's premium, calm, local-first, AI-native desktop experience.*
