//! Anti-Corruption Layer (ACL) — the ONLY place provider-native types live.
//!
//! Each submodule adapts one provider's internal API to the neutral
//! [`crate::capability::CapabilityProvider`] boundary. Nothing outside this
//! module tree may import a provider-native type (`openclaw::*`, `mcp::client`,
//! …); the boundary-integrity test asserts this. Adapters translate *into* the
//! neutral domain types and never leak provider types back out.

pub mod code_sandbox;
pub mod local_fs;
pub mod mcp;
pub mod openclaw;
pub mod synthesis;

pub use code_sandbox::{CodeSandbox, SandboxLimits};
pub use local_fs::{LocalFsProvider, LocalManifest};
pub use mcp::McpProvider;
pub use openclaw::OpenClawProvider;
pub use synthesis::SynthesisProvider;
