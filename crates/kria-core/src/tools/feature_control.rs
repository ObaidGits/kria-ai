//! Prompt-accessible status and control for runtime-managed KRIA features.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::infra::ToolResult;
use crate::safety::RiskLevel;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use crate::tools::{ToolContext, TriggerProvenance};

/// Prompt-facing lifecycle state for a controllable feature.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeatureControlState {
    Disabled,
    Starting,
    Running,
    Stopping,
    Error,
}

/// Unified prompt-facing feature status.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureControl {
    pub id: String,
    pub label: String,
    pub description: String,
    pub desired_enabled: bool,
    pub state: FeatureControlState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Runtime adapter implemented by the host that owns feature lifecycles.
#[async_trait]
pub trait FeatureControlBackend: Send + Sync {
    async fn list(&self) -> Result<Vec<FeatureControl>, String>;

    async fn set_enabled(&self, id: &str, enabled: bool) -> Result<FeatureControl, String>;
}

pub struct FeatureStatusTool;
pub struct FeatureControlTool;

impl FeatureStatusTool {
    pub fn def() -> ToolDef {
        ToolDef {
            name: "feature_status".into(),
            description: "List controllable KRIA features and their runtime status, or inspect one feature by ID.".into(),
            category: "system".into(),
            parameters: vec![ParamDef {
                name: "id".into(),
                param_type: "string".into(),
                description: "Optional feature ID. Omit to list every controllable feature.".into(),
                required: false,
                default: None,
            }],
            default_tier: RiskLevel::Green,
            min_tier: "lite",
        }
    }
}

impl FeatureControlTool {
    pub fn def() -> ToolDef {
        ToolDef {
            name: "feature_control".into(),
            description:
                "Enable or disable a runtime-managed KRIA feature by ID. Requires user approval."
                    .into(),
            category: "system".into(),
            parameters: vec![
                ParamDef {
                    name: "id".into(),
                    param_type: "string".into(),
                    description: "Feature ID to enable or disable.".into(),
                    required: true,
                    default: None,
                },
                ParamDef {
                    name: "enabled".into(),
                    param_type: "boolean".into(),
                    description: "True to enable the feature; false to disable it.".into(),
                    required: true,
                    default: None,
                },
            ],
            default_tier: RiskLevel::Red,
            min_tier: "lite",
        }
    }
}

#[async_trait]
impl ToolHandler for FeatureStatusTool {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let Some(backend) = ctx.feature_control_backend else {
            return ToolResult::err("Feature control backend unavailable.");
        };

        let features = match backend.list().await {
            Ok(features) => features,
            Err(error) => return ToolResult::err(format!("Failed to list features: {error}")),
        };

        match params.get("id") {
            None | Some(serde_json::Value::Null) => ToolResult::ok(serde_json::json!(features)),
            Some(value) => {
                let Some(id) = value.as_str().filter(|id| !id.is_empty()) else {
                    return ToolResult::err("feature_status 'id' must be a non-empty string.");
                };
                match features.into_iter().find(|feature| feature.id == id) {
                    Some(feature) => ToolResult::ok(serde_json::json!(feature)),
                    None => ToolResult::err(format!("Unknown feature ID: {id}")),
                }
            }
        }
    }
}

#[async_trait]
impl ToolHandler for FeatureControlTool {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        if ctx.provenance != TriggerProvenance::User {
            return ToolResult::err(
                "Refused: feature changes are only allowed from direct user input.",
            );
        }

        let Some(backend) = ctx.feature_control_backend else {
            return ToolResult::err("Feature control backend unavailable.");
        };
        let Some(id) = params
            .get("id")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.is_empty())
        else {
            return ToolResult::err("feature_control requires a non-empty string 'id'.");
        };
        let Some(enabled) = params.get("enabled").and_then(serde_json::Value::as_bool) else {
            return ToolResult::err("feature_control requires boolean 'enabled'.");
        };

        match backend.set_enabled(id, enabled).await {
            Ok(feature) => ToolResult::ok(serde_json::json!(feature)),
            Err(error) => ToolResult::err(format!("Failed to update feature '{id}': {error}")),
        }
    }
}

pub fn register(registry: &ToolRegistry) {
    registry.register(
        FeatureStatusTool::def(),
        std::sync::Arc::new(FeatureStatusTool),
    );
    registry.register(
        FeatureControlTool::def(),
        std::sync::Arc::new(FeatureControlTool),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::environment::{LocalEnvironment, ShellState};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tokio_util::sync::CancellationToken;

    struct MockBackend {
        features: Mutex<Vec<FeatureControl>>,
        calls: Mutex<Vec<(String, bool)>>,
    }

    impl MockBackend {
        fn new() -> Self {
            Self {
                features: Mutex::new(vec![FeatureControl {
                    id: "voice".into(),
                    label: "Voice".into(),
                    description: "Voice interaction".into(),
                    desired_enabled: true,
                    state: FeatureControlState::Running,
                    detail: Some("ready".into()),
                    error: None,
                }]),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl FeatureControlBackend for MockBackend {
        async fn list(&self) -> Result<Vec<FeatureControl>, String> {
            Ok(self.features.lock().await.clone())
        }

        async fn set_enabled(&self, id: &str, enabled: bool) -> Result<FeatureControl, String> {
            self.calls.lock().await.push((id.to_string(), enabled));
            let mut features = self.features.lock().await;
            let feature = features
                .iter_mut()
                .find(|feature| feature.id == id)
                .ok_or_else(|| format!("unknown feature: {id}"))?;
            feature.desired_enabled = enabled;
            feature.state = if enabled {
                FeatureControlState::Running
            } else {
                FeatureControlState::Disabled
            };
            Ok(feature.clone())
        }
    }

    fn context(provenance: TriggerProvenance) -> ToolContext {
        ToolContext::new(
            Arc::new(LocalEnvironment::new()),
            Arc::new(Mutex::new(ShellState {
                cwd: PathBuf::from("."),
                env_vars: HashMap::new(),
                generation: 0,
            })),
            CancellationToken::new(),
        )
        .with_provenance(provenance)
    }

    #[test]
    fn dto_serializes_camel_case_and_lowercase_state() {
        let feature = MockBackend::new().features.into_inner()[0].clone();
        let value = serde_json::to_value(feature).unwrap();
        assert_eq!(value["desiredEnabled"], true);
        assert_eq!(value["state"], "running");
        assert!(value.get("desired_enabled").is_none());
        assert!(value.get("error").is_none());
    }

    #[tokio::test]
    async fn status_lists_or_returns_one_feature() {
        let backend = Arc::new(MockBackend::new());
        let ctx = context(TriggerProvenance::User).with_feature_control_backend(backend);
        let list = FeatureStatusTool
            .execute_with_context(serde_json::json!({}), ctx.clone())
            .await;
        assert!(list.success);
        assert_eq!(list.data.as_array().unwrap().len(), 1);

        let one = FeatureStatusTool
            .execute_with_context(serde_json::json!({"id": "voice"}), ctx)
            .await;
        assert!(one.success);
        assert_eq!(one.data["id"], "voice");
    }

    #[tokio::test]
    async fn tools_reject_missing_backend() {
        let status = FeatureStatusTool
            .execute_with_context(serde_json::json!({}), context(TriggerProvenance::User))
            .await;
        let control = FeatureControlTool
            .execute_with_context(
                serde_json::json!({"id": "voice", "enabled": false}),
                context(TriggerProvenance::User),
            )
            .await;
        assert!(!status.success);
        assert!(!control.success);
        assert!(status.error.unwrap().contains("backend unavailable"));
        assert!(control.error.unwrap().contains("backend unavailable"));
    }

    #[tokio::test]
    async fn mutation_requires_user_provenance_and_updates_backend() {
        let backend = Arc::new(MockBackend::new());
        let rejected = FeatureControlTool
            .execute_with_context(
                serde_json::json!({"id": "voice", "enabled": false}),
                context(TriggerProvenance::ExternalContent)
                    .with_feature_control_backend(backend.clone()),
            )
            .await;
        assert!(!rejected.success);
        assert!(backend.calls.lock().await.is_empty());

        let updated = FeatureControlTool
            .execute_with_context(
                serde_json::json!({"id": "voice", "enabled": false}),
                context(TriggerProvenance::User).with_feature_control_backend(backend.clone()),
            )
            .await;
        assert!(updated.success);
        assert_eq!(updated.data["desiredEnabled"], false);
        assert_eq!(
            backend.calls.lock().await.as_slice(),
            &[("voice".into(), false)]
        );
    }

    #[test]
    fn definitions_and_registry_wiring_are_correct() {
        assert_eq!(FeatureStatusTool::def().default_tier, RiskLevel::Green);
        assert_eq!(FeatureControlTool::def().default_tier, RiskLevel::Red);

        let registry = ToolRegistry::new();
        register(&registry);
        registry.set_feature_control_backend(Arc::new(MockBackend::new()));
        assert_eq!(
            registry.get_def("feature_status").unwrap().default_tier,
            RiskLevel::Green
        );
        assert_eq!(
            registry.get_def("feature_control").unwrap().default_tier,
            RiskLevel::Red
        );
        assert!(registry
            .make_tool_context(CancellationToken::new())
            .feature_control_backend
            .is_some());
    }
}
