# OpenClaw — Security Review

> Scope: trust model, secrets, audit integrity, supply chain, injection, install-time gates.
> Sandbox/isolation escape analysis is in `sandbox-review.md`.

## 1. Threat model

Adversaries: (a) a malicious/compromised Community skill author, (b) a malicious skill
*manifest* served from a compromised registry mirror, (c) prompt-injection embedded in skill
output, (d) a local attacker tampering with the audit ledger, (e) a skill attempting resource
exhaustion or escape. KRIA's stance: **never trust the skill or its author; the Rust core is
the sole authority.** The design honours this stance in principle; several controls are
unfinished.

## 2. Findings

### SEC-1 (Critical) — Hardcoded audit HMAC key, duplicated
`init.rs`: `const DEV_HMAC_KEY: &[u8] = b"kria-openclaw-dev-audit-key-0001";` and the same
literal is re-typed in `commands/openclaw.rs`. Consequences:
- Any party who reads the source can forge or "repair" audit entries → `verify_chain` becomes
  security theatre.
- Tamper-evidence, the entire point of an HMAC ledger, is void.
**Fix:** derive a per-install key from the encrypted secrets vault (roadmap Phase 0.1
`keyring`/`aes-gcm`), store only in the vault, never in source. Single source, injected at
boot. Rotate on vault rotation; record key-id in entries.

### SEC-2 (High) — No manifest signing or version pinning
`clawhub.rs` validates HTTPS + host allowlist + 64 KiB cap and the transpiler discards prose —
good. But there is **no signature check, no content hash pin**, and install writes
`version: "remote"` literally. A compromised `raw.githubusercontent.com` path or MITM on a
self-hosted mirror can swap a manifest.
**Fix:** require a detached signature (minisign/ed25519) per manifest, pin `sha256` from the
index entry, verify before transpile; store the verified hash + real semver in the descriptor.

### SEC-3 (High) — Install HITL is client-trusted
`RemoteInstallRequest.approved_capabilities` is `#[allow(dead_code)]`; the backend never
validates the user-approved set against the transpiled descriptor. The PermissionModal is the
only gate and it lives in the frontend.
**Fix:** enforce approval server-side: transpile → compute effective capabilities/risk →
require an authenticated HITL approval token that matches the descriptor hash before
`registry.install`. Reject if approved set ⊂ required set is violated.

### SEC-4 (Medium) — Capabilities are declared, not enforced
Every container is the same locked profile regardless of declared caps (see
`sandbox-review.md`). Security-positive (nothing is granted) but **misleading**: the modal
tells users a skill "needs filesystem write / network" yet the sandbox grants neither, so
either the skill silently fails or a future change that *does* grant caps could bypass review.
**Fix:** make capability grants explicit and materialized (mounts, egress allowlist), gated by
the approved set, so declaration ↔ grant ↔ audit are the same object.

### SEC-5 (Medium) — Network egress unimplemented
`egress_proxy_port` (default 18800) and `DomainAllowlist` are configured but no proxy exists;
containers are `network=none`. Today this is safe-by-omission, but the moment network is
enabled without the proxy, `Unrestricted` becomes full egress.
**Fix:** implement an egress proxy (per-invocation allowlist) *before* any network grant path
ships; default-deny; log all egress to audit.

### SEC-6 (Medium) — Audit not centralized, `verify_chain` never run
Audit lives in a separate `skills.db`; there is no scheduled integrity check and no UI. A
tamper would go unnoticed.
**Fix:** schedule `verify_chain` at boot + periodically; surface results in the activity log;
alert on first tampered id.

### SEC-7 (Low) — Trust tier is a label, network allowed for Community
`TrustTier::Community.allows_network() == true` and config `community_allows_network=true` by
default. Combined with SEC-2 (no signing), a community skill could request broad domains.
**Fix:** default community network to allowlist-only + explicit HITL per domain; require
signing for any network-capable community skill.

## 3. Controls that are correct (keep)

- YAML-only transpile; prose (and embedded injection) discarded; name/description validated;
  **risk assigned by KRIA, never the author** (`capabilities.classify_risk()`).
- `EvidenceWrapper` marks all skill output `trust="untrusted"`, XML-escapes, size-caps, and
  prevents wrapper-escape (tested). Strong prompt-injection boundary.
- Image hardening: `--ignore-scripts`, npm/apt removed from final image, non-root `USER node`.
- Remote skills forced to `TrustTier::Community` on install regardless of self-claim.

## 4. Priority order

1. SEC-1 (key from vault) — blocks any trust claim.
2. SEC-3 + SEC-4 (server-side approval + real capability grants) — make the permission model real.
3. SEC-2 (signing + pin) — before wider ClawHub catalog.
4. SEC-5 (egress proxy) — before any network grant.
5. SEC-6, SEC-7 — hardening.
