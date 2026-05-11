use crate::infra::ToolResult;
use crate::safety::RiskLevel;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use async_trait::async_trait;
use std::sync::Arc;

fn param(name: &str, ty: &str, desc: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        param_type: ty.into(),
        description: desc.into(),
        required,
        default: None,
    }
}

struct SetProcessPriority;
#[async_trait]
impl ToolHandler for SetProcessPriority {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let pid = params["pid"].as_u64().unwrap_or(0);
        let priority = params["priority"].as_i64().unwrap_or(0);
        let output = tokio::process::Command::new("renice")
            .args([&priority.to_string(), "-p", &pid.to_string()])
            .output()
            .await;
        match output {
            Ok(o) if o.status.success() => ToolResult::ok(serde_json::json!({
                "pid": pid, "priority": priority, "set": true
            })),
            _ => ToolResult::err(format!("failed to set priority for PID {pid}")),
        }
    }
}

struct GetActiveConnections;
#[async_trait]
impl ToolHandler for GetActiveConnections {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        let output = tokio::process::Command::new("ss")
            .args(["-tuln"])
            .output()
            .await;
        match output {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout).to_string();
                ToolResult::ok_text(text)
            }
            _ => ToolResult::err("failed to get active connections"),
        }
    }
}

pub fn register(reg: &ToolRegistry) {
    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        // GREEN
        (
            ToolDef {
                name: "get_active_connections".into(),
                description: "Get active network connections".into(),
                category: "process".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(GetActiveConnections),
        ),
        // RED
        (
            ToolDef {
                name: "set_process_priority".into(),
                description: "Set process priority/niceness".into(),
                category: "process".into(),
                default_tier: RiskLevel::Red,
                min_tier: "standard",
                parameters: vec![
                    param("pid", "integer", "Process ID", true),
                    param("priority", "integer", "Nice value (-20 to 19)", true),
                ],
            },
            Arc::new(SetProcessPriority),
        ),
    ];
    for (def, handler) in tools {
        reg.register(def, handler);
    }
}
