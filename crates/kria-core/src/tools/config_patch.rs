//! `config_patch` agent tool (settings-config-revamp Task 13 — agent wiring).
//!
//! Lets the chat agent change a KRIA setting when the user asks (e.g. "switch to
//! dark mode"). The LLM emits `{section, field, value}`; the handler validates
//! against the schema, enforces the injection wall (provenance must be `User`),
//! and applies GREEN changes via `ConfigService`. Because the tool's default
//! risk is RED, the agent loop's existing HITL gate asks the user before the
//! handler runs — so YELLOW/RED/BLACK (and even GREEN, when routed through the
//! agent) changes require explicit approval. NL settings control is ON by default
//! (`config::nl::nl_settings_enabled`); disable with `KRIA_NL_SETTINGS=0`.

use async_trait::async_trait;

use crate::infra::ToolResult;
use crate::safety::RiskLevel;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler};
use crate::tools::{ToolContext, TriggerProvenance};

pub struct ConfigPatchTool;

impl ConfigPatchTool {
    pub fn def() -> ToolDef {
        ToolDef {
            name: "config_patch".to_string(),
            description: "Change a KRIA application setting the user asked for (e.g. theme, \
                voice mode, image mode, search engine). Provide the config section, the field, \
                and the new value. Only use this for KRIA's OWN settings — never for the user's \
                code or external systems. Requires user approval."
                .to_string(),
            category: "system".to_string(),
            parameters: vec![
                ParamDef {
                    name: "section".to_string(),
                    param_type: "string".to_string(),
                    description:
                        "Config section, e.g. 'ui', 'voice', 'search', 'image_generation'."
                            .to_string(),
                    required: true,
                    default: None,
                },
                ParamDef {
                    name: "field".to_string(),
                    param_type: "string".to_string(),
                    description: "Field within the section, e.g. 'theme', 'mode', 'engine'."
                        .to_string(),
                    required: true,
                    default: None,
                },
                ParamDef {
                    name: "value".to_string(),
                    param_type: "string".to_string(),
                    description: "New value, e.g. 'dark', 'continuous', 'duckduckgo'.".to_string(),
                    required: true,
                    default: None,
                },
            ],
            // RED so the agent loop's HITL gate asks the user before applying.
            default_tier: RiskLevel::Red,
            min_tier: "lite",
        }
    }
}

#[async_trait]
impl ToolHandler for ConfigPatchTool {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        // NL settings control is ON by default (single source of truth); disabled
        // only by an explicit KRIA_NL_SETTINGS=0 opt-out.
        if !crate::config::nl::nl_settings_enabled() {
            return ToolResult::err("Prompt-driven settings are disabled (KRIA_NL_SETTINGS=0).");
        }

        // Injection wall: never mutate config from non-user-originated content.
        if ctx.provenance != TriggerProvenance::User {
            return ToolResult::err(
                "Refused: configuration changes are only allowed from direct user input.",
            );
        }

        let Some(config) = ctx.config.clone() else {
            return ToolResult::err("Configuration service unavailable.");
        };

        let section = params.get("section").and_then(|v| v.as_str()).unwrap_or("");
        let field = params.get("field").and_then(|v| v.as_str()).unwrap_or("");
        if section.is_empty() || field.is_empty() {
            return ToolResult::err("config_patch requires 'section' and 'field'.");
        }
        // Accept the value as-is; coerce common scalars from string form.
        let raw = params
            .get("value")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let value = coerce_value(&raw);

        // Validate against the schema (field exists, prompt-changeable, allowed value).
        // On failure, emit a STRUCTURED, schema-grounded rejection listing the allowed
        // values so the model can reject-and-reask with a valid value on the next round.
        // This is the cloud-safe reask contract (grammar binds only on local llama.cpp;
        // on cloud we rely on strict validate + a grounded error, never applying an
        // unvalidated value — analysis.md §8 item 4).
        match crate::config::schema::validate_change(section, field, &value, false) {
            Ok(_) => {}
            Err(e) => {
                let meta = crate::config::schema::field_meta(section, field);
                let allowed = meta
                    .valid_values
                    .map(|vs| vs.join(", "))
                    .unwrap_or_else(|| "(free-form; provide a valid value)".to_string());
                return ToolResult::err(format!(
                    "Invalid settings change: {e}. Allowed values for {section}.{field}: {allowed}. \
                     Re-issue config_patch with one of these exact values, or answer the user instead."
                ));
            }
        }

        // Env-lock guard.
        if crate::config::schema::is_env_locked(section, field) {
            return ToolResult::err(format!(
                "{section}.{field} is locked by an environment variable and cannot be changed here."
            ));
        }

        // Apply. INVARIANT: every live path that reaches this handler is already
        // risk-gated for non-GREEN fields —
        //   • ReAct tool dispatch: the loop HITL-gates `config_patch` (RED tier)
        //     BEFORE calling the handler, so a YELLOW/RED change only lands here
        //     after approval;
        //   • deterministic pre-dispatch router (try_config_prompt_dispatch): only
        //     auto-routes GREEN (auto-execute) fields — YELLOW+ fall through;
        //   • the `config_prompt` command uses `evaluate()` (its own HITL), not
        //     this tool.
        // The injection wall + schema/env-lock checks above are the tool's own
        // guards. Persist + live-apply through the single writer.
        match config
            .patch(
                section,
                field,
                value.clone(),
                crate::config::ChangeSource::Prompt,
                None,
            )
            .await
        {
            Ok(applied) => ToolResult::ok(serde_json::json!({
                "status": "applied",
                "section": section,
                "field": field,
                "value": value,
                "version": applied.version,
                "message": format!("Updated {section}.{field}."),
            })),
            Err(e) => ToolResult::err(format!("Failed to apply settings change: {e}")),
        }
    }
}

/// Coerce a string value into the most natural JSON scalar ("true"→bool,
/// "42"→number), leaving other strings/values untouched.
fn coerce_value(raw: &serde_json::Value) -> serde_json::Value {
    if let Some(s) = raw.as_str() {
        match s.trim().to_ascii_lowercase().as_str() {
            "true" => return serde_json::json!(true),
            "false" => return serde_json::json!(false),
            _ => {}
        }
        if let Ok(i) = s.trim().parse::<i64>() {
            return serde_json::json!(i);
        }
        if let Ok(f) = s.trim().parse::<f64>() {
            return serde_json::json!(f);
        }
        return serde_json::json!(s);
    }
    raw.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ConfigService, KriaConfig, NoopPersist};
    use crate::infra::event_bus::EventBus;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn ctx_with_config(provenance: TriggerProvenance) -> (ToolContext, Arc<ConfigService>) {
        let cfg = Arc::new(RwLock::new(KriaConfig::default()));
        let bus = Arc::new(EventBus::new(16));
        let svc = Arc::new(ConfigService::with_persist(cfg, bus, Arc::new(NoopPersist)));
        let ctx = crate::tools::ToolContext::new(
            Arc::new(crate::infra::environment::LocalEnvironment::new()),
            svc_shell(),
            tokio_util::sync::CancellationToken::new(),
        )
        .with_provenance(provenance)
        .with_config(svc.clone());
        (ctx, svc)
    }

    fn svc_shell() -> crate::infra::environment::SharedShellState {
        Arc::new(tokio::sync::Mutex::new(
            crate::infra::environment::ShellState {
                cwd: std::path::PathBuf::from("."),
                env_vars: std::collections::HashMap::new(),
                generation: 0,
            },
        ))
    }

    #[tokio::test]
    async fn applies_valid_change_from_user() {
        std::env::set_var("KRIA_CONFIG_PROMPT_CONTROL", "1");
        let (ctx, svc) = ctx_with_config(TriggerProvenance::User);
        let res = ConfigPatchTool
            .execute_with_context(
                serde_json::json!({ "section": "ui", "field": "theme", "value": "dark" }),
                ctx,
            )
            .await;
        std::env::remove_var("KRIA_CONFIG_PROMPT_CONTROL");
        assert!(res.success, "expected success, got {res:?}");
        assert_eq!(svc.get().await.ui.theme, "dark");
    }

    #[tokio::test]
    async fn refuses_non_user_provenance() {
        std::env::set_var("KRIA_CONFIG_PROMPT_CONTROL", "1");
        let (ctx, svc) = ctx_with_config(TriggerProvenance::ExternalContent);
        let res = ConfigPatchTool
            .execute_with_context(
                serde_json::json!({ "section": "ui", "field": "theme", "value": "dark" }),
                ctx,
            )
            .await;
        std::env::remove_var("KRIA_CONFIG_PROMPT_CONTROL");
        assert!(!res.success, "injection wall must refuse");
        assert_ne!(svc.get().await.ui.theme, "dark");
    }

    #[tokio::test]
    async fn rejects_invalid_value() {
        std::env::set_var("KRIA_CONFIG_PROMPT_CONTROL", "1");
        let (ctx, _svc) = ctx_with_config(TriggerProvenance::User);
        let res = ConfigPatchTool
            .execute_with_context(
                serde_json::json!({ "section": "ui", "field": "theme", "value": "rainbow" }),
                ctx,
            )
            .await;
        std::env::remove_var("KRIA_CONFIG_PROMPT_CONTROL");
        assert!(!res.success, "invalid enum value must be rejected");
    }

    #[tokio::test]
    async fn invalid_value_error_lists_allowed_values_for_reask() {
        // Cloud reject-and-reask: the rejection must ground the model with the
        // allowed values so it can retry with a valid one.
        std::env::set_var("KRIA_CONFIG_PROMPT_CONTROL", "1");
        let (ctx, _svc) = ctx_with_config(TriggerProvenance::User);
        let res = ConfigPatchTool
            .execute_with_context(
                serde_json::json!({ "section": "ui", "field": "theme", "value": "rainbow" }),
                ctx,
            )
            .await;
        std::env::remove_var("KRIA_CONFIG_PROMPT_CONTROL");
        assert!(!res.success);
        let msg = res.error.clone().unwrap_or_default();
        assert!(
            msg.contains("light") && msg.contains("dark"),
            "reask must list allowed values: {msg}"
        );
    }
}
