# Security Contract (FROZEN — Phase A0)

> INV-6: one safety authority (Rust core). No duplicated security logic across marketplace,
> install, runtime, or UI. Fixes today's dev-HMAC key, client-trusted approval, and unsigned
> manifests.

## 1. Identity, signing, verification (frozen)

- **Skill identity = (slug, publisher)**. Publisher is a stable public key (`did:key`/ed25519).
  A publisher change = a new slug (prevents takeover of a trusted name).
- **Every bundle is signed**: `bundle.sig` = ed25519 over `MANIFEST.sha256` (content hash tree).
- **Install verifies**: signature valid → content hash matches index-pinned hash → publisher
  trusted for the claimed tier. Any failure → refuse. No unsigned install of network/subprocess/
  gpu-capable skills, ever.
- **KRIA-signed** for Verified/bundled skills; **publisher-signed** for Community; **vault-signed**
  for locally-generated skills (extension-contract).

## 2. Trust ladder (frozen)

| Tier | Signer | Isolation floor | Network default | Auto-approve |
|------|--------|-----------------|-----------------|--------------|
| Verified | KRIA key | Docker+seccomp | allowlist if declared | GREEN only |
| Community | publisher key (registered) | gVisor | allowlist + per-domain HITL | no |
| Local | user | gVisor/microVM | none unless HITL | no |
| Generated | vault key | microVM | none unless per-run HITL | no |
| Untrusted | any/unknown | microVM | none | no |

Trust is **assigned by KRIA**, never accepted from `manifest.trust.declared_tier` (advisory only).

## 3. Approval (frozen: server-side, one token)

- Approval is computed in the core: transpile → effective `CapabilityGrant`s → risk → HITL if
  tier/risk requires. The **approval token** binds `hash(slug, version, granted_caps,
  resource_budget, schema_epoch)`.
- The frontend PermissionModal **displays** server-computed grants; it does not decide them.
  Today's ignored `approved_capabilities` field is replaced by this token (capability-contract §2).
- Runtime launch refuses any grant not covered by a valid, matching token.

## 4. Secrets (frozen)

- **No secret ever enters a skill sandbox** by default. Skills receive only their granted,
  scoped inputs. If a skill needs a credential, it is brokered by the core (never injected as
  env), scoped, short-lived, and audited — sourced from the encrypted vault (roadmap Phase 0.1).
- **Audit HMAC key** derives from the vault, per-install, with a `key_id` per entry. The
  hardcoded dev key is removed. `verify_chain` runs at boot + periodically.

## 5. Capability enforcement (single source)

- Capabilities are declared, approved, and **materialized** through the one object
  (capability-contract). There is exactly one enforcement point: the runtime `materialize(grant)`
  + deny-by-default base. No parallel "allow list" lives in the router, UI, or handler.
- Runtime violation of the granted set = hard fail + `SecurityEvent` (event-contract) + optional
  quarantine of the skill.

## 6. Per-domain enforcement (frozen)

| Domain | Rule |
|--------|------|
| Network | default-deny; egress proxy with per-invocation allowlist; `*` requires RED+HITL and signing |
| Filesystem | workspace tmpfs only unless a specific read-only input mount is granted; never host-arbitrary |
| Subprocess | blocked (cap_drop ALL) unless `Subprocess/Execute(bins)` granted; seccomp always on |
| Browser | brokered CDP outside the sandbox; audited; no raw browser in-container |
| GPU | HRA lease + device map only when granted; no ambient device access |
| Remote exec | signed lease + device identity + per-command audit; stricter risk tier by default |
| Generated skills | strictest tier; no network/subprocess without explicit per-run HITL |
| Enterprise | policy overlay can only **tighten** (never loosen) the above defaults |
| Community marketplace | signing + hash pin mandatory; network-capable ⇒ per-domain HITL |

## 7. Audit (frozen: one ledger, tamper-evident)

- Every security-relevant action (install, approve, grant, run, deny, revoke, violation) is an
  HMAC-signed `audit_log` entry with `key_id`, correlation id, and the embedded `CapabilityGrant`.
- Audit is a **projection of the event stream** (event-contract) — not a separate logging system.
- `verify_chain` scheduled; first tampered id surfaced in the Activity UI.

## 8. Self-review (challenge)

- *"Publisher-key trust needs a PKI / revocation."* → Registry index carries publisher keys +
  a revocation list; a revoked publisher's skills are disabled on next sync. Revocation list is
  signed. ⚠ distribution mechanics evolvable; the (slug, publisher) identity rule is frozen.
- *"Vault dependency blocks OpenClaw until Phase 0.1 ships."* → Interim: a per-install random key
  in the OS keyring (not source). Contract (key from a secret store, `key_id` per entry) is what's
  frozen; the store can start as keyring and move to the full vault.
- *"Enterprise overlay could conflict with community defaults."* → Overlay is **tighten-only** by
  contract; conflicts resolve to the stricter rule. No path to loosen.
- *"Brokered browser/secrets add core attack surface."* → Yes, but they move trust from the
  untrusted sandbox to the audited core broker — the correct place. Brokers are least-privilege,
  scoped, and audited.
- *"One enforcement point could become a bottleneck."* → It is a policy check at materialize-time
  (O(grants)), not on the data path. Runtime data flows through the sandbox, not the policy engine.

**Frozen:** (slug, publisher) identity, mandatory signing + hash pin, KRIA-assigned trust ladder,
server-side approval token bound to descriptor+grants, vault-derived audit key with `key_id`,
single capability enforcement point, tighten-only enterprise overlay, audit-as-event-projection.
**May evolve (⚠):** PKI/revocation distribution, signature algorithm parameters, interim key store,
per-tier isolation runtime choice.
