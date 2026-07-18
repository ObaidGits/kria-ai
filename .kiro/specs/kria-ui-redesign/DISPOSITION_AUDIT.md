# Task 15.1 — Disposition Map Execution Audit

Requirement: **20.1**. Source: `KRIA_UI_REDESIGN_MASTERPLAN.md` §6.1. Executable source of truth: `ui/src/migration/dispositionMap.ts`; invariant tests: `ui/src/migration/dispositionMap.test.ts`.

## Result

Every capability-bearing current or live-but-unmounted surface is confirmed in its target Space/global layer. Dead shells and compatibility shims carry no unique capability; their deletion remains scoped to task 15.2.

| Current surface | Target | Result |
|---|---|---|
| Home/Chat | Converse · Conversation lane | Confirmed |
| Prompt Lab | Converse · Lab/tool-lock mode | Confirmed |
| Dashboard Ironclad strip | Observatory · Now/Forensics | Confirmed |
| Analytics toggle | Observatory · Analytics | Confirmed |
| n8n sub-tab | Automations · Run/Build/Schedule/History | Confirmed |
| Tests toggle | Observatory · Diagnostics | Confirmed |
| VM Management + DeviceMatrix | Machines · Fleet/terminal/alerts | Confirmed |
| Tasks | Automations · Schedule | Confirmed |
| Capabilities/CPP | Capabilities · Tools/Governance/Generate/Constellation | Confirmed |
| Memory | Memory · landing/lenses | Confirmed |
| Settings | Settings · eight searchable groups | Confirmed |
| MCP/Telegram/Google/Colab | Capabilities · Integrations | Confirmed |
| OpenClaw/SubstrateStatus/Marketplace | Capabilities · Skills/Governance | Confirmed |
| Mobile & Remote | Machines · mobile/remote desktop | Confirmed |
| HitlModal | Global Approval Center | Confirmed |
| DecisionActionCenter | Global Approval Center | Confirmed |
| GUI Cognition HITL | Global Approval Center | Confirmed |
| n8n HITL resume | Global Approval Center | Confirmed |
| VoiceOverlay/Onboarding | Global Core/Voice | Confirmed |
| Scattered toasts | Global Notification Center | Confirmed |
| Top/bottom status chrome | Global Core/status line | Confirmed |
| Descriptor/result/detail panes | Global Context Inspector | Confirmed |
| ExecutiveDashboard | Observatory · Jobs & Cognition | Revived, confirmed |
| PlanVisualization | Converse · plan-compare WorkBlock | Revived, confirmed |
| QuarantineQueue | Capabilities · Governance | Revived, confirmed |
| N8nDiagnosticsPanel value | Automations · Build/Health | Folded, confirmed |
| workflowSession HITL/cancel/continuation | Approval Center + work lane | Fixed, confirmed |
| CapabilityGraph/Manager/ExecutionLogs/PermissionManager shells | Capabilities concepts | No unique capability; cleanup pending 15.2 |
| N8nWorkflowBrowser/PermissionModal shims | Automations + Approval Center | No unique capability; cleanup pending 15.2 |

## Authority and safety gate

Execution-bearing rows declare and test the invariant `Intent → Capability → Policy → Substrate → Tool → Verification`. Each records approval, cancellation, and verification surfaces; `bypass` is structurally fixed to `false`. Manifest is presentation/migration metadata only: it adds no execution route, direct tool call, safety bypass, verification bypass, or cancellation bypass.

## Evidence model

Tests enforce: exact §6.1 current-row coverage; unique source rows; target homes limited to seven canonical Spaces or global layers; every Space represented; every capability-bearing current row `confirmed`; every evidence path checked in; every execution guard complete. Legacy file removal is intentionally deferred to task 15.2.
