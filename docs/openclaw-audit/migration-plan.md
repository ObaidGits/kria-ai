# OpenClaw — Migration Plan

> How to move from the current state to the target without breaking the app, keeping
> `openclaw.enabled = false` as the safety switch throughout. No behaviour change lands for
> users until a phase's exit test passes.

## 0. Invariants during migration

- OpenClaw stays **disabled by default**; all work is behind the existing config gate.
- Native + MCP tools are unaffected (oc_* is additive in `ToolRegistry`).
- Safety authority stays in the Rust core (PolicyEngine + HitlGateway) — never weakened.
- Each phase is independently revertible; no destructive data migration.

## 1. Data / schema migrations

- `installed_skills` and `audit_log` already exist (WAL). Additions are **additive columns**,
  applied via `CREATE TABLE IF NOT EXISTS` + `ALTER TABLE ADD COLUMN` guards:
  - `installed_skills`: `version TEXT`, `manifest_sha256 TEXT`, `signature TEXT`,
    `approved_caps_json TEXT`.
  - `audit_log`: `cpu_ms INTEGER`, `peak_mem_bytes INTEGER`, `gpu_ms INTEGER`, `exit_kind TEXT`,
    `key_id TEXT`.
- **Key migration (SEC-1):** on first boot after B1, if legacy dev-key entries exist, mark them
  `key_id = "legacy-dev"` and re-sign nothing (history is immutable); new entries use the
  vault key with a new `key_id`. `verify_chain` verifies per-entry against its `key_id`.

## 2. Sequenced cutover

### Step 1 — Exec path (A1), no user-visible change
Swap `docker attach` CLI for bollard attach/exec behind the same `execute_in_container`
signature. Gate with a temporary `openclaw.exec_backend = "bollard"` flag; default to it once
the handshake test passes; delete the CLI path.

### Step 2 — Skill package contract (A2), define once
Introduce `skill-bundle` format (`manifest.toml` + handler + `schema.json` + `SKILL.md` +
`bundle.sig`). Bake 2 real skills into the image. Keep the transpiler as the descriptor source;
add a bundle loader in the substrate. **This is the contract everything else depends on — land
it deliberately.**

### Step 3 — Hot-register (A3)
Make install/uninstall/toggle call `register_into_tool_registry` + `tool_index.rebuild`. Purely
additive; no schema change.

### Step 4 — Vault key (B1)
Add vault-backed key provider with fallback to legacy `key_id` for verification only. Remove
both hardcoded literals in the same change. `verify_chain` gains per-`key_id` verification.

### Step 5 — Capability grants + egress proxy (B3)
Introduce a per-invocation grant layer: workspace mount (already tmpfs), optional read-only
input mount, egress proxy sidecar with per-invocation allowlist. Default-deny preserved; grants
only for approved caps. Roll out to Verified skills first, then Community.

### Step 6 — Server-side HITL (B2)
Change `clawhub_install_skill` to compute effective caps and require an approval token bound to
the descriptor hash. Update `PermissionModal` to display server-computed caps. Old client
`approved_capabilities` field becomes advisory then removed.

### Step 7 — HRA admission + cancellation (B5)
Register OpenClaw HRA consumer; checkout requests a lease; implement `execute_with_context`
holding the handle; cancellation/`global_halt` tears down container + releases lease. Shadow
mode first (admit-always, log decisions), then enforce — mirroring the existing HRA cutover
pattern used for STT/TTS/vision.

### Step 8 — Sandbox hardening (B4)
Add `pids_limit`, ulimits, seccomp profile, and wire `events.rs` subscriber into the pool.
Feature-flag the seccomp profile; validate bundled skills still run, then enable by default.

### Step 9 — Signing (B6)
Add signature + hash verification to install; refuse unsigned network-capable community skills;
implement update-with-diff. Bundled skills are KRIA-signed.

### Step 10 — Router unification (C1)
Decide Option A (unify on tool_index + OpenClaw admission pre-stage). Implement the pre-stage;
move BM25/intent-pre-filter logic there if useful; **delete `resolver.rs`** (or the redundant
half) so only one router remains. Validate with a scale test (200 synthetic skills).

### Step 11 — Streaming, Activity UI, telemetry, tiered runtimes (C2–C5)
Additive; land incrementally.

## 3. Dead-code disposition

- `openclaw/resolver.rs` — either promote (Step 10 Option B) or delete (Option A). **Do not
  leave dormant** — it misleads future readers and hides the real routing path.
- `openclaw/events.rs` — wire into pool (Step 8). Currently dead.
- `RemoteInstallRequest.approved_capabilities` — activate (Step 6) or remove.

## 4. Verification per phase

Each step ships with: unit tests (transpile/pool/audit already have patterns to extend),
one integration test that runs a bundled skill end-to-end, and a manual smoke via the Settings
→ OpenClaw panel. `cargo build -p kria-core -p kria-desktop` + `cargo test -p kria-core` green
before enabling any flag by default.

## 5. Rollback

Every step is behind a flag or is additive. Emergency rollback = set `openclaw.enabled=false`;
the rest of KRIA is unaffected because oc_* tools simply are not registered.
