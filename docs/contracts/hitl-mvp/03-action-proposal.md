# KRIA Action Proposal Spec

**Document status:** MVP execution-intent contract  
**Purpose:** Define immutable execution intent before policy, HITL, leases, or tools run.

---

## 1. Purpose

`ActionProposal` is the canonical description of what KRIA intends to do.

It prevents this failure:

```text
user approves action A
runtime executes mutated action B
```

---

## 2. Required Fields

| Field | Requirement |
|---|---|
| `workflow_id` | Current workflow lineage. |
| `attempt_id` | Current attempt/replan generation. |
| `stage_id` | Stage requesting execution. |
| `tool_name` | Canonical tool name. |
| `parameters` | Canonical JSON after validation and normalization. |
| `target` | Bound execution target. |
| `action_hash` | Hash of tool, params, target, workflow attempt, and risk-relevant metadata. |
| `target_hash` | Hash of target identity and execution boundary. |
| `created_at` | UTC timestamp. |

---

## 3. Hash Inputs

`action_hash` must include:

- tool name,
- normalized parameters,
- target hash,
- workflow ID,
- attempt ID,
- stage ID,
- privilege boundary,
- rollbackability class,
- declared affected resources.

`target_hash` must include:

- target kind,
- target ID,
- workspace/session ID if relevant,
- browser profile/account if relevant,
- VM/container ID if relevant,
- filesystem canonical path if relevant,
- external owner token hash if relevant.

---

## 4. Mutation Rule

Any mutation creates a new `ActionProposal`.

```text
same proposal ID + changed params = forbidden
changed target = new target_hash
changed risk-relevant field = new action_hash
```

---

## 5. Canonicalization

Before hashing:

- sort JSON object keys,
- normalize paths,
- reject unresolved symlinks for filesystem writes,
- normalize target IDs,
- remove non-semantic UI fields,
- preserve risk-relevant defaults inserted by runtime.

---

## 6. MVP Tests

- parameter mutation changes `action_hash`,
- target mutation changes `target_hash`,
- JSON key order does not change hash,
- path alias/symlink cannot hide target mutation,
- approval for old hash cannot execute new hash.

