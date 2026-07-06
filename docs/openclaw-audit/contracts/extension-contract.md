# Extension Contract (FROZEN — Phase A0)

> How future capabilities plug in **without redesign**: workflow composition, agent-to-agent,
> prompt-generated skills, marketplace, background/scheduled jobs, distributed/remote/GPU/cloud
> execution, enterprise policy. Each is an *additive consumer* of the frozen contracts, not a new
> architecture.

## 1. Principle

Every extension below is expressed purely in terms of the seven invariants. If an extension would
require changing a frozen contract, it is designed wrong — not the contract. This section proves
each extension fits.

## 2. Workflow composition (Execution Router / master Phase 8)

- A workflow is a **DAG of skill invocations** sharing one `correlation_id`. Each node is a normal
  execution (execution-contract); edges pass outputs (schema.json typed) to inputs.
- The composer is a *consumer* of the router (candidates + scores) and the execution contract
  (launch/monitor/cancel). No new skill format, no new router, no new runtime.
- Cancellation of the correlation cancels all live nodes (resource-contract binding).
- **Fits with zero contract change.**

## 3. Agent-to-agent

- An agent is addressed as an MCP endpoint; calling it is a `SkillRuntime` `call` over the
  Remote/Cloud backend with a signed lease. Sub-agent events share the `correlation_id`.
- Capability grants and HRA admission apply to the outbound call exactly like any skill.
- **Fits: agent = a skill whose runtime is Remote.**

## 4. Prompt-generated skills (master 9.3)

- Generation produces a **Skill Bundle** (package-contract) like any other, with
  `trust=Generated`, **vault-signed** (security-contract §1), runtime forced to microVM, network/
  subprocess denied unless per-run HITL.
- It flows through the identical install → approve → index → execute path. Router down-weights it
  until it earns audited successes (router-contract §4 / analytics projection).
- **Fits: generation is a new *producer* of the frozen bundle; consumers unchanged.**

## 5. Marketplace (community + enterprise)

- Marketplace = a signed index of `RemoteSkillEntry` (existing) extended with publisher key +
  content hash + revocation. Browse/install already exist; A0 adds signing + hash pin (security).
- Enterprise = a **tighten-only policy overlay** (security-contract §6) + a private index URL.
  Same install path; policy can forbid tiers/capabilities/publishers, never loosen.
- **Fits: marketplace is a source of bundles + a policy overlay.**

## 6. Background & scheduled jobs

- Reuse the durable scheduler (`tasks/scheduler.rs`, SQLite, restart-survivable). A scheduled job
  is a stored intent that, when fired, runs through the normal loop → router → execution path with
  `priority = OpenClawBatch/Scheduled` (resource-contract).
- Results/failures emit the same `SkillEvent`s and can notify via existing channels (ntfy/Telegram).
- **Fits: scheduling is a trigger, not a new execution path.**

## 7. Distributed / remote / GPU / cloud execution

- All are `RuntimeKind`s behind the one `SkillRuntime` trait (execution-contract §4). Admission is
  via local or remote HRA view with signed leases (resource-contract §7).
- The bundle, capabilities, router, events, and audit are identical regardless of where it runs.
- **Fits: location is metadata; the contract is placement-agnostic.**

## 8. Enterprise deployment

- Adds: private registry, tighten-only policy overlay, org publisher keys, centralized audit
  export (event projection). No change to package/capability/router/execution/resource contracts.
- **Fits: enterprise is configuration + policy overlay + audit export.**

## 9. Extension compatibility matrix

| Extension | New producer | New consumer | New RuntimeKind | Contract change? |
|-----------|-------------|--------------|-----------------|------------------|
| Composition | — | composer/planner | — | **No** |
| Agent-to-agent | — | — | (uses Remote) | **No** |
| Generated skills | generator | — | (uses microVM) | **No** |
| Marketplace | index | — | — | **No** |
| Enterprise | policy overlay | audit export | — | **No** |
| Background/scheduled | scheduler trigger | — | — | **No** |
| Distributed/GPU/cloud | — | — | Remote/Gpu/Cloud (additive) | **No** (additive enum) |

## 10. Self-review (challenge)

- *"Composition output→input typing could break across skill versions."* → schema.json epochs +
  semver gate compatibility; a composer validates edge types against current schemas before run.
- *"Generated skills could be abused to escalate."* → Forced strictest tier + microVM + no ambient
  caps + per-run HITL for any power; vault-signed provenance; analytics gate. Escalation requires
  explicit human approval each time.
- *"Enterprise overlay + community defaults + generated skills interaction."* → Overlay is
  tighten-only and evaluated last; the strictest applicable rule always wins. Deterministic.
- *"Remote worker trust."* → Trust the signed, time-bounded lease and audited events, not the
  worker's internal state. Same model as fleet execution already in-repo.
- *"Does 10k skills stress any contract?"* → Router (quotas+cap), registry (SQLite+ArcSwap),
  events (projections, sampled) all scale; the only cost is embedding re-index, which is a
  background job, not a contract change.

**Frozen:** every extension is an additive producer/consumer/RuntimeKind over the seven
invariants; **no extension may require changing a frozen contract**. If one appears to, the
extension is redesigned, not the contract.
**May evolve (⚠):** composer/planner internals, scheduler policy, marketplace index schema
(additive), enterprise policy language.
