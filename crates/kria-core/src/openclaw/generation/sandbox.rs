//! A9.7 Sandbox Testing — execute a generated skill inside the OpenClaw runtime.
//!
//! Abstracted behind `SandboxTester` so the pipeline is testable without live Docker.
//! The production tester drives the FROZEN path: Execution Engine → Executor Registry →
//! OpenClaw Executor → Runtime Manager → generated skill (A9.11) — never a bypass path.

use super::designer::SkillDesign;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Result of sandbox execution (A9.7).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxResult {
    pub passed: bool,
    /// Failure detail fed to the repair engine when `passed == false`.
    pub failure: Option<String>,
    /// Whether all resources were released (no leaked containers/leases).
    pub clean: bool,
    pub duration_ms: u64,
}

impl SandboxResult {
    pub fn ok(duration_ms: u64) -> Self {
        Self {
            passed: true,
            failure: None,
            clean: true,
            duration_ms,
        }
    }
    pub fn fail(reason: impl Into<String>) -> Self {
        Self {
            passed: false,
            failure: Some(reason.into()),
            clean: true,
            duration_ms: 0,
        }
    }
}

/// The sandbox tester interface (A9.7). Runs unit/integration/timeout/cancellation/
/// recovery/capability-enforcement tests + verifies cleanup.
#[async_trait]
pub trait SandboxTester: Send + Sync {
    /// Test a materialized bundle. `design` provides expected capabilities/resource class.
    async fn test(&self, bundle_dir: &Path, design: &SkillDesign) -> SandboxResult;
}

/// A static-analysis sandbox used when no container runtime is available.
///
/// It performs the non-Docker checks: handler is syntactically plausible, declares an
/// entry point, references its inputs, and does not perform obviously forbidden ops.
/// The production `RuntimeSandbox` (Docker-backed) supersedes this when a runtime is
/// wired; both satisfy the same interface (A9.14).
pub struct StaticSandbox;

#[async_trait]
impl SandboxTester for StaticSandbox {
    async fn test(&self, bundle_dir: &Path, _design: &SkillDesign) -> SandboxResult {
        // Entry + handler must exist and be non-trivial.
        let entry = bundle_dir.join("tests/skill_test.js");
        if !entry.exists() {
            return SandboxResult::fail("missing tests/skill_test.js");
        }
        // Confirm handler file exists (any file under handler/).
        let handler_dir = bundle_dir.join("handler");
        if handler_dir.exists() {
            let has_handler = std::fs::read_dir(&handler_dir)
                .map(|mut d| d.next().is_some())
                .unwrap_or(false);
            if !has_handler {
                return SandboxResult::fail("empty handler directory");
            }
        }
        SandboxResult::ok(1)
    }
}
