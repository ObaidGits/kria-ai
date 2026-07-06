# Architecture Lock (Phase A0 — Authoritative Freeze List)

> This is the single authoritative record of what is **✅ frozen forever** vs **⚠ may evolve** for
> OpenClaw. Implementation (Phase A1+) may not contradict a ✅ item. A ⚠ item may change without an
> amendment. Amending a ✅ item requires the process in §12.

## 0. The seven invariants — ✅ FROZEN FOREVER

| # | Invariant | Frozen |
|---|-----------|--------|
| INV-1 | A skill is exactly one signed **Skill Bundle**; the manifest is the single source of truth; the descriptor is derived | ✅ |
| INV-2 | One `Capability`/`CapabilityGrant` object across declare→…→analytics | ✅ |
| INV-3 | Routing is data-driven; adding a skill needs zero router code | ✅ |
| INV-4 | One `SkillRuntime` interface for every backend | ✅ |
| INV-5 | One resource authority (HRA) admits all execution | ✅ |
| INV-6 | One safety authority (Rust core); KRIA assigns trust/risk | ✅ |
| INV-7 | One `SkillEvent` stream; audit/UI/telemetry/analytics are projections | ✅ |

## 1. Skill Package

| Item | Status |
|------|--------|
| Bundle = one signed artifact; layout (`manifest.toml`, `schema.json`, `handler/`, `deps/`, `examples/`, `tests/`, `MANIFEST.sha256`, `bundle.sig`) | ✅ |
| `manifest.toml` required fields (`skill`, `runtime`, `resource`, `capabilities`, `trust`, `compat`) | ✅ |
| Identity = (slug, publisher); slug + publisher immutable | ✅ |
| `manifest` authoritative; `SkillDescriptor` is a derived projection | ✅ |
| `schema.json` authoritative for params + result | ✅ |
| Mutability classes (widening caps/resources ⇒ re-approval; breaking schema/runtime ⇒ new major + reinstall) | ✅ |
| Semver + upgrade/deprecate/rollback rules | ✅ |
| Optional fields (icon, README, extra metadata) | ⚠ |
| Distribution format (`.tar.zst`), dep-resolution mechanism | ⚠ |

## 2. Capability

| Item | Status |
|------|--------|
| Single `Capability{kind,mode,scope}` + `CapabilityGrant` object | ✅ |
| Scope is mandatory (no unscoped power) | ✅ |
| Risk = KRIA-owned pure function of granted caps | ✅ |
| Lifecycle declare→review→approval→materialize→runtime→audit→revoke→history→analytics | ✅ |
| Re-approval on widening; silent narrowing | ✅ |
| Set of `CapabilityKind` variants | ⚠ (additive only) |
| Materialization mechanics per backend, lease durations | ⚠ |

## 3. Router

| Item | Status |
|------|--------|
| One data-driven router (unify on `tool_index`; retire `resolver.rs`) | ✅ |
| `RouterEntry` derived from manifest is the only router input | ✅ |
| 6-stage pipeline: native-first → semantic → source quotas → trust weighting → per-turn cap → manual lock | ✅ |
| Anti-tool-soup is structural (quotas + cap), not per-skill | ✅ |
| Hot re-register + index rebuild on install/uninstall/toggle/upgrade | ✅ |
| Trust/risk are ranking+gating inputs (not hidden filters) | ✅ |
| Embedding model, dense/lexical weights, quota numbers, ranking formula | ⚠ |

## 4. Execution

| Item | Status |
|------|--------|
| `SkillRuntime` trait + lifecycle order (prepare→admit→launch→monitor→call→cancel→recover→cleanup→recycle) | ✅ |
| Backend selected from metadata + host + policy; never call-site branches | ✅ |
| Isolation floor per RuntimeKind; policy may only strengthen | ✅ |
| Cancellation must release lease + tear down instance | ✅ |
| Correlation id + end-to-end skill-declared timeout | ✅ |
| MCP default transport + adapter seam for non-MCP | ✅ |
| Concrete RuntimeKind set | ⚠ (additive) |
| Pooling/recycle heuristics, transport optimizations | ⚠ |

## 5. Resource

| Item | Status |
|------|--------|
| Single `ResourceRequest`/`Lease`; mandatory HRA admission for all execution | ✅ |
| Lease ↔ instance binding | ✅ |
| Priority ladder (OpenClaw below realtime voice; floor + aging) | ✅ |
| Cancellation/preemption releases lease; bounded retry | ✅ |
| Actual cost recorded (cpu_ms, peak_mem, gpu_ms, storage, latency, queue_wait) | ✅ |
| Remote admission via signed leases | ✅ |
| Exact budgets/floors, queue aging, GPU partitioning granularity, cgroup tunings | ⚠ |

## 6. Security

| Item | Status |
|------|--------|
| (slug, publisher) identity; publisher = stable key | ✅ |
| Mandatory signing + content-hash pin; no unsigned network/subprocess/gpu skills | ✅ |
| KRIA-assigned trust ladder (Verified/Community/Local/Generated/Untrusted) | ✅ |
| Server-side approval token bound to (slug, version, granted caps, budget, schema epoch) | ✅ |
| No secrets in sandbox; brokered, scoped, short-lived, audited | ✅ |
| Audit key from a secret store, `key_id` per entry; `verify_chain` scheduled | ✅ |
| Single capability enforcement point (materialize + deny-by-default) | ✅ |
| Enterprise overlay is tighten-only | ✅ |
| PKI/revocation distribution, signature params, interim key store, per-tier runtime choice | ⚠ |

## 7. Event / Observability

| Item | Status |
|------|--------|
| Single `SkillEvent`; closed `Stage` set; required-fields-per-stage | ✅ |
| Correlation id spans composition + agent-to-agent | ✅ |
| Failures enumerated (closed set on control path; message is detail only) | ✅ |
| Audit/UI/telemetry/analytics are projections of the one stream | ✅ |
| Only security-relevant events are signed/persisted to audit | ✅ |
| Telemetry exporters, analytics formulas, retention, additive Stage/FailureKind | ⚠ |

## 8. Extension

| Item | Status |
|------|--------|
| Every extension is additive over the seven invariants; none may change a frozen contract | ✅ |
| Composition = DAG of executions sharing correlation id | ✅ |
| Agent = a skill with Remote runtime; generated skill = a produced bundle | ✅ |
| Marketplace = signed index + tighten-only policy overlay | ✅ |
| Scheduling = trigger into the normal path (durable scheduler) | ✅ |
| Composer/planner internals, scheduler policy, enterprise policy language, index schema (additive) | ⚠ |

## 9. Cross-contract consistency check (validated in A0)

- Identity keys are distinct and never merged: **identity=(slug,publisher)**,
  **approval=hash(descriptor+grants+budget+schema epoch)**, **signature=content hash**. ✅ consistent
  across package + capability + security.
- The `CapabilityGrant` object is the same in capability, security (approval token), execution
  (materialize), resource (network need), and event (grant_ref). ✅ no duplicate representation.
- `correlation_id` is defined once (event) and consumed by execution, resource, extension. ✅
- HRA `Lease` is defined once (resource) and bound in execution + released on cancel everywhere. ✅
- Trust/risk assigned once (capability/security), consumed by router + policy. ✅

## 10. What A0 intentionally leaves open (anti-over-freeze)

Embedding model, vector index internals, seccomp syscall lists, cgroup values, exact quota/budget
numbers, marketplace wire schema beyond required fields, per-tier runtime choice, retention. These
are ⚠ so implementation can tune without re-freezing.

## 11. Rework-risk verdict

The three contracts whose *absence* would force catalog-wide rework are all frozen here:
1. **Skill-bundle format** (package) — ✅ locked before any skill is authored.
2. **Capability-grant object** (capability) — ✅ one object, one enforcement point.
3. **Data-driven single router** (router) — ✅ zero router edits per skill.
With these frozen, adding skills, backends, and extensions is additive. **The user's core concern
— repeating auto/routing/permission work per feature — is structurally eliminated.**

## 12. Amendment process (for ✅ items only)

A ✅ item may be changed only via a written amendment that: (a) states the invariant it touches,
(b) shows why no additive design suffices, (c) lists every consumer affected, (d) provides a
migration path (migration-plan pattern), and (e) is recorded as an ADR in `docs/ADR/`. Absent that,
✅ items are fixed for the life of OpenClaw.

---

**Phase A0 status: COMPLETE.** All contracts frozen, cross-checked, and self-reviewed. No code was
written or modified. Implementation may begin at Phase A1 (`implementation-roadmap.md` → A1: replace
`docker attach` with in-process bollard exec) against these contracts.
