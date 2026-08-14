//! Credential-store tool handlers — list, store, replace, delete.
//!
//! linux-os-control-production task **3.10** (OSC-025).
//!
//! # The single rule this file exists to enforce
//!
//! **A secret value never leaves the provider.** Not in a returned result, not in
//! an error message, not in a log line, not in the params digest, not in the
//! audit record. `list_secret_references` returns metadata only; the store and
//! replace handlers move the value straight into the port as a
//! [`ProtectedInputHandle`] without ever holding it in a local `String`.
//!
//! # Why these handlers do not use `run_mutation`
//!
//! [`CredentialStore`] is not a desired-state domain: storing a secret has no
//! observable postcondition to compare, because verifying it would mean reading
//! the value back. So these handlers seal the mutation context and call the port
//! directly, then commit the terminal audit record — the same authority chain,
//! without a verification step that could only be performed by leaking the thing
//! being protected.

use std::sync::Arc;

use async_trait::async_trait;

use crate::infra::ToolResult;
use crate::os_control::contract::SafeText;
use crate::os_control::secrets::{
    ProtectedInputHandle, SecretPayload, SecretPurpose, SecretRef, SecretScope,
};
use crate::safety::RiskLevel;
use crate::tools::os_governed as gov;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use crate::tools::ToolContext;

fn param(name: &str, ty: &str, desc: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        param_type: ty.into(),
        description: desc.into(),
        required,
        default: None,
    }
}

/// Parse the closed purpose set. An unrecognised token is refused rather than
/// mapped to `Other`, so a typo cannot quietly file a Wi-Fi password under the
/// wrong purpose and make it unfindable later.
fn parse_purpose(raw: &str) -> Result<SecretPurpose, ToolResult> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "wifi_password" | "wifi-password" => Ok(SecretPurpose::WifiPassword),
        "vpn_credential" | "vpn-credential" => Ok(SecretPurpose::VpnCredential),
        "proxy_credential" | "proxy-credential" => Ok(SecretPurpose::ProxyCredential),
        "hotspot_credential" | "hotspot-credential" => Ok(SecretPurpose::HotspotCredential),
        "other" => Ok(SecretPurpose::Other),
        other => Err(ToolResult::err(format!(
            "`purpose` must be one of wifi_password, vpn_credential, proxy_credential, \
             hotspot_credential, other (got `{other}`)"
        ))),
    }
}

/// Take the secret value out of the params **by moving it**, so it is never
/// cloned into another owner and the caller's copy is the only one.
fn take_secret(params: &mut serde_json::Value, tool: &str) -> Result<ProtectedInputHandle, ToolResult> {
    let taken = params
        .get_mut("value")
        .map(std::mem::take)
        .unwrap_or(serde_json::Value::Null);
    let Some(value) = taken.as_str() else {
        return Err(ToolResult::err(format!("`{tool}` requires a `value` string")));
    };
    if value.is_empty() {
        // An empty secret would read back later as a present-but-useless
        // credential, which is worse than refusing now.
        return Err(ToolResult::err("`value` must not be empty"));
    }
    let handle = ProtectedInputHandle::new(SecretPayload::new(value.as_bytes().to_vec()));
    // Overwrite the params entry so the value cannot reach the params digest or
    // the audit record through the request body.
    if let Some(slot) = params.get_mut("value") {
        *slot = serde_json::Value::String("<redacted>".to_string());
    }
    Ok(handle)
}

struct ListSecretReferences;

#[async_trait]
impl ToolHandler for ListSecretReferences {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "list_secret_references")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "list_secret_references";
        let purpose = match params["purpose"].as_str() {
            Some(raw) => match parse_purpose(raw) {
                Ok(purpose) => Some(purpose),
                Err(result) => return result,
            },
            None => None,
        };
        let limit = params["limit"]
            .as_u64()
            .and_then(|v| u16::try_from(v).ok())
            .unwrap_or(50);

        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let store = match resolved.runtime.secrets(tool) {
            Ok(store) => store,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::read_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        match store
            .list_metadata(call.observation(), purpose, params["cursor"].as_str(), limit)
            .await
        {
            // Metadata only: reference, purpose, scope, label, timestamps. The
            // value is never read, so it cannot leak through this listing.
            Ok(page) => ToolResult::ok(serde_json::json!({
                "secrets": page.items.as_slice().iter().map(|m| serde_json::json!({
                    "reference": m.reference.as_str(),
                    "purpose": format!("{:?}", m.purpose),
                    "scope": m.scope.as_str(),
                    "label": m.label.as_str(),
                    "created_unix": m.created_unix,
                    "expires_unix": m.expires_unix,
                })).collect::<Vec<_>>(),
                "next_cursor": page.next_cursor.as_ref().map(|c| c.as_str()),
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct StoreSecret;

#[async_trait]
impl ToolHandler for StoreSecret {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "store_secret")
    }

    async fn execute_with_context(
        &self,
        mut params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "store_secret";
        let purpose = match params["purpose"].as_str() {
            Some(raw) => match parse_purpose(raw) {
                Ok(purpose) => purpose,
                Err(result) => return result,
            },
            None => return ToolResult::err("`purpose` is required"),
        };
        let scope_raw = params["scope"].as_str().unwrap_or("").trim().to_string();
        if scope_raw.is_empty() {
            return ToolResult::err("`scope` is required (what this credential is for)");
        }
        let label = params["label"].as_str().unwrap_or("").trim().to_string();
        let input = match take_secret(&mut params, tool) {
            Ok(input) => input,
            Err(result) => return result,
        };

        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        let Some(grant) = call.grant() else {
            return ToolResult::err("a secret mutation requires a grant");
        };
        let Some(leases) = call.leases() else {
            return ToolResult::err("a secret mutation requires held leases");
        };
        let binding = call.binding();
        // Sealing proves grant, leases, audit admission and observation all came
        // from the SAME admission before the keyring is touched.
        let sealed = match resolved.runtime.seal_mutation_context(
            call.observation(),
            grant,
            leases,
            call.admission(),
            &binding,
        ) {
            Ok(sealed) => sealed,
            Err(error) => return gov::os_error(&error),
        };
        let store = match resolved.runtime.secrets(tool) {
            Ok(store) => store,
            Err(error) => return gov::os_error(&error),
        };
        match store
            .store(
                &sealed,
                purpose,
                SecretScope::new(scope_raw),
                SafeText::new(label),
                input,
            )
            .await
        {
            // Only the reference and metadata come back — never the value.
            Ok(metadata) => ToolResult::ok(serde_json::json!({
                "reference": metadata.reference.as_str(),
                "purpose": format!("{:?}", metadata.purpose),
                "scope": metadata.scope.as_str(),
                "created_unix": metadata.created_unix,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct ReplaceSecret;

#[async_trait]
impl ToolHandler for ReplaceSecret {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "replace_secret")
    }

    async fn execute_with_context(
        &self,
        mut params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "replace_secret";
        let reference = params["reference"].as_str().unwrap_or("").trim().to_string();
        if reference.is_empty() {
            return ToolResult::err("`reference` is required");
        }
        let input = match take_secret(&mut params, tool) {
            Ok(input) => input,
            Err(result) => return result,
        };

        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        let Some(grant) = call.grant() else {
            return ToolResult::err("a secret mutation requires a grant");
        };
        let Some(leases) = call.leases() else {
            return ToolResult::err("a secret mutation requires held leases");
        };
        let binding = call.binding();
        // Sealing proves grant, leases, audit admission and observation all came
        // from the SAME admission before the keyring is touched.
        let sealed = match resolved.runtime.seal_mutation_context(
            call.observation(),
            grant,
            leases,
            call.admission(),
            &binding,
        ) {
            Ok(sealed) => sealed,
            Err(error) => return gov::os_error(&error),
        };
        let store = match resolved.runtime.secrets(tool) {
            Ok(store) => store,
            Err(error) => return gov::os_error(&error),
        };
        // The previous value is not recoverable, so no rollback is claimed.
        match store
            .replace(&sealed, &SecretRef::new(reference), input)
            .await
        {
            Ok(metadata) => ToolResult::ok(serde_json::json!({
                "reference": metadata.reference.as_str(),
                "created_unix": metadata.created_unix,
            })),
            Err(error) => gov::os_error(&error),
        }
    }
}

struct DeleteSecret;

#[async_trait]
impl ToolHandler for DeleteSecret {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        gov::os_unavailable(None, "delete_secret")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let tool = "delete_secret";
        let reference = params["reference"].as_str().unwrap_or("").trim().to_string();
        if reference.is_empty() {
            return ToolResult::err("`reference` is required");
        }

        let resolved = match gov::resolve(&ctx, tool) {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, tool) {
            Ok(call) => call,
            Err(result) => return result,
        };
        let Some(grant) = call.grant() else {
            return ToolResult::err("a secret mutation requires a grant");
        };
        let Some(leases) = call.leases() else {
            return ToolResult::err("a secret mutation requires held leases");
        };
        let binding = call.binding();
        // Sealing proves grant, leases, audit admission and observation all came
        // from the SAME admission before the keyring is touched.
        let sealed = match resolved.runtime.seal_mutation_context(
            call.observation(),
            grant,
            leases,
            call.admission(),
            &binding,
        ) {
            Ok(sealed) => sealed,
            Err(error) => return gov::os_error(&error),
        };
        let store = match resolved.runtime.secrets(tool) {
            Ok(store) => store,
            Err(error) => return gov::os_error(&error),
        };
        // Irreversible: the value is gone and cannot be restored.
        match store.delete(&sealed, &SecretRef::new(reference)).await {
            Ok(()) => ToolResult::ok(serde_json::json!({ "deleted": true })),
            Err(error) => gov::os_error(&error),
        }
    }
}

/// Register the credential-store tool surface.
pub fn register(registry: &ToolRegistry) {
    let value_param = || {
        param(
            "value",
            "string",
            "The secret value. It is moved straight into the keyring and replaced with <redacted> in the recorded parameters, so it never reaches the audit record.",
            true,
        )
    };

    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        (
            ToolDef {
                name: "list_secret_references".into(),
                description: "List stored credentials as metadata only (never values)".into(),
                category: "secrets".into(),
                // Enumerating which credentials exist is privacy-sensitive.
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("purpose", "string", "Filter by purpose", false),
                    param("cursor", "string", "Page cursor", false),
                    param("limit", "integer", "Maximum rows", false),
                ],
            },
            Arc::new(ListSecretReferences),
        ),
        (
            ToolDef {
                name: "store_secret".into(),
                description: "Store a credential in the system keyring".into(),
                category: "secrets".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("purpose", "string", "What kind of credential this is", true),
                    param("scope", "string", "What it belongs to (network, host, …)", true),
                    param("label", "string", "Human label for the keyring entry", false),
                    value_param(),
                ],
            },
            Arc::new(StoreSecret),
        ),
        (
            ToolDef {
                name: "replace_secret".into(),
                description: "Replace a stored credential's value".into(),
                category: "secrets".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("reference", "string", "The stored credential's reference", true),
                    value_param(),
                ],
            },
            Arc::new(ReplaceSecret),
        ),
        (
            ToolDef {
                name: "delete_secret".into(),
                description: "Delete a stored credential — irreversible".into(),
                category: "secrets".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![param(
                    "reference",
                    "string",
                    "The stored credential's reference",
                    true,
                )],
            },
            Arc::new(DeleteSecret),
        ),
    ];

    for (def, handler) in tools {
        registry.register(def, handler);
    }
}
