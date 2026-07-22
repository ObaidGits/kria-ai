# KRIA Documentation

This is the canonical documentation entry point. KRIA docs are organized as
production documentation, not implementation scratch notes or temporary rollout
logs.

Use this file to find the authoritative document for each subsystem.

## Structure

```text
docs/
  index.md
  architecture/
    overview.md
    core-runtime.md
    llm-orchestrator-runtime.md
    gui-cognition-runtime.md
    safety-hitl-runtime.md
    presence-homepage-runtime.md
    memory-graph-current-state.md
  orchestration/
    runtime-authority.md
    tool-system.md
    gui-execution.md
    opgraph-contract.md
  contracts/
    memory-graph-current-contract.md
    hitl-mvp/
      01-boundary.md
      02-runtime-contracts.md
      03-action-proposal.md
      04-execution-gate.md
      05-decision-invalidation.md
      06-state-transitions.md
      07-resource-lease.md
      08-safety-invariants.md
      09-audit-scope.md
      10-eval-plan.md
  operations/
    development.md
    deployment.md
    provider-orchestration.md
    hardware.md
  evaluations/
    overview.md
    gui-e2e.md
    voice-validation.md
  integrations/
    n8n.md
    openclaw.md
  llm-context/
    index.md
    entry-points.md
    query-guide.md
    project-graph-summary.md
    project-graph.json
    context-scope.json
    routing-corpus.jsonl
  decisions/
    adr/
      001-e2e-eval-harness.md
      002-tool-execution-overhaul.md
      003-browser-page-content-scope.md
      004-presence-first-homepage.md
      005-hybrid-navigation.md
      006-modal-vs-page-framework.md
      007-view-modes-and-companion.md
      008-3d-core-capability-gating.md
    rfc/
      007-gui-system-control.md
      008-recursive-intelligence.md
  voice/
    overview.md
  reference/
    source-navigation.md
    design-system.md
    motion.md
    accessibility.md
    navigation.md
    memory-graph-host-capabilities.md
```

`docs/index.md` is the docs entry point. Do not create a parallel
`docs/README.md` unless a packaging tool explicitly requires it.

## Authority Map

| Domain | Canonical document |
|---|---|
| Memory Graph shipped architecture | `architecture/memory-graph-current-state.md` |
| Memory Graph current facade/renderer contract | `contracts/memory-graph-current-contract.md` |
| Memory Graph host capabilities | `reference/memory-graph-host-capabilities.md` |
| Platform overview | `architecture/overview.md` |
| Core runtime | `architecture/core-runtime.md` |
| Runtime flow | `architecture/overview.md` |
| Subsystem boundaries | `architecture/overview.md` |
| Result synthesis | `architecture/core-runtime.md` |
| Memory architecture | `architecture/core-runtime.md` |
| LLM orchestration | `architecture/llm-orchestrator-runtime.md` |
| GUI cognition runtime | `architecture/gui-cognition-runtime.md` |
| Safety + HITL runtime | `architecture/safety-hitl-runtime.md` |
| Presence homepage runtime | `architecture/presence-homepage-runtime.md` |
| Runtime authority | `orchestration/runtime-authority.md` |
| Tool system | `orchestration/tool-system.md` |
| GUI execution | `orchestration/gui-execution.md` |
| OpGraph execution contract | `orchestration/opgraph-contract.md` |
| HITL MVP contracts | `contracts/hitl-mvp/01-boundary.md` through `contracts/hitl-mvp/10-eval-plan.md` |
| Voice runtime | `voice/overview.md` |
| Provider operations | `operations/provider-orchestration.md` |
| Development | `operations/development.md` |
| Deployment | `operations/deployment.md` |
| Hardware | `operations/hardware.md` |
| Evaluation system | `evaluations/overview.md` |
| GUI E2E runbook | `evaluations/gui-e2e.md` |
| Voice validation | `evaluations/voice-validation.md` |
| n8n integration | `integrations/n8n.md` |
| OpenClaw integration | `integrations/openclaw.md` |
| AI/LLM development context | `llm-context/index.md` |
| Source navigation | `reference/source-navigation.md` |
| Design system (homepage tokens/stories) | `reference/design-system.md` |
| Motion system | `reference/motion.md` |
| Accessibility contract (homepage) | `reference/accessibility.md` |
| Navigation architecture | `reference/navigation.md` |

## Reading Order

For a new engineer or LLM agent:

1. `architecture/overview.md`
2. `reference/source-navigation.md`
3. `orchestration/runtime-authority.md`
4. `orchestration/tool-system.md`
5. The subsystem-specific document for the change being made.

For GUI cognition work:

1. `architecture/gui-cognition-runtime.md`
2. `orchestration/gui-execution.md`
3. `orchestration/opgraph-contract.md`
4. `evaluations/gui-e2e.md`

For voice work:

1. `voice/overview.md`
2. `evaluations/voice-validation.md`
3. `architecture/safety-hitl-runtime.md`
4. `orchestration/runtime-authority.md`

For HITL contract work, preserve the numeric order under
`contracts/hitl-mvp/`.

## Documentation Rules

1. A subsystem has one canonical overview document.
2. Deep architecture docs stay in `architecture/`.
3. Execution authority and runtime contracts stay in `orchestration/`.
4. Implementation-binding contracts stay in `contracts/`.
5. ADRs and RFCs are historical decision records under `decisions/`; do not edit
   them as live architecture unless the decision record itself is being amended.
6. Generated eval reports, logs, phase notes, debug notes, and one-off planning
   docs are not permanent documentation.
7. The HITL MVP contract pack keeps numeric order because implementation and
   review depend on that sequence.
8. AI-facing project context belongs in `llm-context/`; do not recreate a
   separate `ai-context/` folder.
9. Keep filenames lowercase kebab-case for Markdown and JSON docs.
10. When code changes runtime behavior, update the matching canonical doc in the
    same change set.
