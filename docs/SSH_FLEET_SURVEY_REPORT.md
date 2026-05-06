# SSH Fleet Survey Report

## Executive Summary
Fleet enrollment is implemented in the desktop command layer and persists correctly to a local registry, but live fleet HTTP routes expected by the UI are not present in the server or desktop local API surface. The current UNKNOWN/50% row is a deliberate frontend fallback path triggered when live fleet telemetry is unavailable (source_unwired + no SSE/WS feed), not a measured runtime health score. The advanced control-plane logic in kria-connection-control exists, but there is no verified runtime wiring from UI/server/desktop into that crate in the current code paths.

## Scope, Method, and Missing-Link Policy
- Scope reviewed: ui/src, crates/kria-desktop/src, crates/kria-server/src, crates/kria-connection-control/src, crates/kria-core/src (only where needed for dispatch/wiring truth).
- Method: static code audit only (no runtime execution in this survey).
- Rule applied: no assumptions; unresolved runtime facts are marked as Missing Link.

## Completed Checklist (Verified Code)
- [Verified] Add Soldier flow exists and is wired through Tauri invoke:
  - UI calls invoke("register_new_target") via app store.
  - Desktop exposes register_new_target command and validates SSH tools, host key, auth, bootstrap.
- [Verified] Enrollment persistence exists:
  - Desktop saves records to data_dir/fleet/targets.json.
  - Desktop returns registry_path in RegisterNewTargetResponse.
- [Verified] Runtime admission exists (desktop path):
  - register_new_target calls admit_enrolled_target_to_fleet_runtime.
  - Target is admitted into TargetPool with QemuSshEnvironment.
- [Verified] Fleet status fallback path exists:
  - get_ironclad_status includes enrolled_targets and source_unwired.
  - Frontend renders enrolled_targets fallback when live heartbeat targets are empty.
- [Verified] UI expects fleet HTTP endpoints:
  - /api/fleet/events (SSE)
  - /api/fleet/terminal (WS)
  - /api/fleet/leases/:lease_id/heartbeat
  - /api/fleet/docker-evals
- [Verified] Server route surface does not include /api/fleet/*:
  - kria-server api_routes exposes only /api/health, /api/chat, /api/sessions, /api/models, /api/settings.
  - kria-server ws_routes exposes only /ws (chat/ping style stub), not /api/fleet/terminal.
- [Verified] Desktop local API bridge does not include /api/fleet/*:
  - local bridge routes only /api/health and /api/chat.
- [Verified] kria-connection-control logic is substantial but isolated:
  - ConnectionManager, signer, FleetStore/Connector traits, docker eval path, trust rotation, terminal gap handling, HA promotion exist.
  - No runtime crate dependency found from kria-desktop or kria-server to kria-connection-control.
- [Verified] Security signer truth split:
  - kria-connection-control signer is implemented and invoked inside that crate's manager dispatch path.
  - Active desktop remote SSH execution path uses kria-core QemuSshEnvironment signed execution envelope (separate implementation), not kria-connection-control signer wiring.

## Gap Matrix

| Source Action | Backend Route | Status |
|---|---|---|
| Add Soldier (UI modal submit) | Tauri command: register_new_target | Implemented |
| Fetch Ironclad status snapshot | Tauri command: get_ironclad_status | Implemented |
| Subscribe fleet live telemetry (SSE) | GET /api/fleet/events | Missing Route |
| Attach focused terminal stream (WS) | WS /api/fleet/terminal?target_id=...&lease_id=... | Missing Route |
| Lease heartbeat renewal | POST /api/fleet/leases/:lease_id/heartbeat | Missing Route |
| Run Docker Evals button | POST /api/fleet/docker-evals | Missing Route |
| Generic server websocket | /ws (chat stub) | Wired but Broken (contract mismatch for fleet terminal stream) |
| Control-plane manager dispatch/signing path | ConnectionManager::spawn + manager handle wiring from app runtime | Missing Route (runtime integration missing) |

## Root Cause Analysis: UNKNOWN / 50% Health

### What the UI is doing
- Frontend fleet matrix prefers live targets from useFleetHeartbeat().
- If live targets are empty, it falls back to enrolled_targets from ironclad status.
- In that fallback, when fleet.source_unwired is true:
  - state is forced to unknown.
  - healthScore is forced to 0.5.
  - taintReason says live telemetry is unavailable.

### Why 50% appears
- FleetMatrix healthPct computes percent from healthScore and failure rate.
- With fallback values healthScore=0.5 and recentFailureRate=0, displayed health is 50%.
- This is placeholder health, not measured VM health.

### Why source_unwired stays true in current topology
- get_ironclad_status marks source_unwired when orchestrator latest_pool_packet is None.
- Live fleet SSE/WS/heartbeat endpoints expected by frontend are missing in backend route surfaces.
- Commander base URL discovery depends on fields that are not guaranteed in current status/config payloads.

## Security and Trust Wiring Verdict

### Verdict
- Statement: "Signed envelopes are used during SSH command dispatch today."
- Accurate only with qualification:
  - TRUE for kria-core QemuSshEnvironment path (signed execution envelope with HMAC).
  - FALSE for kria-connection-control signer path in current runtime wiring (no verified integration from desktop/server crates).

### Evidence
- kria-connection-control manager dispatch_with_retry signs SignedEnvelopeInput and verifies inbound envelopes.
- kria-desktop and kria-server Cargo.toml do not include kria-connection-control dependency.
- No usage sites found in runtime crates for ConnectionManager::spawn from kria-connection-control.
- kria-core remote_qemu execute_command builds and signs execution envelopes directly.

## Persistence Truth Test
- Enrollment persistence: Present (JSON registry file under data_dir/fleet/targets.json).
- Fleet control-plane relational persistence (target_identity, lease_sessions, docker_eval_runs, etc.): Schema file exists in kria-connection-control/sql.
- Runtime migration/wiring for that SQL schema: Missing Link (no invocation path verified from current server/desktop runtime code).

## Technical Debt and Risk Register
- Missing fleet API contract implementation: UI contract and backend route surface diverge for SSE/WS/heartbeat/docker-evals.
- Split control-plane architectures: desktop TargetPool/Qemu path and kria-connection-control path both exist without a proven unified runtime selection strategy.
- Security-path ambiguity risk: two different envelope/signing systems increase audit and operational complexity unless explicitly documented and selected.
- SQL migration debt: 0001_fleet_orchestration.sql contains duplicated schema blocks in a single file; even if not wired today, this is high-risk once migration execution is introduced.
- Fallback masking risk: UNKNOWN/50% can be misread as real health by operators.

## Missing Link Ledger (Explicit)
- Missing Link: Runtime mode for the user's current failing session (desktop-only, server-only, or mixed) was not directly executed in this survey.
- Missing Link: External reverse proxy or sidecar that could provide /api/fleet/* outside the audited crates was not found in reviewed code, but not runtime-probed.
- Missing Link: Whether custom settings include a valid commander_base_url in the user's active config at runtime (static code confirms lookup paths, not live value).

## Final Auditor Conclusion
Inventory enrollment is working in the desktop layer, but live fleet transport and control endpoints expected by the frontend are not implemented in the active backend route surfaces, producing fallback UNKNOWN/50% behavior. The issue is primarily a code-wiring/contract gap, not evidence of SSH connectivity failure in enrollment itself. kria-connection-control is currently an implemented but non-integrated control-plane candidate in this codebase snapshot.