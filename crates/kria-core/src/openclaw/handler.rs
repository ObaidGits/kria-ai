//! OpenClaw tool handler — `ToolHandler` implementation.
//!
//! Wraps tool execution through the OpenClaw container substrate.
//! Integrates with `EvidenceWrapper` for safe output handling and
//! `AuditLedger` for invocation tracking.

use super::audit::{AuditEntry, AuditLedger};
use super::bridge::McpBridge;
use super::pool::{ContainerHandle, ContainerPool, PoolError};
use super::sanitizer::EvidenceWrapper;
use super::types::*;
use crate::infra::isolation::ToolResult;
use crate::tools::registry::ToolHandler;
use async_trait::async_trait;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::process::Command;

/// Tool handler that delegates execution to the OpenClaw substrate.
///
/// Each installed skill gets its own `OpenClawToolHandler` instance
/// registered in the `ToolRegistry` with the `oc_` prefix.
pub struct OpenClawToolHandler {
    /// The skill descriptor for this handler.
    skill: SkillDescriptor,
    /// Reference to the container pool.
    pool: Arc<ContainerPool>,
    /// Reference to the HMAC-signing audit ledger.
    audit: Arc<AuditLedger>,
}

impl OpenClawToolHandler {
    pub fn new(
        skill: SkillDescriptor,
        pool: Arc<ContainerPool>,
        audit: Arc<AuditLedger>,
    ) -> Self {
        Self { skill, pool, audit }
    }

    /// Get the skill descriptor.
    pub fn skill(&self) -> &SkillDescriptor {
        &self.skill
    }

    /// Sign and append a single audit entry — helper to avoid repetition.
    fn audit_append(&self, entry: &mut AuditEntry) {
        entry.signature = self.audit.sign_entry(entry);
        if let Err(e) = self.audit.append(entry) {
            tracing::warn!(
                skill_id = %self.skill.skill_id,
                invocation_id = %entry.invocation_id,
                error = %e,
                "OpenClaw: failed to write audit entry"
            );
        }
    }
}

#[async_trait]
impl ToolHandler for OpenClawToolHandler {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let start = Instant::now();
        let invocation_id = uuid::Uuid::new_v4().to_string();

        // 1. Write InvocationStarted audit entry before any container work.
        let mut started = AuditLedger::create_invocation_entry(
            AuditEventType::InvocationStarted,
            &self.skill.skill_id,
            &invocation_id,
            "",
            "",
            &self.skill.skill_id,
            self.skill.risk_level.as_str(),
            &params,
            &ToolResult { success: true, data: serde_json::Value::Null, error: None },
            0,
            self.skill.resource_profile.resource_class.as_str(),
            "",
        );
        self.audit_append(&mut started);

        // 2. Checkout a container from the pool.
        let container = match self
            .pool
            .checkout(self.skill.resource_profile.resource_class, &self.skill.skill_id)
            .await
        {
            Ok(h) => h,
            Err(PoolError::MaxConcurrent(max)) => {
                let err_msg = format!(
                    "OpenClaw substrate: max concurrent invocations reached ({})", max
                );
                let mut entry = AuditLedger::create_invocation_entry(
                    AuditEventType::InvocationFailed,
                    &self.skill.skill_id, &invocation_id, "", "",
                    &self.skill.skill_id, self.skill.risk_level.as_str(),
                    &params,
                    &ToolResult { success: false, data: serde_json::Value::Null, error: Some(err_msg.clone()) },
                    start.elapsed().as_millis() as u64,
                    self.skill.resource_profile.resource_class.as_str(), "",
                );
                self.audit_append(&mut entry);
                return ToolResult { success: false, data: serde_json::Value::Null, error: Some(err_msg) };
            }
            Err(e) => {
                let err_msg = format!("OpenClaw substrate error: {}", e);
                let mut entry = AuditLedger::create_invocation_entry(
                    AuditEventType::InvocationFailed,
                    &self.skill.skill_id, &invocation_id, "", "",
                    &self.skill.skill_id, self.skill.risk_level.as_str(),
                    &params,
                    &ToolResult { success: false, data: serde_json::Value::Null, error: Some(err_msg.clone()) },
                    start.elapsed().as_millis() as u64,
                    self.skill.resource_profile.resource_class.as_str(), "",
                );
                self.audit_append(&mut entry);
                return ToolResult { success: false, data: serde_json::Value::Null, error: Some(err_msg) };
            }
        };

        // 3. Execute via MCP bridge inside the ephemeral container.
        let timeout = Duration::from_secs(self.skill.resource_profile.timeout_secs);
        let raw_result = execute_in_container(
            &container,
            &self.skill.skill_id,
            &params,
            timeout,
        )
        .await;

        let duration_ms = start.elapsed().as_millis() as u64;
        let container_id = container.container_id.clone();

        // 4. Destroy the container — no state survives.
        let _ = self.pool.checkin(container).await;

        // 5. Wrap result in structured evidence block (prevents prompt injection).
        let wrapped = EvidenceWrapper::wrap(
            &self.skill.skill_id,
            ExecutionSource::OpenClaw,
            &raw_result,
            duration_ms,
        );

        // 6. Write InvocationCompleted/Failed — sign then append.
        let event_type = if raw_result.success {
            AuditEventType::InvocationCompleted
        } else {
            AuditEventType::InvocationFailed
        };
        let mut completed = AuditLedger::create_invocation_entry(
            event_type,
            &self.skill.skill_id,
            &invocation_id,
            "",
            "",
            &self.skill.skill_id,
            self.skill.risk_level.as_str(),
            &params,
            &raw_result,
            duration_ms,
            self.skill.resource_profile.resource_class.as_str(),
            &container_id,
        );
        self.audit_append(&mut completed);

        // 7. Return the EvidenceWrapper output to the LLM.
        ToolResult {
            success: raw_result.success,
            data: serde_json::json!(wrapped),
            error: raw_result.error,
        }
    }
}

/// Execute a tool in a container via MCP bridge.
///
/// 1. Attach to the container's stdin/stdout via `docker attach`.
/// 2. Send a Content-Length framed JSON-RPC `tools/call` message.
/// 3. Read the response.
/// 4. Parse the MCP response into a `ToolResult`.
async fn execute_in_container(
    container: &ContainerHandle,
    tool_name: &str,
    params: &serde_json::Value,
    timeout: Duration,
) -> ToolResult {
    // Attach to the container's stdio for MCP communication
    let mut child = match Command::new("docker")
        .args(["attach", "--no-stdin", &container.container_id])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return ToolResult {
                success: false,
                data: serde_json::Value::Null,
                error: Some(format!("Failed to attach to container: {}", e)),
            };
        }
    };

    // Create MCP bridge from the child process
    let mut bridge = match McpBridge::new(&mut child) {
        Ok(b) => b,
        Err(e) => {
            let _ = child.kill().await;
            return ToolResult {
                success: false,
                data: serde_json::Value::Null,
                error: Some(format!("MCP bridge init failed: {}", e)),
            };
        }
    };

    // Initialize the bridge (handshake)
    if let Err(e) = bridge.initialize().await {
        let _ = child.kill().await;
        return ToolResult {
            success: false,
            data: serde_json::Value::Null,
            error: Some(format!("MCP bridge handshake failed: {}", e)),
        };
    }

    // Call the tool
    match bridge
        .call_tool(tool_name, Some(params.clone()), timeout)
        .await
    {
        Ok(result) => {
            let text = result.text();
            let _ = child.kill().await;

            if result.is_error {
                ToolResult {
                    success: false,
                    data: serde_json::Value::Null,
                    error: Some(text),
                }
            } else {
                // Try to parse as JSON, fall back to string
                let data = serde_json::from_str(&text)
                    .unwrap_or(serde_json::json!(text));

                ToolResult {
                    success: true,
                    data,
                    error: None,
                }
            }
        }
        Err(e) => {
            let _ = child.kill().await;
            ToolResult {
                success: false,
                data: serde_json::Value::Null,
                error: Some(format!("Tool execution failed: {}", e)),
            }
        }
    }
}
