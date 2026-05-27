# KRIA Documentation

KRIA documentation is organized as production documentation, not as implementation scratch notes.

## Canonical Structure

```text
docs/
  index.md
  architecture/
    overview.md
    core-runtime.md
    llm-orchestrator-runtime.md
    gui-cognition-runtime.md
    safety-hitl-runtime.md
  orchestration/
    runtime-authority.md
    tool-system.md
    gui-execution.md
    opgraph-contract.md
  contracts/
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
  llm-context/
    index.md
    entry-points.md
    query-guide.md
    project-graph-summary.md
    project-graph.json
    context-scope.json
    routing-corpus.jsonl
  integrations/
    n8n.md
    openclaw.md
  decisions/
    adr/
    rfc/
  voice/
    overview.md
  reference/
    source-navigation.md
```

## Authority Map

| Domain | Canonical Document |
|---|---|
| Platform overview | `architecture/overview.md` |
| Core runtime | `architecture/core-runtime.md` |
| LLM orchestration | `architecture/llm-orchestrator-runtime.md` |
| GUI cognition runtime | `architecture/gui-cognition-runtime.md` |
| Operational cognition / OpGraph | `architecture/core-runtime.md` |
| Runtime flow | `architecture/overview.md` |
| Subsystem boundaries | `architecture/overview.md` |
| Result synthesis | `architecture/core-runtime.md` |
| Safety model | `architecture/safety-hitl-runtime.md` |
| Safety + HITL runtime | `architecture/safety-hitl-runtime.md` |
| Memory architecture | `architecture/core-runtime.md` |
| Orchestration authority | `orchestration/runtime-authority.md` |
| Tool contracts | `orchestration/tool-system.md` |
| GUI execution | `orchestration/gui-execution.md` |
| OpGraph execution contract | `orchestration/opgraph-contract.md` |
| HITL MVP contracts | `contracts/hitl-mvp/01-boundary.md` through `contracts/hitl-mvp/10-eval-plan.md` |
| Voice runtime | `voice/overview.md` |
| Provider operations | `operations/provider-orchestration.md` |
| OpenClaw integration | `integrations/openclaw.md` |
| n8n integration | `integrations/n8n.md` |
| Evaluation system | `evaluations/overview.md` |
| GUI E2E runbook | `evaluations/gui-e2e.md` |
| Voice validation | `evaluations/voice-validation.md` |
| AI/LLM development context | `llm-context/index.md` |
| Development | `operations/development.md` |
| Deployment | `operations/deployment.md` |
| Hardware | `operations/hardware.md` |
| Source navigation | `reference/source-navigation.md` |

## Rules

1. A subsystem has one canonical overview document.
2. Deep architecture docs stay in `architecture/`; execution contracts stay in `orchestration/` or `contracts/`.
3. ADRs and RFCs are historical decision records under `decisions/`.
4. Implementation trackers, debug notes, phase logs, and one-off planning notes are not permanent documentation.
5. The HITL MVP contract pack keeps its numeric order because downstream implementation and review depend on that sequence.
6. AI-facing project context belongs in `llm-context/`; do not create a separate duplicate context folder.
