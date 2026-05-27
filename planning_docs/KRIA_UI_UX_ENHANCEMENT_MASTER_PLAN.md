# KRIA UI/UX Enhancement Master Plan

**Document status:** Production refinement planning
**Scope:** Desktop UI/UX audit, frontend refinement planning, visual consistency, accessibility, responsiveness, interaction quality, and production polish
**Implementation status:** Planning only
**Default theme target:** Light mode
**Primary goal:** Make the existing KRIA desktop app feel compact, clean, professional, responsive, stable, interactive, and trustworthy without a destructive redesign.

---

# Executive Summary

KRIA already has a functional desktop UI with broad product coverage:

- chat and prompt execution,
- session sidebar,
- model/provider settings,
- GUI automation status,
- HITL approval surfaces,
- dashboards and analytics,
- voice overlay,
- fleet/device views,
- test and evaluation panels,
- skill marketplace,
- setup and provisioning surfaces.

The next UI/UX phase should not be a full redesign. The production need is refinement: remove inconsistency, reduce friction, improve information hierarchy, stabilize light/dark theming, strengthen accessibility, and make complex AI/runtime state understandable without hiding power-user detail.

The current UI appears to have grown feature-by-feature. That is normal for a fast-moving desktop assistant, but production quality now requires a stronger visual and component system.

Key current risks observed in the frontend:

| Area | Production Risk |
|---|---|
| Theme boot | `ui/index.html` defaults to dark while `appStore.resolveInitialTheme()` defaults to light. This can create first-paint mismatch and violates the desired light-mode default. |
| Styling consistency | CSS tokens exist, but there are multiple token naming patterns and many inline styles across components. |
| Settings complexity | `SettingsModal.tsx` is very large and covers many unrelated settings domains in one component. |
| Chat rendering | Markdown, code, tool calls, images, and long outputs are all handled in one high-value surface that needs careful performance and accessibility hardening. |
| Accessibility | Focus states exist in CSS, but several controls rely on hover visibility or custom controls that need stronger keyboard and screen-reader behavior. |
| Visual density | Some views are compact and operational; others use dashboard/card patterns and inline styling that can feel inconsistent or visually noisy. |
| Error/recovery UX | KRIA has many runtime states, but user-facing feedback needs consistent severity, recovery, and partial-completion language. |
| Frontend maintainability | Large files and ad hoc styles increase risk for regressions during future feature work. |

The recommended direction is:

```text
preserve app shell
preserve existing workflows
make light mode first-class and default
unify design tokens
extract shared UI primitives
refine chat, settings, model selector, workflow feedback
improve keyboard/accessibility behavior
add responsive desktop constraints
polish micro-interactions
measure performance and rendering stability
```

This is a production-grade refinement program, not a random redesign exercise.

## Risk Review Disposition

The following review items are correct and impactful. None should be rejected. They are integrated as production guardrails, not as immediate implementation mandates.

| # | Review Item | Disposition | Integrated As |
|---:|---|---|---|
| 1 | Settings architecture still too centralized | Accepted | Settings Navigation Layers |
| 2 | Dashboard sprawl risk remains high | Accepted | Operational Surface Priority Doctrine |
| 3 | No strict primary workflow preservation doctrine | Accepted | Chat-first canonical UX anchor |
| 4 | Design-token system may become overengineered | Accepted | Token Minimalism Doctrine |
| 5 | Progressive disclosure can become hidden UX | Accepted | Discoverability Safeguards |
| 6 | Too many modals already exist | Accepted | Modal Governance Rules |
| 7 | Chat surface risks feature overload | Accepted | Message Surface Hierarchy |
| 8 | Accessibility plan lacks automation requirements | Accepted | Automated accessibility checks |
| 9 | No explicit keyboard-first doctrine | Accepted | Keyboard-First Productivity Layer |
| 10 | Light mode direction is slightly vague | Accepted | Light Mode Visual Identity |
| 11 | No typography doctrine | Accepted | Typography and hierarchy rules |
| 12 | No explicit motion philosophy | Accepted | Motion Doctrine |
| 13 | AI futuristic styling creep risk | Accepted | Visual Restraint Doctrine |
| 14 | No frontend state-boundary governance | Accepted | Frontend State Ownership Model |
| 15 | No explicit z-index/layering governance | Accepted | Layering Hierarchy Specification |
| 16 | No interaction latency targets | Accepted | UI responsiveness budgets |
| 17 | Performance lacks observability | Accepted | Frontend telemetry plan |
| 18 | Error philosophy lacks severity model | Accepted | User-Facing Severity Taxonomy |
| 19 | Markdown strategy needs hard caps | Accepted | Rendering caps and deferred rendering |
| 20 | No mobile/non-desktop rejection doctrine | Accepted | Desktop-first, not mobile-primary doctrine |
| 21 | Too much reliance on modals | Accepted | Docked/persistent surfaces for long workflows |
| 22 | No visual information hierarchy doctrine | Accepted | Primary/secondary/tertiary hierarchy |
| 23 | "Beautiful" is subjective | Accepted | Concrete aesthetic constraints |
| 24 | No UX consistency enforcement | Accepted | Component review checklist |
| 25 | Shared primitives may become pseudo-framework | Accepted | Primitive Simplicity Rule |
| 26 | No explicit empty-space governance | Accepted | Compact-density layout constraints |
| 27 | Workflow visibility underspecified | Accepted | Workflow information prioritization |
| 28 | No shell-layout doctrine | Accepted | Locked shell structure principles |
| 29 | No onboarding philosophy | Accepted | Lightweight onboarding/help strategy |
| 30 | No visual regression governance | Accepted | Screenshot diff testing strategy |

---

# Current UI/UX State Analysis

## Current Frontend Shape

The frontend is a SolidJS desktop app loaded through Vite/Tauri-style APIs.

Important current files:

| File | Role |
|---|---|
| `ui/index.html` | Initial HTML, first theme assignment, root mount. |
| `ui/src/index.tsx` | Frontend entry point. |
| `ui/src/App.tsx` | Main app shell, routing, modal orchestration, dashboard route selection. |
| `ui/src/stores/app.ts` | Central frontend state, persisted theme/environment/session state, IPC event batching. |
| `ui/src/components/ChatView.tsx` | Main chat surface, attachments, prompt submit, empty state, thinking state. |
| `ui/src/components/MessageBubble.tsx` | Markdown/tool rendering, code blocks, images, tool evidence surfaces. |
| `ui/src/components/SettingsModal.tsx` | Main settings modal covering providers, voice, UI, services, GUI automation, hardware, integrations, and more. |
| `ui/src/components/ProviderSettings.tsx` | Provider/model/runtime switching surface. |
| `ui/src/components/HitlModal.tsx` | Human approval/review workflow surface. |
| `ui/src/components/DecisionActionCenter.tsx` | Decision/action review surface. |
| `ui/src/components/GuiWorkflowViewer.tsx` | GUI workflow progress and state surface. |
| `ui/src/components/VoiceOverlay.tsx` | Voice interaction overlay. |
| `ui/src/components/SessionSidebar.tsx` | Session navigation and session actions. |
| `ui/src/components/DeviceMatrix.tsx` | Fleet/device target status surface. |
| `ui/src/components/TestRunnerDashboard.tsx` | Test/eval execution dashboard. |
| `ui/src/components/AnalyticsDashboard.tsx` | Runtime/eval analytics dashboard. |
| `ui/src/styles/*.css` | Global theme, layout, chat, provider, device, and wizard styling. |

## Current Routes And Surfaces

The current app route model is intentionally simple:

| Route | Surface |
|---|---|
| `home` | Assistant or prompt lab workspace. |
| `dashboard` | Runtime overview, operations, forensics, n8n, analytics, tests. |
| `vm-management` | Fleet/device target management. |
| `settings` | Settings modal route integration. |

This route model should be preserved unless a specific route creates measurable workflow friction.

## Canonical UX Anchor

The canonical KRIA UX anchor is:

```text
chat-first assistant workflow
  + visible workflow status
  + recoverable actions
  + advanced diagnostics on demand
```

This means the app shell, dashboards, settings, evals, workflow panels, and integrations must support the assistant workflow. They must not turn KRIA into a generic multi-panel enterprise suite where the primary user path becomes unclear.

## Shell Layout Doctrine

Preserve this shell structure unless user research or production defects prove it harmful:

```text
left session/navigation rail
  -> top operational/status bar
  -> primary assistant workspace
  -> docked workflow/status regions when needed
  -> modals only for short, blocking decisions
```

Rules:

- Chat and prompt execution remain the center of gravity.
- Dashboards are support surfaces, not the default mental model.
- Settings are configuration surfaces, not workflow destinations.
- Long-running workflows should prefer docked or persistent panels over blocking modals.
- The shell should not become mobile-first or card-dashboard-first.

## Current Strengths

- The app already has a real desktop product shape, not just a chat page.
- Core assistant, prompt lab, provider, voice, GUI automation, and eval surfaces are present.
- There is already a theme token layer in CSS.
- IPC event batching exists in `appStore`, which is important for smooth desktop behavior under high-frequency backend events.
- Several surfaces already expose status and progress rather than hiding runtime behavior.
- Settings are extensive enough for power users.
- The UI has test selectors and component tests that should be preserved or intentionally migrated.

## Current Weaknesses

- The UI is feature-rich but not yet design-system-stable.
- Some files are too large for safe production iteration.
- Inline styles and hardcoded colors reduce theme consistency.
- Light mode is intended as default in app state, but first paint currently defaults to dark.
- Settings are powerful but can overwhelm layman users.
- Dashboards can trend toward enterprise-panel clutter if not governed.
- Chat is a critical surface and needs stronger large-content, markdown, copy, focus, and scroll behavior.
- Several interactions need clearer loading, recovery, retry, and partial-completion states.

---

# Critical UI Problems

## 1. Theme Default Mismatch

Observed behavior:

```text
ui/index.html:
  saved === "light" ? "light" : "dark"

ui/src/stores/app.ts:
  saved === "dark" ? "dark" : "light"
```

Impact:

- First paint can default to dark even when the app state defaults to light.
- Users may see a flash or mismatch during startup.
- The product requirement says light mode is the default.

Planning decision:

- Fix the boot theme contract in a future implementation phase.
- The first paint script, app store default, settings UI, and persisted value must agree.
- Light mode must be production-grade, not a washed-out override.

## 2. Styling Fragmentation

Observed behavior:

- CSS variables exist across `base.css`, `theme-shell.css`, `messages.css`, `providers.css`, and other files.
- Token naming includes multiple families such as `--bg-*`, `--surface-*`, `--border`, `--border-color`, `--accent`, provider-specific tokens, and fallback hardcoded colors.
- Many components use inline styles, especially dashboards, plan visualization, settings subsections, analytics, marketplace, and quarantine views.

Impact:

- Dark/light parity is difficult to maintain.
- Visual drift increases with each new feature.
- Accessibility contrast auditing becomes harder.
- UI changes carry higher regression risk.

Planning decision:

- Stabilize a shared token system before broad visual polish.
- Move repeated inline styles into component classes or shared primitives incrementally.
- Do not rewrite all CSS at once.

## 3. Settings Modal Scale

Observed behavior:

- `SettingsModal.tsx` is a large component with many settings domains:
  - LLM,
  - voice,
  - safety,
  - UI,
  - assistant,
  - labs,
  - search,
  - services,
  - Telegram,
  - automation,
  - GUI automation,
  - hardware,
  - knowledge,
  - Google,
  - Colab,
  - Ironclad,
  - marketplace.

Impact:

- Hard to maintain.
- Hard to polish consistently.
- Hard to create beginner/power-user layering.
- Changes in one settings area can regress another.

Planning decision:

- Keep the settings experience, but split internally into section components and shared controls.
- Add progressive disclosure and search improvements.
- Avoid turning settings into many unrelated full pages unless modal usability breaks.

## 4. Chat Surface Complexity

Observed behavior:

- `ChatView.tsx` handles input, attachments, slash commands, thinking states, prompt submit, drag/drop, paste, and auto-scroll.
- `MessageBubble.tsx` handles markdown, code, syntax highlighting, tool calls, images, trusted/untrusted surfaces, web results, and local images.

Impact:

- Chat is the highest-value user surface and has the highest regression risk.
- Large markdown/log output can hurt performance.
- Hover-only copy controls can hurt discoverability and keyboard accessibility.
- Injected inline `onclick` inside rendered code block controls is a maintainability and security smell even with sanitization.

Planning decision:

- Prioritize chat refinement early.
- Preserve the current message model.
- Improve rendering, copy interactions, scroll behavior, and accessibility without changing the core chat workflow.

## 5. Dashboard Visual Noise Risk

Observed behavior:

- Runtime status, analytics, tests, forensics, n8n, executive state, device matrix, and plan surfaces are all present.
- Many dashboard-like components use inline styles and local visual decisions.

Impact:

- Power users benefit from detail.
- Layman users can feel overwhelmed.
- The app can feel like a collection of panels rather than one product.

Planning decision:

- Use layered detail:
  - summary first,
  - expandable detail,
  - raw logs only on demand.
- Use one status language across dashboards.
- Do not remove power-user surfaces.

## 6. Modal Fatigue Risk

Observed behavior:

- KRIA has settings, HITL, permission, add/edit target, voice overlay, shortcut overlay, and workflow-adjacent surfaces.
- Some of these are blocking by nature, while others are informational or long-running.

Impact:

- Users can lose context when too many surfaces interrupt the primary workflow.
- Modal stacking can create focus and z-index bugs.
- Long-running operations inside modals can feel trapped and fragile.

Planning decision:

- Add modal governance and layering hierarchy.
- Use blocking modals only for short decisions, risky approvals, or focused configuration.
- Use docked/persistent panels for progress, logs, diagnostics, and long-running workflow visibility.

## 7. State And Layering Governance Gap

Observed behavior:

- UI state is distributed across app store signals, component-local signals, modals, dashboards, overlays, and IPC event streams.
- This is workable, but production polish needs explicit ownership boundaries.

Impact:

- Future contributors can accidentally duplicate state or create contradictory UI states.
- Overlay-heavy workflows can create layering conflicts.
- Recovery, approval, and workflow state can become visually inconsistent.

Planning decision:

- Add a frontend state ownership model.
- Add z-index/layering rules.
- Add review checks for new modals, overlays, and persistent panels.

---

# Layout + Compactness Audit

## Current Assessment

KRIA should remain desktop-first and productivity-oriented. The UI should be compact, but not cramped. It should avoid both empty marketing-page spacing and dense unstructured control clutter.

The current shell already supports a productivity layout:

```text
sidebar + topbar + primary workspace + status/footer surfaces + modals
```

This should be preserved.

## Compactness Problems To Audit

| Surface | Audit Concern |
|---|---|
| Chat | Message width, code block width, attachment chip spacing, input bar height, thinking row height. |
| Settings | Section padding, large repeated groups, long vertical forms, advanced controls always visible. |
| Dashboards | Too many cards, repeated headings, raw data blocks, nested panels. |
| Modals | Header/footer height, scroll containment, body padding, action placement. |
| Sidebar | Session density, active state clarity, collapse behavior. |
| Status bar | Useful compact signal vs visual noise. |

## Layout Refinement Rules

- Keep primary workflows one click away.
- Use 4px/8px/12px/16px/24px spacing scale.
- Use smaller headings inside panels and tools.
- Avoid large hero-style type inside operational surfaces.
- Avoid nested cards.
- Prefer full-width workspace regions and compact repeated rows.
- Keep action buttons stable in size and position.
- Use fixed min/max sizes for repeated controls to prevent layout shift.
- Use `min-width: 0` and proper overflow handling in flex/grid children.
- Avoid horizontal overflow in modals and tables.

## Empty-Space Governance

Whitespace is a tool, not an aesthetic goal.

Rules:

- Use whitespace to separate task regions, not to create a marketing-page feel.
- Avoid oversized cards around simple controls.
- Avoid tall empty headers inside operational tools.
- Keep repeated rows compact and scannable.
- Use dense tables/lists for logs, evals, sessions, providers, and devices.
- Preserve minimum touch/click target size without inflating every control.
- Prefer content-width constraints for prose, full-width constraints for code/tables/logs.

## Density Strategy

KRIA should support two density levels later if needed:

| Density | User |
|---|---|
| Comfortable | Default for layman users and first-run flows. |
| Compact | Power users, developers, dashboards, evals, logs. |

Do not add a density toggle until shared spacing tokens exist. First normalize the default density.

---

# Chat UI Audit

## Current Role

The chat surface is KRIA's primary interaction surface. It must serve both:

- layman users asking natural tasks,
- developers reading code, tool calls, logs, plans, and verification output.

## Required Chat Quality Bar

Chat should feel:

```text
readable
stable
fast
copy-friendly
keyboard-friendly
large-output-safe
developer-readable
non-technical-user-readable
```

## Message Layout Audit

Audit:

- role separation,
- assistant vs user bubble width,
- avatar visibility,
- message grouping,
- timestamp strategy,
- retry/regenerate placement,
- copy controls,
- long message wrapping,
- markdown spacing,
- table overflow,
- code block readability,
- tool call nesting,
- image preview behavior,
- attachment chip behavior.

Refinement plan:

- Preserve current bubble model.
- Make copy/retry actions visible on focus, not hover only.
- Keep assistant technical output wider than normal conversation output.
- Use readable max-widths for prose and full-width containers for tables/code/logs.
- Provide table horizontal scroll instead of shrinking text too aggressively.
- Keep code font size compact but readable.
- Use consistent spacing between paragraphs, lists, code, and tool outputs.

## Message Surface Hierarchy

Chat must not render every message artifact as the same kind of bubble. Different content classes need different density, affordance, and visibility.

| Content Type | Rendering Priority | Default Treatment |
|---|---|---|
| Assistant prose | Primary | Readable conversation width with normal markdown. |
| User prompt | Primary | Compact user bubble, easy to scan. |
| Code | Primary technical | Syntax-highlighted block, copy action, horizontal overflow safe. |
| Tool evidence | Secondary verified | Distinct evidence container with trust/status labels. |
| Workflow state | Secondary operational | Compact step/status panel, not long prose. |
| Logs | Tertiary detail | Collapsed or height-limited by default with expand/copy. |
| Eval reports | Tertiary detail | Summary first, raw report behind disclosure. |
| Attachments/images | Contextual | Thumbnail/chip first, full preview on demand. |
| Errors/partial completion | High priority | Clear severity container with recovery action. |

Rules:

- Prose, code, logs, tool evidence, and workflow state must not visually compete equally.
- Verified evidence should be visually distinct from assistant narration.
- Raw logs should never dominate the conversation unless the user explicitly expands them.
- Workflow state should be compact, current, and actionable.

## Markdown And Code Audit

Current rendering uses `marked`, `DOMPurify`, and `highlight.js`.

Risks:

- Large markdown can trigger expensive re-renders.
- Code block copy currently appears to be injected through rendered HTML.
- Inline handlers in sanitized HTML are not ideal for a production desktop app.
- Tables, huge logs, and long planning docs need controlled overflow.

Refinement plan:

- Move code-copy behavior to component-managed event delegation or explicit Solid components.
- Avoid allowing inline event handlers in sanitized markdown output.
- Memoize markdown rendering per message content hash where practical.
- Add collapsed rendering for extremely large tool/log blocks.
- Add "expand full output" for huge logs and generated reports.
- Preserve syntax highlighting but cap expensive highlighting for very large blocks.

## Markdown Rendering Caps

Production chat rendering needs hard limits:

| Item | Default Cap | Behavior After Cap |
|---|---:|---|
| Highlighted code block | 400 lines or 80 KB | Render plain text preview, offer expand/open. |
| Log block | 200 lines or 60 KB | Collapse middle, preserve head/tail, offer full copy. |
| Table columns | viewport-safe width | Horizontal scroll, do not shrink below readability. |
| Single message rendered HTML | measured budget | Defer non-visible heavy blocks. |
| Images | bounded preview size | Open full image on demand. |

Implementation guidance for later phases:

- Defer syntax highlighting for offscreen blocks.
- Avoid re-highlighting unchanged messages.
- Use content hashes or message IDs for memoization.
- Prefer progressive rendering for huge generated docs.
- Never let one message freeze the input composer.

## Streaming UX Audit

Audit:

- thinking state visibility,
- streaming cursor behavior,
- scroll anchoring,
- user reading history during streaming,
- interrupted stream recovery,
- partial response display.

Refinement plan:

- Auto-scroll only when user is near the bottom.
- Show a "new output" affordance when the user has scrolled away.
- Keep thinking and tool-running states compact.
- Separate "model is thinking" from "tool is executing" from "waiting for approval."

## Attachments Audit

Audit:

- drag/drop state,
- paste state,
- accepted file types,
- upload errors,
- attachment removal,
- keyboard focus,
- file preview,
- large file feedback.

Refinement plan:

- Use consistent attachment chips.
- Show file type, name, size, and error state.
- Avoid oversized previews by default.
- Make drag/drop visible but restrained.

## Chat Non-Negotiables

- Chat must remain fast with long sessions.
- User must be able to copy code and tool output reliably.
- Large markdown must not destroy layout.
- Hidden backend work must not be presented as visible GUI success.
- Chat must clearly show partial completion, blockers, and next actions.

---

# Settings UX Audit

## Current Role

Settings are KRIA's control center. They must support:

- normal setup,
- provider configuration,
- local/cloud model switching,
- runtime behavior,
- GUI automation,
- safety/HITL,
- integrations,
- hardware/fleet,
- advanced developer configuration.

## Current Risk

Settings currently concentrate many product domains into one large modal. This is powerful but can feel dense and hard to reason about.

## Settings Organization Strategy

Use layered settings:

| Layer | Content |
|---|---|
| Basic | Theme, language, active provider/model, voice enablement, core safety toggles. |
| Workflow | GUI automation, HITL, approvals, automation behavior. |
| Providers | LLM providers, local/cloud runtime, model availability, API keys. |
| Integrations | Google, Telegram, n8n, OpenClaw, search, MCP. |
| Runtime | Hardware, Colab, Ironclad, services, diagnostics. |
| Advanced | Developer-only controls, raw config, experimental labs. |

This does not require full page redesign. It can be achieved inside the existing modal with better sectioning, search, and component extraction.

## Settings Navigation Layers

The settings modal should keep one shell, but it needs internal hierarchy.

Recommended navigation:

| Layer | Default Visibility | Examples |
|---|---|---|
| Basic | Always visible | Theme, language, active model, voice on/off, core safety. |
| Workflow | Visible to most users | GUI automation, HITL, approvals, execution behavior. |
| Integrations | Visible but grouped | Google, Telegram, n8n, search, MCP, OpenClaw. |
| Advanced | Collapsed by default | Runtime tuning, experimental labs, hardware detail. |
| Developer | Searchable and explicit | raw IDs, diagnostics, eval toggles, debug config. |

Rules:

- Keep the modal shell, but avoid one flat settings universe.
- Show Basic first unless the user entered from a specific deep link.
- Preserve direct access for power users through search and anchors.
- Do not bury high-frequency settings behind deep nesting.
- Do not expose high-risk controls without clear context.

## Discoverability Safeguards

Progressive disclosure must not become hidden UX.

Controls may be collapsed by default only when at least one is true:

- the control is low-frequency,
- the control is advanced or risky,
- the control is diagnostic-only,
- the control is irrelevant until a parent feature is enabled,
- the control would overwhelm a first-run user.

Controls should remain discoverable through:

- settings search,
- section summaries,
- visible advanced toggles,
- contextual links from related error states,
- keyboard navigation,
- stable terminology.

## Settings Interaction Requirements

Audit and refine:

- tab labels,
- section descriptions,
- setting grouping,
- save/apply/reset behavior,
- dirty-state indication,
- validation,
- error placement,
- disabled-state explanation,
- environment-locked values,
- provider health feedback,
- search result highlighting,
- advanced section collapse,
- keyboard navigation.

## Settings Component Strategy

Introduce or standardize reusable primitives:

- `SettingsShell`,
- `SettingsSection`,
- `SettingsField`,
- `SettingsFieldGroup`,
- `SettingsToggle`,
- `SettingsSelect`,
- `SettingsNumberInput`,
- `SettingsSecretInput`,
- `SettingsHelpText`,
- `SettingsValidationMessage`,
- `SettingsAdvancedDisclosure`,
- `SettingsActionRow`,
- `SettingsStatusBadge`.

Do not extract everything in one pass. Start with the most repeated patterns.

## Settings Copy Strategy

For layman users:

- use short labels,
- avoid backend jargon in primary labels,
- place technical names in secondary text,
- explain risk in plain language.

For developers:

- keep exact provider/model/runtime IDs visible where useful,
- allow raw values in advanced sections,
- show diagnostic state without hiding it.

---

# Model Selector UX Audit

## Current Role

The model selector and provider settings are critical because they determine how KRIA thinks, falls back, and handles local/cloud capability.

## Required Model Selector Signals

The UI should clearly show:

| Signal | User Value |
|---|---|
| Active provider | User knows which backend is answering. |
| Active model | User knows which model is selected. |
| Local vs cloud | User understands privacy/performance/cost implications. |
| Health | User knows whether provider is available. |
| Fallback | User knows when KRIA changed execution path. |
| Env lock | User understands why a value cannot be changed. |
| Context size | Developers can judge long prompt behavior. |
| Cost/token hints | Cloud usage is not surprising. |
| Download/loading state | Local model availability is understandable. |

## UX Problem To Avoid

Do not expose backend complexity as the default experience.

Bad default:

```text
Show every provider, runtime, route, fallback, context, scheduler, lease, and diagnostic at once.
```

Better default:

```text
Show active model, health, local/cloud class, and a concise fallback indicator.
Put detailed routing and diagnostics behind expandable controls.
```

## Refinement Plan

- Provide a compact active model pill in the main shell.
- In settings, show provider health and active runtime in a consistent status row.
- Group local and cloud models separately.
- Show unavailable models as disabled with a reason.
- Use "recommended" labels sparingly and only when backed by current capability.
- Keep advanced model metadata expandable.
- Standardize provider testing feedback:
  - testing,
  - healthy,
  - unavailable,
  - auth missing,
  - model missing,
  - fallback active.

---

# Workflow UX Audit

## Current Workflow Surfaces

KRIA has multiple workflow-related surfaces:

- GUI workflow viewer,
- HITL modal,
- permission modal,
- decision action center,
- substrate status,
- plan visualization,
- test runner dashboard,
- analytics dashboard,
- voice overlay,
- provider fallback alerts.

## Production Workflow UX Goal

The user should feel:

```text
KRIA is working, visible, recoverable, and under control.
```

The user should not feel:

```text
KRIA is silently doing unknown things in the background.
```

## Workflow State Model

Use consistent states:

| State | Meaning |
|---|---|
| Understanding | KRIA is interpreting the request. |
| Planning | KRIA is choosing execution mode and required evidence. |
| Waiting | KRIA needs user input, approval, login, or missing capability. |
| Running | KRIA is executing bounded work. |
| Verifying | KRIA is checking evidence. |
| Partial | Some work succeeded, but required workflow fidelity failed. |
| Completed | Required result and fidelity were satisfied. |
| Failed | Required result was not completed. |
| Recovered | KRIA completed after a bounded fallback or retry. |

## Workflow Refinement Plan

- Standardize progress indicators across GUI automation, tools, HITL, and evals.
- Show current step and next step for long-running tasks.
- Distinguish "backend completed" from "visible workflow completed."
- Use clear partial-completion language.
- Keep approval actions stable and unambiguous.
- Show stale/invalidated approval state clearly.
- Show recovery options as concrete buttons where possible.
- Avoid raw backend errors as the only user-facing message.

## Operational Surface Priority Doctrine

KRIA has many operational surfaces. They must have explicit priority so the UI does not become dashboard sprawl.

| Priority | Surface Class | Default Behavior |
|---|---|---|
| Primary | Chat, active task, HITL decision, current workflow status | Always easy to reach. |
| Secondary | Provider/model status, GUI automation status, session state, device health | Visible as compact status or one-click panel. |
| Tertiary | Analytics, eval reports, forensics, n8n, raw logs, debug details | Dashboard or disclosure-only, not primary flow. |
| Debug-only | Raw traces, internal metrics, low-level runtime packets | Hidden behind developer/diagnostic mode. |

Rules:

- Primary workflow information should never be buried under dashboards.
- Secondary operational state should be compact and glanceable.
- Tertiary/debug surfaces should not compete with chat.
- Dashboard panels should summarize first and reveal detail on demand.

## Workflow Information Prioritization

For complex workflows, display information in this order:

1. Current state.
2. Required user action, if any.
3. What KRIA completed.
4. What KRIA is doing next.
5. What failed or degraded.
6. Evidence and verification details.
7. Raw logs and diagnostic traces.

Do not show raw logs before the user understands the workflow state.

## HITL UX Requirements

HITL surfaces must:

- show what action is being proposed,
- show target identity,
- show risk level,
- show what will happen after approval,
- show what changed if approval becomes stale,
- never imply approval from passive visibility,
- support keyboard confirmation/cancel with safe defaults.

---

# Dark Mode Audit

## Current Dark Mode Assessment

Dark mode has broad token support and appears to be the historically dominant theme. It uses deep backgrounds, green/blue accents, translucent surfaces, overlays, and glow effects.

## Risks

- Too many translucent layers can reduce clarity.
- Accent glows can feel visually busy in operational tools.
- Hardcoded dark colors in inline styles can break light mode parity.
- Contrast must be checked on secondary/muted text.
- Background decoration must not compete with dense information surfaces.

## Dark Mode Refinement Plan

- Keep dark mode first-class.
- Reduce decorative intensity where it harms readability.
- Ensure all panels, dropdowns, modals, code blocks, tables, and toasts use semantic tokens.
- Audit contrast for:
  - muted text,
  - disabled controls,
  - warning states,
  - success states,
  - code comments,
  - selected rows.
- Ensure shadows/elevation are visible but not muddy.
- Use subtle borders more than heavy blur.

## Dark Mode Non-Negotiables

- No washed-out gray text.
- No low-contrast disabled controls.
- No over-bright neon accent dominance.
- No background effects under critical reading surfaces.

---

# Light Mode Audit

## Current Light Mode Assessment

Light tokens exist in `theme-shell.css`, and app state defaults to light when no saved value exists. However, the HTML boot script defaults to dark, which must be corrected in a later implementation phase.

## Light Mode Production Goal

Light mode should feel:

```text
clean
sharp
calm
professional
not washed out
not low contrast
```

## Light Mode Visual Identity

Light mode should not become a sterile white enterprise dashboard.

Principles:

- Use warm-neutral or cool-neutral off-white app backgrounds instead of pure white everywhere.
- Use crisp dark text with restrained muted text.
- Use subtle borders to define structure.
- Use low, soft shadows only for true elevation.
- Keep green/blue accent use purposeful and limited.
- Avoid glassy overlays that look cloudy in light mode.
- Ensure code and logs feel technical and readable, not pale.
- Use compact, precise spacing to keep the app productivity-oriented.

## Light Mode Risks

- White surfaces can become visually flat without proper borders and elevation.
- Muted text can become too pale.
- Warning/error colors need enough contrast.
- Inline dark colors can leak into light mode.
- Transparent overlays designed for dark mode may look dirty in light mode.

## Light Mode Refinement Plan

- Make light mode the first-paint default.
- Define clear surface levels:
  - app background,
  - workspace background,
  - panel surface,
  - raised control surface,
  - modal surface.
- Use restrained shadows and visible borders.
- Use stronger text hierarchy instead of oversized headings.
- Audit all hardcoded colors.
- Ensure code blocks remain readable with light highlight theme.
- Ensure dropdowns, overlays, and scrollbars are not afterthoughts.

---

# Responsiveness Audit

## Desktop-First Requirement

KRIA is a desktop app. Responsiveness should support different desktop window sizes, not turn the product into a mobile web layout.

Target environments:

- small laptop windows,
- 1080p desktop,
- high DPI screens,
- fractional scaling on Linux,
- ultrawide monitors,
- resized Tauri windows,
- touchpad scrolling,
- keyboard-heavy usage.

## Desktop-First, Not Mobile-Primary Doctrine

KRIA should be responsive, but it is not a mobile-primary app.

Rules:

- Optimize for desktop productivity first.
- Support narrow desktop windows without bloating layout.
- Do not convert dense tools into oversized mobile cards.
- Do not hide primary desktop controls behind mobile hamburger patterns unless the window is genuinely narrow.
- Treat mobile-like scaling as a fallback, not the design center.
- Preserve keyboard and pointer workflows as first-class.

## Audit Areas

| Surface | Responsiveness Checks |
|---|---|
| App shell | Sidebar collapse, topbar overflow, status bar stability. |
| Chat | Message width, input bar height, code/table overflow, scroll anchoring. |
| Settings | Modal height, tab overflow, form grid behavior, sticky actions. |
| Dashboards | Card wrapping, chart scaling, table overflow, dense lists. |
| Modals | Header/footer fixed behavior, body scroll, action button visibility. |
| Dropdowns | Positioning near screen edges, max height, keyboard navigation. |
| Voice overlay | Does not block critical controls unless active. |
| HITL | Approval buttons remain visible at small heights. |

## Refinement Plan

- Add viewport breakpoints for desktop sizes:
  - 1280px and up,
  - 900px to 1279px,
  - 700px to 899px,
  - narrow fallback below 700px.
- Use CSS grid/flex constraints rather than JS layout hacks.
- Keep key controls visible during scroll.
- Put raw logs/tables in scroll containers.
- Avoid changing font size based on viewport width.
- Test fractional scaling and long labels.

---

# Accessibility Audit

## Current Assessment

Some accessibility foundations are present:

- `aria-live` is used on some status surfaces.
- `role="status"` appears in progress surfaces.
- global focus-visible styling exists.
- some modal close buttons have `aria-label`.

Production accessibility requires more systematic coverage.

## Required Accessibility Checks

Audit:

- keyboard navigation,
- tab order,
- focus trap in modals,
- Escape behavior,
- screen reader labels,
- hover-only interactions,
- button labels,
- icon-only controls,
- disabled-state explanations,
- contrast ratios,
- colorblind-safe status indicators,
- reduced-motion support,
- live regions for streaming/progress,
- form validation messages,
- drag/drop alternatives,
- copy controls accessible by keyboard.

## Accessibility Refinement Plan

- Ensure all icon-only controls have labels or tooltips.
- Ensure tooltips are not the only source of required information.
- Make hover-only actions visible on keyboard focus.
- Standardize focus ring color and offset.
- Use semantic buttons instead of clickable divs where possible.
- Add modal focus trap and restore focus after close.
- Add status text alongside color indicators.
- Respect `prefers-reduced-motion`.
- Ensure all form errors are associated with the relevant input.

## Automated Accessibility Testing Strategy

Manual accessibility review is required, but it is not enough.

Add automated checks in later implementation phases:

| Tool Class | Purpose |
|---|---|
| Playwright accessibility smoke tests | Keyboard navigation, modal focus, critical flows. |
| axe-core or equivalent | Detect missing labels, contrast issues, invalid ARIA, landmark issues. |
| Screenshot contrast checks | Catch low-contrast theme regressions. |
| Reduced-motion test mode | Ensure animations and transitions respect user preference. |
| Keyboard-only route tests | Verify chat, settings, provider selector, HITL, and modals without pointer input. |

Minimum automated coverage targets:

- open settings,
- switch tabs,
- change model/provider selection,
- send chat prompt,
- copy code block,
- open/close HITL modal,
- approve/reject safe mock action,
- navigate dashboard tabs,
- open and close overlays.

## Keyboard-First Productivity Layer

Power-user UX requires more than keyboard reachability.

Planned layer:

- command palette for common actions,
- consistent shortcut registry,
- visible shortcut help,
- focus cycling through sidebar, chat, workflow status, and composer,
- Escape behavior for overlays,
- slash commands in chat,
- keyboard-accessible copy/retry/regenerate,
- shortcut-safe HITL approvals with explicit confirmation.

Do not implement advanced keyboard features before focus behavior and modal governance are stable.

## Accessibility Non-Negotiables

- Every critical action must be keyboard reachable.
- Every destructive action must have clear text and safe default focus.
- Color cannot be the only indicator of status.
- Reduced motion users must not get unnecessary animation.

---

# Micro-Interaction Audit

## Goal

Micro-interactions should improve orientation and perceived responsiveness. They should not be decorative noise.

## Motion Doctrine

Motion exists to assist cognition, not to decorate the app.

Allowed motion:

- communicate state change,
- show progress,
- preserve orientation during panel changes,
- confirm a user action,
- reduce perceived waiting.

Avoid motion that:

- runs continuously without user value,
- draws attention away from the active task,
- hides latency,
- animates layout-heavy properties,
- makes dense operational views feel unstable,
- violates reduced-motion preference.

## Audit Areas

- hover states,
- active states,
- focus states,
- disabled states,
- modal open/close,
- dropdown open/close,
- tab switching,
- sidebar collapse,
- streaming indicators,
- progress bars,
- copy feedback,
- attachment drag/drop,
- workflow step transitions,
- toast appearance,
- loading skeletons,
- voice listening state.

## Timing Guidelines

| Interaction | Target |
|---|---|
| Hover/focus | 80ms to 120ms. |
| Dropdown/modal entrance | 120ms to 180ms. |
| Progress changes | Smooth but not delayed. |
| Toast entrance/exit | 120ms to 200ms. |
| Streaming cursor | Subtle, disabled under reduced motion. |

## Refinement Plan

- Standardize transition durations and easing tokens.
- Avoid animating layout-heavy properties.
- Prefer opacity/transform for lightweight transitions.
- Use reduced motion alternatives.
- Provide immediate click feedback for long-running actions.
- Show copy success inline and briefly.
- Keep workflow progress animations calm.

---

# Visual Consistency Audit

## Current Assessment

KRIA has a recognizable visual direction, but it needs consistency governance.

Current consistency risks:

- mixed border radius sizes,
- mixed hardcoded colors,
- mixed button styles,
- mixed card patterns,
- mixed inline dashboard styles,
- multiple status badge styles,
- multiple input/select styles,
- multiple modal/panel surface styles.

## UI Consistency Doctrine

KRIA UI should follow these rules:

```text
one spacing scale
one typography scale
one radius scale
one elevation scale
one status color system
one control style per control role
one modal shell
one settings field pattern
one table/log pattern
one workflow progress pattern
```

## Visual Information Hierarchy Doctrine

Every screen should clearly distinguish:

| Hierarchy Level | Meaning | Visual Treatment |
|---|---|---|
| Primary | Current task, active conversation, required user action | Strongest text, clearest surface, stable position. |
| Secondary | Supporting state, provider/model status, workflow progress | Compact status rows, badges, side panels. |
| Tertiary | Diagnostics, logs, eval detail, historical traces | Collapsed, scrollable, or dashboard-contained. |

Rules:

- Do not give all panels equal visual weight.
- Do not make debug surfaces look as important as user decisions.
- Use color and elevation sparingly for priority, not decoration.
- Keep the active task visually dominant.

## Typography Doctrine

Typography should make KRIA feel precise and calm.

Recommended scale:

| Role | Size | Weight | Usage |
|---|---:|---:|---|
| App title/major page | 20-24px | 650-700 | Rare, top-level only. |
| Section heading | 16-18px | 600-700 | Settings sections, dashboard panels. |
| Subsection heading | 13-15px | 600 | Groups inside tools and modals. |
| Body | 13-14px | 400-500 | Chat prose, settings labels. |
| Dense body | 12-13px | 400-500 | Tables, status rows, metadata. |
| Caption/meta | 11-12px | 400-500 | Hints, timestamps, diagnostics. |
| Code/logs | 12-13px | 400-500 | Monospace technical output. |

Rules:

- Do not use hero-scale typography inside operational panels.
- Keep line height readable for chat prose.
- Keep dense metadata compact but legible.
- Use font weight before increasing size.
- Avoid negative letter spacing.

## Visual Restraint Doctrine

KRIA should feel modern and intelligent without fake AI-futuristic styling.

Forbidden or heavily restricted patterns:

- decorative glow fields behind dense text,
- excessive glassmorphism,
- animated gradient backgrounds,
- neon status overload,
- oversized decorative cards,
- abstract AI ornaments,
- background effects under code/logs/chat,
- multiple competing accent colors in one panel,
- decorative motion with no task value.

Allowed patterns:

- subtle accent on primary actions,
- restrained surface elevation,
- crisp borders,
- compact badges,
- calm progress indicators,
- purposeful iconography,
- readable technical surfaces.

## Concrete Aesthetic Constraints

"Beautiful" means:

- aligned,
- readable,
- consistent,
- responsive,
- calm,
- precise,
- visually balanced,
- low-friction.

It does not mean:

- flashy,
- glossy,
- oversized,
- decorative,
- animated by default,
- visually loud.

## Design Token Strategy

Recommended token groups:

| Token Group | Purpose |
|---|---|
| `color.bg.*` | App, workspace, panel, raised, overlay. |
| `color.text.*` | Primary, secondary, muted, inverse, disabled. |
| `color.border.*` | Subtle, default, strong, focus. |
| `color.accent.*` | Primary action, hover, active, soft. |
| `color.status.*` | Success, warning, danger, info, neutral. |
| `space.*` | 4, 8, 12, 16, 20, 24, 32. |
| `radius.*` | 4, 6, 8, 12. |
| `shadow.*` | None, low, modal, popover. |
| `motion.*` | Fast, normal, slow, easing. |
| `font.*` | Sans, mono, sizes, weights, line heights. |

Do not introduce a heavy external design-system dependency unless the current CSS approach becomes unmaintainable.

## Token Minimalism Doctrine

Tokenize only repeated semantic values.

Tokenize:

- repeated colors,
- spacing scale,
- radius scale,
- typography roles,
- status colors,
- elevation levels,
- motion durations,
- focus ring,
- overlay layers.

Do not tokenize:

- one-off layout values,
- component internals used once,
- arbitrary shades without semantic meaning,
- every possible font size,
- every temporary experiment.

If a token does not make future UI safer or more consistent, it should not exist.

## Status Color Doctrine

Status should be consistent:

| Status | Meaning |
|---|---|
| Success | Completed and verified. |
| Warning | Degraded, partial, retryable concern. |
| Danger | Failed, blocked, unsafe, destructive. |
| Info | Neutral progress or available detail. |
| Muted | Disabled, unavailable, historical, inactive. |

Use text labels with status color. Do not rely on color alone.

---

# Frontend Architecture Audit

## Current Architecture Strengths

- SolidJS signals provide efficient reactive state.
- IPC event batching exists in `appStore`, which is valuable.
- Lazy-loaded dashboard components reduce initial bundle pressure.
- Component tests exist for important surfaces.
- CSS is centralized enough to be improved incrementally.

## Current Architecture Risks

| Risk | Evidence |
|---|---|
| Large components | `SettingsModal.tsx`, `MessageBubble.tsx`, `App.tsx`, `ProviderSettings.tsx`, `ChatView.tsx` are all large enough to increase change risk. |
| Inline styles | Many dashboard and settings surfaces use inline styles. |
| Token drift | Multiple CSS token naming patterns and fallbacks. |
| Modal layering risk | Multiple modals/overlays can coexist from app shell. |
| Render pressure | Chat markdown, logs, and high-frequency runtime events can become heavy. |
| Accessibility drift | Custom controls need consistent ARIA/focus behavior. |

## Architecture Refinement Plan

Refactor by stable seams:

1. Extract shared primitives, not new pages.
2. Move repeated styles into classes and tokens.
3. Split large components by product domain.
4. Preserve route model and test selectors where possible.
5. Add visual regression coverage before significant polish.
6. Avoid broad rewrites during UX refinement.

## Frontend State Ownership Model

State should have one clear owner.

| State Class | Owner |
|---|---|
| Global app/session/theme/provider state | `appStore` or equivalent shared store. |
| Route selection | App shell. |
| Modal visibility | App shell, unless modal is private to one component. |
| Form draft values | Local section component until saved/applied. |
| Workflow progress | Workflow-specific store/event stream, rendered by workflow components. |
| HITL decision state | HITL/decision store, not duplicated in unrelated panels. |
| Tooltip/dropdown open state | Local component. |
| Dashboard filters | Dashboard component or persisted dashboard store if cross-session. |

Rules:

- Do not duplicate authoritative state in multiple components.
- Derived UI state should be computed, not manually synchronized.
- Modal state must not outlive the workflow it represents.
- Long-running workflow state should not be trapped inside a modal component.
- New stores require a clear ownership reason.

## Modal Governance Rules

Use modals only when the user must stop and decide.

| Surface Type | Preferred Pattern |
|---|---|
| Risky approval | Blocking modal or HITL panel. |
| Short configuration | Modal is acceptable. |
| Long-running workflow | Docked or persistent panel. |
| Logs/diagnostics | Drawer, panel, or dashboard detail. |
| Passive status | Status strip, toast, or inline panel. |
| Non-critical help | Popover or inline hint. |

Rules:

- No unbounded modal stacking.
- A modal may open another modal only for a clearly nested confirmation.
- Escape and cancel behavior must be predictable.
- Focus must be trapped inside blocking modals and restored on close.
- Long-running operations should survive modal close when safe.

## Layering Hierarchy Specification

Use a small z-index/layer model:

| Layer | Purpose |
|---:|---|
| 0 | App background. |
| 10 | Main shell and workspace. |
| 20 | Sticky topbar/status surfaces. |
| 30 | Docked panels and drawers. |
| 40 | Dropdowns/popovers/tooltips. |
| 50 | Non-blocking overlays. |
| 60 | Blocking modals. |
| 70 | Critical HITL/security approval. |
| 80 | Toasts and transient global alerts. |

Rules:

- Do not create arbitrary z-index values.
- Layer values should map to named tokens.
- HITL/security approvals must not be visually obscured by normal overlays.
- Tooltips should not appear over blocking approval text.

## Candidate Component Boundaries

| Current Area | Suggested Boundary |
|---|---|
| `SettingsModal.tsx` | Settings shell, tab nav, provider tab, voice tab, safety tab, UI tab, automation tab, integrations tab, hardware tab, advanced tab. |
| `MessageBubble.tsx` | Markdown renderer, code block, tool call renderer, web result renderer, image renderer, trust badge, action row. |
| `ChatView.tsx` | Message list, composer, attachment tray, empty state, streaming state. |
| `ProviderSettings.tsx` | Provider list, active runtime card, model picker, health test row, advanced metadata. |
| Dashboards | Shared metric card, status strip, table, log viewer, empty/error/loading states. |

---

# Error/Loading/Empty State Audit

## Current Need

KRIA exposes many states where users need clear feedback:

- model unavailable,
- provider auth missing,
- local runtime warming,
- GUI automation unavailable,
- workflow partial completion,
- HITL waiting,
- eval running,
- browser/app capability missing,
- device/fleet disconnected,
- voice listening/processing errors,
- integration failure.

## State Design Requirements

Every state should answer:

```text
what happened?
is it still running?
what can I do now?
what did KRIA preserve?
what is the safe next step?
```

## Loading State Plan

- Use skeletons for stable page regions.
- Use compact spinners only for short operations.
- Use progress bars when progress is known.
- Use stage labels for long workflows.
- Avoid indefinite "loading" with no recovery option.

## Empty State Plan

Empty states should be useful but compact:

- explain what belongs here,
- provide one primary next action,
- avoid marketing copy,
- avoid oversized illustrations,
- avoid hiding advanced users from direct controls.

## Error State Plan

Errors should include:

- clear title,
- user-readable cause,
- technical detail behind disclosure,
- retry action if safe,
- recovery option,
- copy diagnostics action for developer workflows.

## User-Facing Severity Taxonomy

Use a consistent severity model:

| Severity | Meaning | UI Treatment |
|---|---|---|
| Info | Normal background status or helpful note. | Low-emphasis inline status. |
| Notice | User may want to know, but no action required. | Compact banner or status chip. |
| Warning | Degraded, partial, retryable, or attention needed. | Visible warning with recovery action. |
| Blocked | KRIA cannot proceed without user action or missing capability. | Prominent state with next action. |
| Error | Operation failed and needs retry/recovery. | Error panel with cause and action. |
| Critical | Safety, destructive, credential, approval, or irreversible concern. | Blocking HITL/security surface. |

Rules:

- Use "critical" only for genuine safety or irreversible actions.
- Partial completion is usually warning or blocked, not success.
- Developer diagnostics should be secondary to the user-facing cause.
- Never show a raw stack trace as the primary message.

## Partial Completion Plan

For workflow automation, partial is a first-class state:

```text
Completed:
- file created
- command ran

Not completed:
- visible app verification failed

Next:
- reopen app
- retry visible verification
- continue structurally with disclosure
```

Do not label this as full completion.

---

# Performance and Smoothness Audit

## Performance Risks

| Area | Risk |
|---|---|
| Chat | Long markdown, code highlighting, tool logs, image previews, auto-scroll. |
| Dashboards | High-frequency event updates, charts, large tables. |
| Settings | Huge DOM from many settings sections. |
| Theme switching | Broad repaint from many tokenized surfaces. |
| Animations | Blur/shadow/large backdrop effects can be expensive. |
| Logs | Large preformatted output can freeze rendering. |

## Current Positive Signal

`appStore` batches high-frequency backend events before updating SolidJS signals. This is the right direction and should be preserved.

## Performance Refinement Plan

- Memoize expensive markdown rendering.
- Cap syntax highlighting for very large code blocks.
- Collapse large tool/log outputs by default.
- Avoid auto-scroll when user is reading history.
- Virtualize or window very long message lists if sessions become large.
- Lazy-load heavy dashboard panels.
- Avoid expensive backdrop blur in nested overlays.
- Use `requestAnimationFrame` batching consistently for high-frequency events.
- Measure startup time, first usable interaction, stream smoothness, and memory growth.

## UI Responsiveness Budgets

Target budgets for later implementation:

| Interaction | Target Budget |
|---|---:|
| First usable app shell after frontend load | under 1000ms on normal dev machine. |
| Open settings modal | under 150ms perceived response. |
| Switch settings tab | under 100ms perceived response. |
| Type in chat composer during streaming | no dropped input frames. |
| Send prompt feedback | visual acknowledgement under 100ms. |
| Open model selector | under 150ms perceived response. |
| Render normal message | under 50ms after message data arrives. |
| Render very large message preview | under 150ms, with deferred detail. |
| Theme switch | no full-app jank or unreadable flash. |
| Dashboard tab switch | under 200ms perceived response. |

These are product budgets, not hard real-time guarantees. Regressions should be visible in profiling or telemetry.

## Frontend Telemetry And Observability Plan

Add lightweight frontend observability later:

- startup timing,
- route switch timing,
- settings open timing,
- message render timing,
- markdown/code highlight timing,
- long task detection,
- dropped-frame hints during streaming,
- dashboard update frequency,
- modal open/close counts,
- error and recovery state counts.

Rules:

- Do not collect sensitive prompt content for telemetry.
- Prefer aggregate timings and state counts.
- Keep telemetry optional or local-first if privacy is a concern.
- Use telemetry to catch regressions, not to add noise to the UI.

## Smoothness Success Criteria

- No visible UI freeze during normal streaming.
- Settings modal opens quickly.
- Theme switch completes without jarring flash.
- Large messages remain scrollable.
- Dashboard updates do not block chat input.

---

# Layman vs Developer UX Strategy

## Core Strategy

KRIA must support layered complexity:

```text
simple default surface
clear guided actions
visible status
advanced controls behind disclosure
raw technical detail available on demand
```

## Layman User Needs

- simple language,
- clear primary action,
- understandable model/provider status,
- safe defaults,
- visible progress,
- helpful recovery,
- no unexplained backend jargon.

## Developer User Needs

- dense controls,
- exact model/provider/runtime IDs,
- copyable logs,
- keyboard shortcuts,
- fast navigation,
- advanced config,
- diagnostics,
- eval/run details,
- source and artifact paths.

## Balancing Rules

- Use plain labels first, technical detail second.
- Use progressive disclosure, not hidden functionality.
- Keep advanced panels searchable.
- Keep keyboard shortcuts discoverable.
- Do not dumb down errors; layer them.
- Do not overload the default view with every diagnostic.

## Lightweight Onboarding And Help Strategy

KRIA should guide layman users without turning the app into a tutorial.

Recommended approach:

- first-run setup wizard stays focused on essentials,
- empty chat state offers a few practical starter actions,
- contextual help appears near complex settings,
- advanced concepts link to concise explanations,
- command palette can expose available actions,
- tooltips explain icons but do not replace labels for critical actions,
- recovery messages teach the next step only when needed.

Avoid:

- long onboarding tours,
- blocking education screens,
- repeated tips,
- generic marketing copy,
- hiding core functionality until onboarding is complete.

---

# Design-System Stabilization Plan

## Objective

Create a small internal design system that supports KRIA's existing UI without becoming a separate product.

## Required Primitives

| Primitive | Purpose |
|---|---|
| `Button` | Primary, secondary, ghost, danger, icon. |
| `IconButton` | Compact tool actions with tooltip and label. |
| `Input` | Text, number, secret, search. |
| `Select` | Provider/model/settings selection. |
| `Toggle` | Binary settings. |
| `Tabs` | Settings and dashboard section navigation. |
| `Badge` | Status, trust, health, model class. |
| `Panel` | Standard surface container. |
| `ModalShell` | Consistent overlay, focus trap, header/body/footer. |
| `Toast` | Success/error/info feedback. |
| `EmptyState` | Compact empty guidance. |
| `ErrorState` | Recovery-oriented errors. |
| `LoadingState` | Skeleton/progress/spinner patterns. |
| `LogViewer` | Scrollable/copyable logs. |
| `CodeBlock` | Highlighted code with accessible copy. |
| `StatusStrip` | Runtime/model/workflow compact state. |

## Primitive Simplicity Rule

Shared primitives must reduce duplication without becoming a private framework.

Create a primitive only when:

- the pattern appears in at least three places,
- the pattern has accessibility requirements,
- the pattern carries theme complexity,
- the pattern has repeated interaction behavior,
- the pattern is high-risk to implement inconsistently.

Do not create a primitive for:

- one-off layout,
- experimental UI,
- page-specific composition,
- simple wrappers with no semantic value,
- abstractions that hide important behavior.

## Token Migration Strategy

1. Inventory existing token usage.
2. Define canonical token names.
3. Map old tokens to canonical tokens temporarily.
4. Move hardcoded colors into tokens.
5. Convert repeated inline styles to classes.
6. Remove duplicate token families only after surfaces are migrated.

## Design-System Guardrail

Do not create an overbuilt design system. Only add primitives where duplication or inconsistency is already visible.

---

# Minimal vs Necessary Redesign Decisions

## Preserve By Default

Preserve:

- current app shell,
- sidebar plus main workspace layout,
- chat-first interaction model,
- settings modal concept,
- dashboard route,
- VM/device management route,
- prompt lab route,
- voice overlay concept,
- HITL modal concept,
- provider settings functionality,
- existing workflow/test/eval surfaces.

## Redesign Only If Necessary

Redesign is justified only when current structure causes one of these:

- users cannot find critical controls,
- visual hierarchy makes the workflow ambiguous,
- layout breaks under normal desktop sizes,
- accessibility cannot be fixed locally,
- component structure makes safe maintenance impractical,
- current UI causes false confidence about risky actions.

## Likely Necessary Restructures

| Area | Reason |
|---|---|
| Settings internals | Too broad for one component and one flat mental model. |
| Message rendering internals | Too much responsibility in one surface. |
| Theme boot contract | Required to make light mode default. |
| Shared control styles | Required for visual consistency. |
| Error/loading/partial states | Required for trust and recoverability. |

These are refinement restructures, not destructive redesigns.

---

# UI Component Refinement Plan

## App Shell

Plan:

- Keep sidebar/topbar/workspace/status structure.
- Improve route active states.
- Ensure topbar controls do not overflow.
- Keep runtime/model status compact.
- Add consistent keyboard shortcuts display.
- Ensure modals restore focus to launch controls.

Shell constraints:

- The app shell must keep the assistant workspace dominant.
- The sidebar should support sessions and navigation without becoming a dashboard.
- Runtime status should be glanceable, not a second toolbar full of controls.
- Docked workflow panels may appear when they clarify active work.
- Debug/analytics surfaces belong in dashboard routes or explicit drawers.

## Sidebar

Plan:

- Keep session density high.
- Improve active session contrast in both themes.
- Ensure rename/delete controls are keyboard reachable.
- Avoid hover-only critical actions.
- Keep collapse behavior predictable.

## Chat Composer

Plan:

- Keep textarea autogrow with stable max height.
- Make attachments visible in a compact tray.
- Provide clear disabled/running state.
- Keep send button stable.
- Ensure slash commands are discoverable without being intrusive.

## Message Bubble

Plan:

- Split rendering responsibilities.
- Make copy controls accessible.
- Improve large code/table/log overflow.
- Use consistent tool call status badges.
- Clearly distinguish verified evidence, unverified output, and assistant narration.

## Settings Modal

Plan:

- Split settings tabs into section components.
- Add consistent field primitives.
- Add advanced disclosures.
- Keep sticky footer actions where useful.
- Add unsaved-change awareness if settings are not immediate.
- Keep search and validation visible.

## Provider Settings

Plan:

- Make active provider/model obvious.
- Separate local/cloud/provider class.
- Show health and fallback status.
- Collapse advanced metadata.
- Keep exact IDs copyable for developers.

## Workflow And HITL Components

Plan:

- Standardize step/state language.
- Show proposed action, target, risk, and approval consequence.
- Make rejection/cancel visually safe and accessible.
- Show stale decision state clearly.
- Keep recovery actions concrete.

Long-running workflow rule:

- Use modals for approval or interruption.
- Use docked/persistent panels for ongoing workflow status, logs, retries, and recovery.
- If a modal starts a long operation, the operation should continue with visible status after the modal closes when safe.

## Dashboards

Plan:

- Use shared metric cards and status strips.
- Reduce nested panels.
- Keep raw logs behind disclosure.
- Make tables responsive with horizontal scroll.
- Use consistent empty/error/loading states.

---

# Frontend Cleanup Recommendations

## Cleanup Priorities

1. Fix theme default contract.
2. Define canonical design tokens.
3. Extract shared controls used by settings and providers.
4. Replace repeated inline styles in high-traffic surfaces.
5. Split `SettingsModal.tsx` by settings domain.
6. Split `MessageBubble.tsx` by renderer type.
7. Add accessible modal shell and focus handling.
8. Add standardized error/loading/empty state components.
9. Add large-content protections in chat and dashboards.
10. Add visual regression and accessibility checks.

## Test Stability Requirements

Preserve or intentionally migrate selectors used by existing tests:

- `.chat-input`,
- `.send-btn`,
- `.voice-btn`,
- `.chat-messages .message-bubble`,
- `.settings-btn`,
- `.modal-overlay`,
- `.modal`,
- `.close-btn`,
- `.settings-section`.

If selectors change, update tests in the same implementation phase.

## UX Consistency Enforcement

Every UI change should pass a lightweight review checklist:

- Does it preserve the chat-first workflow?
- Does it work in light and dark mode?
- Does it use existing tokens or justified new tokens?
- Does it use an existing primitive where appropriate?
- Is it keyboard reachable?
- Does it have visible focus state?
- Does it avoid hover-only critical actions?
- Does it preserve compact density?
- Does it avoid modal stacking?
- Does it handle loading, empty, error, and disabled states?
- Does it preserve or update tests?

## Visual Regression Governance

Add screenshot diff testing for high-value surfaces:

- app shell in light mode,
- app shell in dark mode,
- chat with prose/code/tool output,
- long markdown/log message,
- settings basic/provider/advanced sections,
- model selector states,
- HITL approval modal,
- GUI workflow progress,
- dashboard overview,
- device/fleet view,
- error/partial-completion state.

Rules:

- Screenshot diffs should run for UI refinement PRs.
- Baselines should be updated intentionally, not casually.
- Light and dark mode baselines are both required.
- Visual tests should cover compact desktop widths and normal desktop widths.

## No-Giant-Rewrite Rule

Do not rewrite the frontend stack. Current SolidJS/Tauri structure is adequate. The production need is component discipline and token consistency.

---

# Production UI/UX Principles

1. Preserve existing functionality.
2. Improve clarity before decoration.
3. Light mode is the default and must be first-class.
4. Dark mode remains first-class.
5. Compact does not mean cramped.
6. Beautiful does not mean flashy.
7. Status must be understandable without reading logs.
8. Advanced controls must remain available.
9. Risky actions require clear confirmation and safe defaults.
10. Workflow partial completion must be visually distinct from success.
11. Copy, retry, approve, cancel, and recover actions must be keyboard reachable.
12. No critical action may depend on hover-only UI.
13. Color must not be the only status channel.
14. Large markdown/log/code output must not break layout or freeze the app.
15. Avoid fake futuristic styling that harms productivity.
16. Avoid excessive whitespace that reduces workspace efficiency.
17. Avoid enterprise-dashboard overload.
18. Prefer shared primitives over one-off UI.
19. Every visual token must work in light and dark modes.
20. Frontend cleanup must be incremental and test-backed.
21. Chat-first assistant workflow is the canonical UX anchor.
22. Dashboards are supporting surfaces, not the product center.
23. Modals are for short blocking decisions, not long-running operations.
24. Progressive disclosure must preserve discoverability.
25. Tokens must stay minimal and semantic.
26. Motion must assist cognition, not decorate.
27. Typography must follow a clear hierarchy.
28. Layering and z-index must use named levels only.
29. Keyboard-first workflows must be treated as product features.
30. Visual regression checks must protect production polish.

---

# Recommended Incremental Rollout Plan

## Phase 0: Baseline Audit And Screenshots

Deliver:

- screenshot inventory for all major surfaces,
- current light/dark screenshots,
- keyboard navigation notes,
- contrast hotspots,
- performance baseline for chat and settings,
- list of inline style hotspots,
- modal/layering inventory,
- dashboard priority inventory,
- settings navigation inventory.

Exit criteria:

- current UI risk is documented before changes.
- no implementation changes required in this phase.

## Phase 1: Theme Contract And Token Stabilization

Deliver:

- light mode first-paint default,
- app store and HTML theme agreement,
- canonical token map,
- token minimalism rules,
- typography scale,
- visual hierarchy rules,
- deprecated token aliases,
- first hardcoded-color cleanup pass.

Exit criteria:

- no startup theme flash.
- basic surfaces look correct in light and dark mode.

## Phase 2: Shared UI Primitives

Deliver:

- button,
- icon button,
- badge,
- input,
- select,
- toggle,
- modal shell,
- status strip,
- error/loading/empty state primitives,
- primitive simplicity checklist,
- z-index/layer tokens.

Exit criteria:

- new UI work uses shared primitives.
- no broad visual redesign yet.

## Phase 3: Chat Refinement

Deliver:

- accessible copy controls,
- large markdown/log safeguards,
- hard rendering caps,
- deferred rendering for expensive blocks,
- message surface hierarchy,
- better scroll anchoring,
- improved code/table overflow,
- clearer streaming/tool/waiting states.

Exit criteria:

- chat remains fast and readable with long technical output.

## Phase 4: Settings And Model Selector Refinement

Deliver:

- settings navigation layers,
- settings section components,
- progressive disclosure,
- discoverability safeguards,
- consistent field layout,
- provider/model status normalization,
- unavailable/fallback/env-lock states.

Exit criteria:

- layman users can configure basics.
- developers can still reach advanced controls.

## Phase 5: Workflow, HITL, And Recovery UX

Deliver:

- consistent workflow state language,
- workflow information prioritization,
- partial completion UI,
- approval/rejection clarity,
- stale approval UI,
- recovery action patterns,
- modal governance rules,
- docked/persistent workflow status rules.

Exit criteria:

- users can understand what KRIA did, what failed, and what happens next.

## Phase 6: Dashboard And Operational Surface Refinement

Deliver:

- operational surface priority doctrine,
- shared dashboard components,
- compact status cards,
- responsive tables/logs,
- reduced nested panels,
- consistent analytics/test/fleet states.

Exit criteria:

- operational views remain dense but less visually fragmented.

## Phase 7: Accessibility, Responsiveness, And Performance Hardening

Deliver:

- automated accessibility checks,
- keyboard-first productivity layer,
- keyboard audit fixes,
- modal focus trap,
- reduced motion compliance,
- responsive desktop testing,
- long-session performance checks,
- frontend responsiveness budgets,
- frontend telemetry hooks,
- visual regression checks.

Exit criteria:

- UI passes production readiness checks for accessibility, performance, and desktop resizing.

---

# Deferred Changes

Defer:

- complete visual rebrand,
- mobile-first redesign,
- new frontend framework,
- full settings route rewrite,
- heavy animation system,
- decorative AI/futuristic visual overhaul,
- full dashboard redesign,
- custom window chrome overhaul,
- theme marketplace,
- large design-system package migration,
- replacing all CSS in one pass,
- moving every inline style in one phase,
- mobile app UX strategy.

These may be useful later, but they are not needed for production-grade refinement.

---

# Non-Negotiable Constraints

1. Do not break existing features.
2. Do not redesign the whole app by default.
3. Do not remove power-user controls.
4. Do not overwhelm layman users with raw backend complexity.
5. Do not hide advanced diagnostics from developers.
6. Light mode must be the default.
7. Dark mode must remain first-class.
8. UI must remain compact and efficient.
9. No excessive whitespace.
10. No fake futuristic styling that reduces clarity.
11. No unnecessary animation.
12. No hover-only critical controls.
13. No color-only status indicators.
14. No raw backend errors as the only user-facing failure state.
15. No full success state when workflow completion is partial.
16. No nested-card sprawl.
17. No giant frontend rewrite.
18. No broad route restructure unless user workflows require it.
19. Preserve existing test selectors or migrate tests intentionally.
20. Every new primitive must support light and dark modes.
21. Every modal must have keyboard and focus behavior.
22. Large chat/log/code output must be layout-safe.
23. Provider/model unavailable states must be understandable.
24. Settings must support both basic and advanced usage.
25. UI refinements must be incremental, reversible, and test-backed.
26. Chat-first assistant workflow remains the canonical UX anchor.
27. Dashboard, analytics, forensics, and eval surfaces must remain secondary or diagnostic unless actively selected.
28. Settings must use navigation layers, not one flat expanding control list.
29. Progressive disclosure must remain searchable and discoverable.
30. Shared primitives must stay simple and meaningfully reused.
31. Tokens must be semantic and minimal.
32. Typography must follow the agreed hierarchy.
33. Light mode must avoid sterile pure-white enterprise styling.
34. Motion must assist cognition and respect reduced-motion preference.
35. Visual restraint must block glow/gradient/glassmorphism creep.
36. Frontend state must have clear ownership.
37. Z-index and overlays must follow the layering hierarchy.
38. Long-running workflows must not be trapped in blocking modals.
39. Automated accessibility checks are required before production readiness.
40. Visual regression screenshots are required for polished surfaces.
41. UI responsiveness budgets must be tracked for critical interactions.
42. Error states must use the severity taxonomy.
43. Huge markdown, code, logs, and tables must use rendering caps or deferred rendering.
44. KRIA is desktop-first and must not drift into mobile-primary layout patterns.
45. Empty space must be governed by compact-density constraints.

---

# Final Production-Ready KRIA UI/UX Enhancement Architecture

The target UI/UX architecture is:

```text
KRIA Desktop App
  -> stable desktop shell
  -> chat-first canonical workflow anchor
  -> light-first theme contract
  -> dark/light semantic design tokens
  -> minimal token doctrine
  -> typography and hierarchy doctrine
  -> compact shared UI primitives
  -> primitive simplicity rule
  -> chat-first interaction surface
  -> message surface hierarchy
  -> accessible markdown/code/tool rendering
  -> hard caps for expensive rendering
  -> layered settings and provider controls
  -> settings navigation layers
  -> clear workflow/HITL/recovery states
  -> modal and layering governance
  -> docked/persistent workflow status where needed
  -> consistent dashboards and operational panels
  -> operational surface priority doctrine
  -> responsive desktop layout constraints
  -> desktop-first, not mobile-primary doctrine
  -> accessible keyboard and focus behavior
  -> keyboard-first productivity layer
  -> measured performance and smoothness
  -> frontend observability and visual regression governance
```

The final product should feel:

```text
compact
clean
beautiful
sleek
responsive
professional
efficient
stable
interactive
modern
trustworthy
```

for both:

- normal users,
- power users and developers.

The correct implementation posture is:

```text
refine what exists
standardize repeated patterns
strengthen interaction quality
make states honest and visible
improve maintainability
avoid destructive redesign
```

Final rule:

```text
If a UI change makes KRIA prettier but slower, less clear, less accessible, or less reliable, it is not a production UI improvement.
```
