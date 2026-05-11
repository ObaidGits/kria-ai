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
pub mod handler;
pub mod init;
pub mod pool;
pub mod registry;
pub mod resolver;
pub mod sanitizer;
pub mod transpiler;
pub mod types;

// Re-export core types for convenience.
pub use config::OpenClawConfig;
pub use init::{OpenClawBootError, OpenClawSubsystem};
pub use pool::{ContainerPool, PoolError};
pub use types::{
    AuditEventType, ExecutionSource, LifecycleAction, OpenClawNetworkPolicy, ResourceClass,
    ResourceProfile, SkillCapabilities, SkillDescriptor, SkillSource, SkillStatus, TrustTier,
};
