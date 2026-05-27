# KRIA Action Proposal Contract

**Document status:** Implementation-bound execution-intent contract
**Last updated:** 2026-05-27
**Primary code:** `crates/kria-core/src/agent/collaborative_decision.rs`, `execution_gate.rs`

---

## 1. Purpose

`ActionProposal` is the immutable description of the side effect KRIA is preparing to run.

It prevents this failure:

```text
user approves action A
runtime executes mutated action B
```

The proposal is created before durable decision creation, policy approval, resource leasing, resume execution, or continuation re-entry.

---

## 2. Required Fields

| Field | Current requirement |
|---|---|
| `workflow_id` | Workflow/session lineage. Current live gate uses `session_id`. |
| `attempt_id` | Attempt generation. Current live gate uses `active-attempt`. |
| `stage_id` | Stage/tool requesting execution. |
| `tool_name` | Canonical registry tool name. |
| `parameters` | JSON parameters after planner/tool normalization. |
| `target` | `TargetBinding` from execution authority. |
| `tool_schema_version` | Current tool schema version at proposal creation. |
| `tool_registry_version` | Current tool registry version at proposal creation. |
| `action_hash` | Hash over workflow, attempt, stage, tool, parameters, target hash, schema version, and registry version. |
| `target_hash` | Hash over the target binding. |
| `requested_by` | Actor creating the proposal. |
| `created_at` | RFC3339 timestamp string. |

---

## 3. Hash Inputs

`compute_action_hash` currently includes:

- `workflow_id`,
- `attempt_id`,
- `stage_id`,
- `tool_name`,
- canonical serialized `parameters`,
- `target_hash`,
- `tool_schema_version`,
- `tool_registry_version`.

`compute_target_hash` currently includes canonical serialized `TargetBinding`.

Do not document rollbackability, affected resources, or verifier evidence as action-hash inputs unless the implementation changes to include them.

---

## 4. Target Binding Inputs

`TargetBinding` includes:

- target kind,
- target ID,
- optional workspace ID,
- optional session ID,
- optional execution boundary,
- metadata.

For live execution-gate proposals, target bindings are derived from:

- authorized execution target,
- ambiguous execution target,
- blocked execution target.

This allows blocked or ambiguous actions to still produce explainable durable decisions.

---

## 5. Mutation Rule

Any mutation creates a new proposal:

```text
changed parameters -> new action_hash
changed target binding -> new target_hash and new action_hash
changed stage/tool/schema/registry version -> new action_hash
changed workflow or attempt -> new action_hash
```

A resolved decision authorizes only the persisted proposal hash and target hash it was created for.

---

## 6. Resume Rule

Before executing a resolved decision, `ResumeExecutor` and `ExecutionGate::revalidate_resume` must:

- load the persisted `ActionProposal`,
- recompute `target_hash`,
- recompute `action_hash`,
- verify current tool schema and registry versions,
- rerun readiness, preflight, policy, and lease requirements,
- reject execution if any value changed.

---

## 7. Required Tests

- parameter mutation changes `action_hash`,
- target mutation changes `target_hash`,
- target mutation also invalidates resume,
- stale tool schema or registry version invalidates resume,
- stale decision version is rejected,
- old approval cannot execute a new action hash,
- JSONL replay preserves proposal hashes.
