//! OpenClaw Skill Substrate integration.
//!
//! Provides headless, network-isolated execution of OpenClaw community skills
//! as sandboxed tool invocations within KRIA's sovereign orchestration boundary.
//!
//! # Architecture
//!
//! KRIA's Rust core remains the sole planner, safety authority, and resource arbiter.
//! OpenClaw skills are exposed as `oc_*` prefixed tools in the `ToolRegistry`.
//! Native KRIA tools always take precedence over OpenClaw skills.
//!
//! # Module Map
//!
//! | Module | Purpose |
//! |--------|---------|
//! | `config` | OpenClaw configuration structs |
//! | `types` | Core types: `SkillDescriptor`, `TrustTier`, `ResourceClass`, `SkillCapabilities` |
//! | `transpiler` | SKILL.md → `SkillDescriptor` (YAML-only, description rewriting) |
//! | `registry` | SQLite-backed skill persistence (`SkillRegistry`) |
//! | `audit` | Append-only HMAC-signed audit ledger |
//! | `sanitizer` | `EvidenceWrapper` — structured evidence blocks for tool output |
//! | `handler` | `OpenClawToolHandler` — `ToolHandler` implementation |
//! | `pool` | Container warm pool with per-invocation ephemeral isolation |
//! | `events` | Docker event stream subscriber with reconnect logic |
//! | `bridge` | Content-Length framed MCP bridge communication |

pub mod audit;
pub mod bridge;
pub mod clawhub;
pub mod config;
pub mod events;
pub mod generation; // A9: Autonomous Skill Generation System (ASGS)
pub mod init;
pub mod platform; // A8: ClawHub publisher ecosystem (repository, publisher, trust, marketplace, updates, sync)
pub mod pool;
pub mod registry;
#[cfg(test)]
pub mod registry_tests;
pub mod trust_runtime;
// pub mod resolver; // A6: REMOVED - replaced by semantic_router
pub mod runtime_manager; // A4: Production runtime manager with lifecycle state machine
pub mod sanitizer;
pub mod transpiler;
pub mod types;

// A1: unified execution substrate (SkillRuntime + HRA admission + SkillEvent stream).
pub mod admission;
pub mod event;
pub mod runtime;

// A2: production skill bundle system (.ocskill) + capability object.
pub mod activation;
pub mod bundle;
pub mod capability;

// A3: capability enforcement (materialization, revocation).
// The legacy `approval::ApprovalCache` permission gate is removed (M12); its one
// reusable piece — the capability-set hash — lives in `cap_hash`.
pub mod cap_hash;
pub mod materialize;
pub mod revocation;

// Re-export core types for convenience.
pub use activation::ToolRegistryActivation;
pub use bundle::{
    Bundle, BundleError, BundleInstaller, InstallError, InstallOutcome, SkillActivation,
};
pub use capability::{
    Capability, CapabilityGrant, CapabilityKind, CapabilityMode, CapabilityScope, GrantSource,
    Materialization,
};
pub use config::OpenClawConfig;
pub use event::{SkillEvent, Stage as SkillEventStage};
pub use init::{OpenClawBootError, OpenClawSubsystem};
pub use materialize::{EnvProvider, MaterializedContainer, NullEnvProvider, ResourceLimits};
pub use pool::{ContainerPool, PoolError};
pub use runtime::{
    DockerRuntime, LaunchSpec, RuntimeContext, RuntimeKind, RuntimeRegistry, SkillRuntime,
};
pub use runtime_manager::{
    ContainerHandle, ContainerState, HealthStatus, Priority, RuntimeError, RuntimeManager,
};
pub use types::{
    AuditEventType, ExecutionSource, LifecycleAction, OpenClawNetworkPolicy, ResourceClass,
    ResourceProfile, SkillCapabilities, SkillDescriptor, SkillSource, SkillStatus, TrustTier,
};
