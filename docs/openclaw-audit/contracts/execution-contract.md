# Execution Contract (FROZEN — Phase A0)

> INV-4: every backend implements the **same** `SkillRuntime` lifecycle. The core never
> special-cases Docker/WASM/Firecracker/Remote/Cloud/GPU. Backend is chosen from bundle metadata
> + policy, not from call-site branches.

## 1. The one interface

```rust
#[async_trait]
trait SkillRuntime: Send + Sync {
    fn kind(&self) -> RuntimeKind;                 // Docker | Wasm | Firecracker | Remote | Cloud | Gpu
    async fn prepare(&self, spec: &LaunchSpec)  -> Result<Prepared>;   // image/module ready, deps present
    async fn admit(&self, spec: &LaunchSpec)    -> Result<Lease>;      // via HRA (resource-contract)
    async fn launch(&self, p: Prepared, l: Lease) -> Result<Instance>; // materialize grants, start
    fn  monitor(&self, i: &Instance) -> EventStream;                   // SkillEvent stream (event-contract)
    async fn call(&self, i: &Instance, req: SkillRequest) -> Result<SkillResponse>; // MCP or adapter
    async fn cancel(&self, i: &Instance) -> Result<()>;                // teardown + release lease
    async fn recover(&self, i: &Instance, f: Failure) -> Recovery;     // retry | rollback | fail
    async fn cleanup(&self, i: Instance) -> Result<()>;                // destroy; free grants
    async fn recycle(&self, i: Instance) -> Result<Option<Warm>>;      // pool reuse if permitted
}
```

- `LaunchSpec` = { bundle descriptor, `Vec<CapabilityGrant>`, `ResourceRequest`, correlation id }.
- One trait, one lifecycle. A new backend = a new impl; **no change** to the loop, router,
  registry, HRA, audit, or UI.

## 2. Lifecycle (frozen order)

```text
PREPARE → ADMIT → LAUNCH → MONITOR → (CALL…)* → CANCEL? → RECOVER? → CLEANUP → RECYCLE
```

- **Prepare:** ensure the runnable artifact exists (image pulled/built, WASM module loaded,
  microVM rootfs ready, remote worker reachable). Idempotent. No resources held.
- **Admit:** request an HRA lease sized by `ResourceRequest` (resource-contract). Backpressure/
  queue here — never allocate outside HRA.
- **Launch:** materialize `CapabilityGrant`s (capability-contract §5), start instance, emit
  `Running`. Deny-by-default base always applied first.
- **Monitor:** single `SkillEvent` stream (event-contract); backend-native signals (Docker
  events, WASM traps, remote heartbeats) normalize into it.
- **Call:** MCP `tools/call` (or adapter for non-MCP). Streaming responses supported.
- **Cancel:** cooperative then forced; **must** tear down instance and release lease (fixes
  today's non-propagated cancellation). Wired to `global_halt`.
- **Recover:** typed `Failure` → `Recovery{Retry(bounded) | Rollback | Fail}`. No infinite retry.
- **Cleanup:** destroy instance, free grants, remove mounts/proxy rules.
- **Recycle:** trust/tier-dependent — Verified may return a warm instance (workspace wiped);
  Untrusted always destroyed.

## 3. Backend selection (data-driven, frozen)

```text
runtime.kind (manifest)  ∩  host capability (docker? wasmtime? kvm? fleet?)  ∩  trust policy
        │
        ▼
   RuntimeKind resolved  → matching SkillRuntime impl from a registry keyed by RuntimeKind
```

- Manifest states the *required* runtime; policy may *upgrade* isolation (e.g., force microVM for
  Untrusted even if manifest says container). Never downgrade below the trust tier's floor.
- If the required runtime is unavailable on the host, the skill is **unavailable** (clear UX), not
  silently downgraded to a weaker sandbox.

## 4. Backend obligations (per RuntimeKind — same contract, different realization)

| RuntimeKind | Isolation floor | Grants realized via | Recycle |
|-------------|-----------------|---------------------|---------|
| Wasm | in-process capability sandbox (no ambient authority) | host-function allowlist | yes (cheap) |
| Docker | readonly rootfs + cap_drop ALL + seccomp + pids/ulimit | mounts + egress proxy + device | Verified only |
| Firecracker | microVM (own kernel) | virtio mounts + tap egress + vfio gpu | no (destroy) |
| Remote | worker-side runtime + signed lease | remote materialization; audited | worker policy |
| Cloud | provider sandbox behind KRIA broker | provider IAM mapped from grants | provider policy |
| Gpu | Docker/microVM + HRA GPU lease + device map | device + egress as needed | HRA-governed |

All satisfy the **same** `SkillRuntime` trait; the core treats them identically.

## 5. Transport (frozen)

- Default: **MCP** (JSON-RPC, Content-Length framed) over the backend's stdio/socket. `bridge.rs`
  is the reference client. Non-MCP skills get an adapter that presents the same `call` surface.
- Streaming: `call` returns a stream of MCP content blocks → normalized into `SkillEvent::Streaming`.

## 6. Idempotency, timeouts, correlation (frozen)

- Every stage carries the **correlation id** (event-contract). Timeout is the skill's
  `resource.timeout_secs`, propagated end-to-end (fixes today's fixed-30s mismatch).
- `prepare` and `cleanup` are idempotent so crash-recovery can re-run them safely.

## 7. Self-review (challenge)

- *"WASM and microVM lifecycles differ too much for one trait."* → The trait models *phases*, not
  mechanisms. A phase a backend doesn't need (e.g., WASM `recycle` returning `None`, microVM no
  warm pool) is a no-op/`None`. The *contract* holds; realization varies. Verified across the 6
  backends in the table above.
- *"Remote/Cloud break the 'destroy-per-invocation' isolation assumption."* → Isolation is a
  per-backend *floor*, not a global rule. Remote workers enforce their own floor + signed lease;
  the core trusts the lease, not the worker's memory.
- *"Backend registry could become a god-object."* → It is a thin map `RuntimeKind → impl`,
  populated at boot from host capabilities. No logic, just dispatch (INV-4).
- *"Policy upgrading isolation could surprise skill authors."* → Documented and one-directional
  (only stronger). Authors declare a floor; the platform may raise it. Safe by construction.
- *"Does this couple execution to MCP?"* → No: MCP is the default transport; the adapter seam lets
  non-MCP runtimes conform without changing the trait.

**Frozen:** the `SkillRuntime` trait + lifecycle order, backend-by-metadata selection, isolation
floors per RuntimeKind, cancellation-must-release-lease, correlation-id + end-to-end timeout,
MCP-default-with-adapter.
**May evolve (⚠):** the concrete set of RuntimeKinds (additive), pooling/recycle heuristics,
transport optimizations.
