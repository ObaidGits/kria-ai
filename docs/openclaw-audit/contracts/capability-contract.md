# Capability Contract (FROZEN — Phase A0)

> INV-2: **one** capability object flows through every layer. No layer invents its own shape.
> Replaces today's `SkillCapabilities` (bool flags) + `OpenClawNetworkPolicy` (separate enum),
> which are two representations of the same concern and cause drift.

## 1. The single object

```rust
// Conceptual (frozen shape, not an implementation).
struct Capability {
    kind:  CapabilityKind,     // what class of power
    mode:  CapabilityMode,     // how much of it
    scope: CapabilityScope,    // bounded to exactly what
}

enum CapabilityKind { Filesystem, Network, Subprocess, Browser, Gpu, Clipboard, Device }
enum CapabilityMode { ReadOnly, ReadWrite, Egress, Execute, Use }
enum CapabilityScope {
    Workspace,                         // ephemeral per-invocation dir (default)
    InputMount { id: String },         // a specific, user-provided read-only input
    Domains(Vec<String>),              // network allowlist (never "*" without RED+HITL)
    Binaries(Vec<String>),             // subprocess allowlist
    None,
}
```

**Rule:** a capability is meaningless without a **scope**. There is no "network: true" — only
"network egress to [domains]". This kills the current cosmetic-capability problem where declared
powers were never bounded or materialized.

## 2. The grant object (what actually carries state through the lifecycle)

```rust
struct CapabilityGrant {
    capability: Capability,      // the requested/approved power
    granted:    bool,            // approved & materializable
    source:     GrantSource,     // Manifest | UserApproval | PolicyDefault | Generated
    approved_by: Option<Approver>,   // who/what approved (HITL id, policy rule, vault-sign)
    materialization: Materialization, // how the runtime realizes it (mount/proxy/device/none)
    granted_at: DateTime, expires_at: Option<DateTime>,
}
```

`CapabilityGrant` is the object that is serialized into the approval token, the runtime launch
spec, and the audit record. **The same struct, not three copies.**

## 3. Lifecycle (the same object at every stage)

```text
DECLARE      manifest.toml [[capabilities]]           → Vec<Capability> (requested)
   ▼
REVIEW       KRIA transpile → risk = classify(caps)    → risk level assigned by KRIA (not author)
   ▼
APPROVAL     HITL / policy → grant decision            → Vec<CapabilityGrant{granted, approved_by}>
   ▼          (approval token = hash(descriptor) + granted set + resource budget)
MATERIALIZE  runtime realizes grants                   → mounts / egress-proxy allowlist / device
   ▼          (deny-by-default base; grants are additive, minimal, per-invocation)
RUNTIME       skill runs within exactly the granted set → violations = hard fail + SecurityEvent
   ▼
AUDIT        every grant + use recorded (event-contract)→ CapabilityGrant embedded in audit entry
   ▼
REVOCATION   user/policy revokes                        → grant.granted=false; re-materialize denies
   ▼
HISTORY      immutable audit trail per (slug, grant)
   ▼
ANALYTICS    aggregate: which caps used, denied, abused → informs trust + recommendations
```

## 4. Risk mapping (KRIA-owned, frozen)

Risk is a **pure function of the granted capability set**, computed by the core — never taken
from the manifest. (Preserves today's correct `classify_risk` principle, generalized.)

| Granted capabilities | Risk |
|----------------------|------|
| read-only workspace / input only | GREEN |
| network egress (allowlist), browser, gpu-use | YELLOW |
| filesystem read_write, subprocess execute | RED |
| network `*`, device, host-scope (never allowed without…) | BLACK (blocked / explicit RED+HITL) |

The risk feeds `PolicyEngine` + `HitlGateway` exactly as tool `default_tier` does today, so the
existing safety authority governs oc_* with no new safety path (INV-6).

## 5. Materialization matrix (declaration ↔ grant ↔ realization are one chain)

| Capability | Grant realized as | Default when not granted |
|------------|-------------------|--------------------------|
| Filesystem/Workspace | tmpfs `/workspace` (always present) | n/a |
| Filesystem/InputMount | read-only bind of the approved input | absent |
| Network/Egress(domains) | egress-proxy allowlist rule | `network=none` |
| Subprocess/Execute(bins) | seccomp/exec allowlist | blocked (cap_drop ALL) |
| Browser | brokered CDP endpoint (out-of-container) | absent |
| Gpu/Use | HRA GPU lease + device map | no device |
| Clipboard/Device | brokered, audited bridge | absent |

## 6. Re-approval rules (tie to package-contract §4)

- **Widening** any granted capability (new kind, broader mode, larger scope) → new approval token
  required; old token invalid (bound to descriptor+grant hash).
- **Narrowing** → allowed silently, audited.
- Revocation is immediate and takes effect at next materialization (in-flight runs are cancelled
  if the revoked grant is in use).

## 7. Self-review (challenge)

- *"Enum-based caps won't cover future powers (e.g., audio capture, camera)."* → `CapabilityKind`
  is the one place new powers are added; adding a variant is additive and forces a match update
  everywhere (compiler-enforced completeness). Scope+mode generalize. This is a *feature* — new
  powers cannot sneak in un-scoped.
- *"Grant materialization couples capability to runtime."* → It couples to the **runtime
  interface** (execution-contract), not a specific backend. Each `SkillRuntime` implements a
  `materialize(grant)` hook; Docker/WASM/microVM realize the same grant differently. One object,
  many realizations — no drift.
- *"Approval token binding to descriptor hash is brittle across cosmetic edits."* → Cosmetic
  fields (name/desc) are excluded from the approval hash; only (slug, version, granted caps,
  resource budget, schema epoch) are hashed. Cosmetic re-index doesn't invalidate approval.
- *"Generated skills have no human approver."* → `GrantSource::Generated` + vault-signed policy
  approver; generated skills are forced to the strictest tier (no network/subprocess unless the
  user approves per-run). Covered by security + extension contracts.

**Frozen:** the single `Capability`/`CapabilityGrant` object, scope-mandatory rule, KRIA-owned
risk function, the declare→…→analytics lifecycle, re-approval on widening.
**May evolve (⚠):** the set of `CapabilityKind` variants (additive), materialization mechanics
per backend, expiry/lease durations.
