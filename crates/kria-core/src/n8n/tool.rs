use super::client::{N8nClient, N8nClientError};
use super::types::N8nToolRequest;
use crate::infra::ToolResult;
use crate::safety::RiskLevel;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use crate::tools::ToolContext;
use async_trait::async_trait;
use std::sync::Arc;

#[derive(Clone)]
pub struct N8nInvokeWorkflowHandler {
    client: Arc<N8nClient>,
}

impl N8nInvokeWorkflowHandler {
    pub fn new(client: Arc<N8nClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl ToolHandler for N8nInvokeWorkflowHandler {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        if ctx.cancellation.is_cancelled() {
            return ToolResult::err("n8n workflow invocation cancelled before dispatch");
        }

        let parsed: N8nToolRequest = match serde_json::from_value(params) {
            Ok(req) => req,
            Err(error) => {
                return ToolResult::err(format!("invalid n8n_invoke_workflow params: {error}"))
            }
        };

        match self.client.invoke(parsed).await {
            Ok(result) => ToolResult::ok(serde_json::to_value(result).unwrap_or_else(
                |_| serde_json::json!({"error": "failed to serialize n8n invocation result"}),
            )),
            Err(error) => tool_error(error),
        }
    }
}

fn tool_error(error: N8nClientError) -> ToolResult {
    ToolResult::err_with_data(
        format!("n8n workflow invocation failed: {error}"),
        serde_json::json!({
            "error_class": classify_error(&error),
        }),
    )
}

fn classify_error(error: &N8nClientError) -> &'static str {
    match error {
        N8nClientError::Catalog(_) => "contract_or_catalog_error",
        N8nClientError::PayloadTooLarge { .. } => "payload_too_large",
        N8nClientError::SchemaValidation(_) => "workflow_input_schema_error",
        N8nClientError::Serialize(_) => "serialization_error",
        N8nClientError::Signing => "signing_error",
        N8nClientError::Http(_) => "external_transport_error",
        N8nClientError::ResponseJson(_) => "external_response_schema_error",
        N8nClientError::BadStatus { .. } => "external_workflow_error",
    }
}

pub fn register(reg: &ToolRegistry, client: Arc<N8nClient>) {
    reg.register(
        ToolDef {
            name: "n8n_invoke_workflow".into(),
            description:
                "Invoke an allowlisted, version-pinned n8n workflow through KRIA governance".into(),
            category: "external_workflow".into(),
            default_tier: RiskLevel::Yellow,
            min_tier: "lite",
            parameters: vec![
                ParamDef {
                    name: "workflow_id".into(),
                    param_type: "string".into(),
                    description: "Allowlisted KRIA-facing n8n workflow ID".into(),
                    required: true,
                    default: None,
                },
                ParamDef {
                    name: "workflow_version".into(),
                    param_type: "string".into(),
                    description: "Expected approved workflow version; rejects mismatches".into(),
                    required: false,
                    default: None,
                },
                ParamDef {
                    name: "input_payload".into(),
                    param_type: "object".into(),
                    description: "Bounded JSON payload for the workflow".into(),
                    required: false,
                    default: Some(serde_json::json!({})),
                },
                ParamDef {
                    name: "correlation_id".into(),
                    param_type: "string".into(),
                    description: "Optional KRIA turn/workflow correlation ID".into(),
                    required: false,
                    default: None,
                },
                ParamDef {
                    name: "idempotency_key".into(),
                    param_type: "string".into(),
                    description: "Optional idempotency key for side-effect-safe retries".into(),
                    required: false,
                    default: None,
                },
            ],
        },
        Arc::new(N8nInvokeWorkflowHandler::new(client)),
    );
}
