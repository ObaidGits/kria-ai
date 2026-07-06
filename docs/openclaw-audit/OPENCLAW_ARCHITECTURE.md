# OpenClaw Architecture (Frozen after A9)

OpenClaw is KRIA's sandboxed skill substrate. KRIA's Rust core is the sole planner,
safety authority and resource arbiter. Skills execute in network-isolated Docker
containers and are exposed as `oc_*` tools.

## Layered view

```
┌───────────────────────────────────────────────────────────────────────┐
│ A9  Autonomous Skill Generation System (generation/)                    │
│     Goal → decide(reuse|generate) → design → codegen → validate →       │
│     sandbox → repair → quality → approval → INSTALL (frozen path)       │
└───────────────┬───────────────────────────────────────────────────────┘
                │ produces ordinary .ocskill bundles
┌───────────────▼───────────────────────────────────────────────────────┐
│ A8  Platform / ClawHub (platform/)                                      │
│     RepositoryManager · PublisherRegistry · TrustFramework ·            │
│     Marketplace · UpdateEngine · SyncEngine · PlatformMetrics           │
└───────────────┬───────────────────────────────────────────────────────┘
┌───────────────▼───────────────────────────────────────────────────────┐
│ A2/A3  Bundle system (bundle/) + Capability system (capability.rs)      │
│     Manifest · verify(hash-tree+ed25519) · BundleInstaller · deps ·     │
│     version(semver) · materialize · approval · revocation               │
└───────────────┬───────────────────────────────────────────────────────┘
┌───────────────▼───────────────────────────────────────────────────────┐
│ A5  Production Skill Registry (registry.rs)                             │
│     Discovered→Verified→Installed→Enabled→Disabled→Deprecated→Removed→  │
│     Broken→Recovering · health · statistics · events · dependency graph │
└───────────────┬───────────────────────────────────────────────────────┘
┌───────────────▼───────────────────────────────────────────────────────┐
│ A6  Semantic Router (semantic_router.rs)                                │
│     Registry-driven routing; never scans filesystem                     │
└───────────────┬───────────────────────────────────────────────────────┘
┌───────────────▼───────────────────────────────────────────────────────┐
│ A7  Generic Execution Engine (crate::execution)                         │
│     Planner · Graph · Scheduler · Context · Recovery · Metrics ·        │
│     Events · ExecutorRegistry → OpenClawExecutor (first executor)       │
└───────────────┬───────────────────────────────────────────────────────┘
┌───────────────▼───────────────────────────────────────────────────────┐
│ A4  Runtime Manager (runtime_manager.rs) + A1 Runtime (runtime/)        │
│     Lifecycle state machine · warm pool · health · recovery ·           │
│     scheduler · HRA integration · cancellation                          │
└───────────────┬───────────────────────────────────────────────────────┘
                ▼
        Docker containers (network-isolated, capability-materialized)
```

## Module map

| Module | Responsibility |
|---|---|
| `types.rs` | Core domain types (`SkillDescriptor`, `TrustTier`, `ResourceClass`). |
| `runtime/` | `SkillRuntime` trait + `DockerRuntime`. |
| `runtime_manager.rs` | Authoritative container lifecycle, warm pool, health, recovery. |
| `pool.rs` | Backwards-compatible `ContainerPool` delegating to `RuntimeManager`. |
| `registry.rs` | `ProductionSkillRegistry` (single source of truth for skills). |
| `semantic_router.rs` | Registry-driven skill selection. |
| `bundle/` | `.ocskill` manifest, verify, installer, deps, version. |
| `capability.rs` | `Capability{kind,mode,scope}` + risk classification. |
| `platform/` | Repository, publisher, trust, marketplace, updates, sync, metrics. |
| `generation/` | ASGS: decision, requirements, designer, codegen, validator, quality, budget, approval, sandbox, repair loop, pipeline, LLM generator. |
| `crate::execution` | Generic execution engine; OpenClaw is the first `Executor`. |

## Cross-cutting rules (frozen)

1. KRIA is the sole planner; the execution planner has zero backend-specific logic.
2. Native KRIA tools take precedence over `oc_*` skills.
3. Every skill is a `.ocskill` bundle; no alternate skill representation.
4. All dangerous operations flow through the capability + trust + approval layers.
5. Optional services (Docker, ComfyUI, MCP) may be unavailable — never mandatory.
