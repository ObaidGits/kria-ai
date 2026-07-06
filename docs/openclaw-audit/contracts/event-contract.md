# Event / Observability Contract (FROZEN — Phase A0)

> INV-7: one event model for everything. UI, audit, telemetry, analytics, and cost accounting are
> all **projections** of this single stream. No parallel logging systems.

## 1. The one event

```rust
struct SkillEvent {
    // Identity / correlation (always present)
    correlation_id: Uuid,     // one per user-intent, spans composition/multi-step
    execution_id:   Uuid,     // one per SkillRuntime instance
    skill_id:       String,   // oc_<slug>
    version:        String,
    // Placement
    source:         ExecutionSource,   // Native|Mcp|OpenClaw|Cloud
    runtime:        RuntimeKind,       // Docker|Wasm|Firecracker|Remote|Cloud|Gpu
    instance_ref:   Option<String>,    // container/vm/worker id
    // State
    stage:          Stage,             // see §2
    ts:             DateTime,
    // Payload (stage-dependent)
    resource:       Option<ResourceUsage>,   // cpu_ms, peak_mem, gpu_ms, storage, net_bytes
    latency_ms:     Option<u64>,
    queue_wait_ms:  Option<u64>,
    partial_output: Option<String>,          // for Streaming (evidence-wrapped, untrusted)
    failure:        Option<FailureInfo>,      // kind, message, exit_code
    recovery:       Option<RecoveryInfo>,     // action, attempt, reason
    grant_ref:      Option<CapabilityGrantId>,// which grant this event concerns (security)
    reason:         Option<String>,
}
```

## 2. Stage lifecycle (frozen, closed set)

```text
Started → Preparing → Waiting(queue/admission) → Running → Streaming* →
   ├─ Completed
   ├─ Cancelled
   ├─ Preempted
   ├─ Failed(FailureInfo)
   ├─ Retrying(RecoveryInfo) → (back to Preparing)
   └─ Recovered
```

- Stages map 1:1 to execution-contract phases and resource-contract admission. Every stage
  transition emits exactly one event. `Streaming` may repeat.

## 3. Required fields per stage (frozen minimums)

| Stage | Must include |
|-------|--------------|
| Started | correlation_id, execution_id, skill_id, version, source, runtime |
| Preparing | + reason (what is being prepared) |
| Waiting | + queue_wait so-far, priority |
| Running | + instance_ref, grant_ref set materialized |
| Streaming | + partial_output (evidence-wrapped) |
| Completed | + latency_ms, resource (final usage) |
| Cancelled/Preempted | + reason |
| Failed | + failure{kind, message, exit_code} |
| Retrying | + recovery{action, attempt, reason} |

## 4. Projections (frozen: everyone reads the same stream)

```text
                       SkillEvent stream (one bus)
     ┌───────────────┬──────────────┬───────────────┬─────────────────┐
     ▼               ▼              ▼               ▼                 ▼
 UI (chat card,   Audit ledger   Telemetry/     Cost accounting   Analytics
  Activity view)  (HMAC, signed) metrics (OTel)  (per-skill $/res)  (trust, usage)
```

- **Audit** is a filtered, signed persistence of security-relevant events (security-contract §7).
- **UI** renders the live card (execution-contract) and the Activity/history view from the same
  events (ui-review recommendations).
- **Telemetry** exports counters/histograms (latency, admission wait, failure rate) — optional
  Prometheus/OTel (master roadmap 0.5).
- **Analytics** aggregates usage/trust signals feeding router weighting (router-contract §4).

## 5. Correlation model (frozen)

- `correlation_id` spans a whole user intent, including **multi-step composition** and
  **agent-to-agent** calls (extension-contract). Sub-executions share it, each with its own
  `execution_id`. This makes a composed workflow one traceable tree.

## 6. Failure taxonomy (frozen closed set)

`FailureKind ∈ { AdmissionDenied, Timeout, Oom, NetworkDenied, CapabilityViolation,
UnknownTool, HandlerError, RuntimeCrash, WorkerUnreachable, PolicyDenied, Cancelled }`.
UI/recovery map off this enum — no free-form failure strings on the control path (the message
field is human detail only).

## 7. Self-review (challenge)

- *"One event struct becomes a kitchen sink."* → Most fields are `Option`; required minimums per
  stage are enforced (§3). It is a tagged union by `Stage`, not an everything-bag.
- *"Streaming partial_output could leak untrusted content into logs."* → partial_output is
  evidence-wrapped and marked untrusted; audit stores hashes, not raw payloads (existing pattern).
- *"Per-event HMAC is expensive at 10k skills × many events."* → Only **security-relevant** events
  are signed/persisted to audit; high-frequency Streaming events go to the in-memory/telemetry
  projection, not the signed ledger.
- *"Coupling analytics to router weighting creates a feedback loop."* → Analytics informs ranking
  weights offline/batched, not on the hot path; bounded and auditable. ⚠ weighting formula evolvable.
- *"Closed failure set will miss future cases."* → Adding a `FailureKind` is additive + compiler-
  checked everywhere it's matched; better than free strings. Frozen that failures are enumerated,
  not that the enum is final.

**Frozen:** the single `SkillEvent`, closed Stage set, required-fields-per-stage, correlation
model spanning composition, failure enumeration, "audit/UI/telemetry/analytics are projections of
one stream".
**May evolve (⚠):** telemetry exporters, analytics formulas, additive FailureKind/Stage variants,
retention policy.
