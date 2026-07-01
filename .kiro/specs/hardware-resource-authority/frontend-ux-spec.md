# HRA Frontend & UX Specification

Frontend is a first-class part of HRA: the "never surprise the user" guarantee only exists if the
UI surfaces it. Stack: SolidJS + TailwindCSS (existing `ui/`). All data arrives via the additive
`resource:*` Tauri event stream + a read-only RA snapshot query (`resource_snapshot` command).
No existing command/event names change (N5).

## Principles
- Calm, non-blocking. Foreground input is NEVER disabled for non-emergency actions.
- Every state has a "why" with evidence (journal seq + telemetry window).
- Emergency interruptions are rare, labeled, progress-bearing, and auto-resume.

## Data plane (frontend)
- `ui/src/stores/resource.ts` — subscribes to `resource:status`, `resource:plan`,
  `resource:forecast`, `resource:lease`, `resource:recovery`, `resource:thermal`, `resource:session`.
- Snapshot query `resource_snapshot()` → `{ devices, leases, queue, session, profile, forecasts }`.
- Each event carries `correlation_id` → resolvable in Diagnostics.

## View 1 — Resource Dashboard (`ResourceView.tsx`)
Shows live: active leases (consumer, device, budget, class, ttl), per-Device state (free/used VRAM,
util, temp), queue (per-class depth + position), residency map (which model is VramHot/RamWarm/Cold),
pressure level per device, thermal state, active PolicyProfile, and the current Plan rationale.
- Visual: per-device cards + a residency timeline. Non-blocking banner for active swaps.

## View 2 — Explainability UI
Answers, with evidence, on demand:
- "Why was a model unloaded?" → journal entry (idle-release/preempt) + telemetry window + class.
- "Why was cloud used?" → plan rationale `FailoverCloud` + local capacity snapshot + breaker state.
- "Why was image generation delayed?" → queue wait + Tier-B swap timeline + VRAM barrier samples.
- "Why was a lease denied?" → contended device + holder class + queue position + fallback offered.
Each answer renders the `RationaleCode` → human string + a link to the raw journal/telemetry slice.

## View 3 — Session Awareness UI
Shows current `SessionProfile` (Coding/Voice/Image/Automation/Research/Idle/Mixed), active
`PolicyProfile`, the resource strategy in effect (e.g., "LLM pinned warm, embeddings batched"),
and predicted upcoming workloads from WPE with confidence.

## View 4 — Forecasting UI
Displays RFE output: predicted VRAM/RAM pressure curves with "time to threshold," thermal forecast,
and scheduled prewarm actions (what WPE will warm next and why). Clearly labeled as predictions.

## View 5 — Recovery UI
Surfaces reliability actions in human terms: checkpoint saves/restores, failovers (+ failback),
reconciler reclaims (what orphan was cleaned), epoch bumps after a Core restart. Each with timestamp
+ outcome + evidence link. This makes "invisible background work" visible on demand (R9.5).

## View 6 — Diagnostics export
One-click export of a signed diagnostics bundle: telemetry ring slice, journal slice, events,
traces, and any anomaly root-cause reports. For support + bug reports. Machine-readable + summary.

## Emergency UX contract
- Non-emergency action → no banner OR a small calm "optimizing in background" chip; input stays live.
- Emergency (true OOM) → labeled notice "Freed GPU memory to keep things stable — resuming your
  response," progress indicator, partial answer preserved, auto-continue. Never a silent cancel.
- This explicitly replaces the current abrupt "Optimizing GPU layers..." behavior in `ChatView.tsx`.

## Accessibility
- Status uses `role="status"`/`aria-live="polite"`; emergency uses `aria-live="assertive"`.
- All evidence links keyboard-reachable; color is not the only pressure signal (icon + text).

## Acceptance (frontend)
- FE1 Dashboard reflects live RA snapshot within 1 s of change.
- FE2 Each Explainability question resolves to a real journal entry + telemetry window.
- FE3 No view disables foreground input for a non-emergency action.
- FE4 Emergency notice shows, preserves partial output, and auto-resumes (matches A16).
- FE5 Diagnostics bundle exports and re-imports for support review.

## Final-pass additions (folded into existing views — no new top-level clutter)

These extend the six existing views rather than adding new screens. Goal: surface the final-pass
backend additions calmly.

- **Session ownership** (into View 3 — Session Awareness): a single line showing the current
  Foreground Owner, Interactive Owners, and Background Owners (e.g., "Foreground: Chat · Background:
  2 agents"). Makes concurrency arbitration legible; prevents "why is my chat slow" confusion.
- **SLA visibility** (into View 1 — Dashboard + View 6 — Diagnostics): per-operation SLA chips
  (Voice/Chat/Image/Automation/Cloud) colored Ok/Warning/Critical with the measured value and the
  threshold; click → evidence (telemetry window + journal). No always-on numbers spam — chips only
  turn amber/red on Warning/Critical, otherwise a single calm "SLAs nominal."
- **Resource simulation explanations** (into View 2 — Explainability): when an action was avoided or
  a fallback chosen, show the `Estimate` ("predicted free after evict: 1.2 GB < hard limit 1.5 GB →
  chose cloud"). Turns silent decisions into one-line evidenced reasons.
- **Residency visibility** (into View 1 — Dashboard residency map): show each model's
  `ResidencyState` (Hot/Warm/Cold/Swapping) with the last transition reason. Already partly present;
  now sourced from `resource:residency` events.
- **Capability visibility** (into View 3 / model picker): show the selected model's quality tier +
  latency class + why it was chosen (registry match), so model selection is explainable to the user.
- **Benchmark visibility** (into View 6 — Diagnostics): a "Benchmark" tab to run/view the harness
  report (before/after, per-hardware-class, regression flags). Developer/power-user facing; hidden
  behind an advanced toggle to keep default UX calm.
- **Memory bands** (into View 4 — Forecasting): the VRAM/RAM curves mark Soft/Hard/Emergency lines so
  forecasts read against meaningful thresholds.

### Updated frontend acceptance
- FE6 Session ownership line reflects live owners within 1 s.
- FE7 SLA chips show Ok by default; turn Warning/Critical on breach with evidence link.
- FE8 Avoided/fallback actions show the simulation estimate in Explainability.
- FE9 Residency map shows per-model state + last transition reason.
- FE10 Model selection shows registry-based rationale (quality/latency class).
- FE11 Benchmark report viewable/exportable from Diagnostics (advanced toggle).
