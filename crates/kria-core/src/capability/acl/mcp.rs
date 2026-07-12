//! `McpProvider` — a plain MCP server exposed as a [`CapabilityProvider`].
//!
//! This is the second reference provider. It proves the boundary is genuinely
//! provider-neutral (not overfit to OpenClaw): any standards-compliant MCP
//! stdio server becomes a capability provider with **zero KRIA-core change**,
//! consumed with a conservatively-derived default descriptor (R3.3).
//!
//! It wraps the frozen [`crate::mcp::client::McpClient`] (the ONLY place that
//! transport lives). Descriptors are derived from the server's `tools/list`;
//! execution goes through `tools/call`. It advertises only the mandatory
//! protocol facets — an MCP server has no install/lifecycle, which is exactly
//! why lifecycle is an *optional* negotiated facet.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::capability::descriptor::CapabilityDescriptor;
use crate::capability::error::CapError;
use crate::capability::protocol::{
    ClientCapabilities, FeatureSet, ProtocolSession, ProtocolVersion, ProviderHealth,
};
use crate::capability::provider::{CapabilityOutcome, CapabilityProvider, CapabilityRequest};
use crate::capability::ProviderId;

use crate::mcp::client::{McpClient, McpServerState};
use crate::mcp::protocol::McpToolDef;

/// A plain MCP server behind the capability boundary.
pub struct McpProvider {
    id: ProviderId,
    client: Arc<McpClient>,
}

impl McpProvider {
    /// Spawn + initialize an MCP server and wrap it as a provider.
    ///
    /// `id` becomes the open-vocabulary provider id (conventionally
    /// `"mcp:<name>"`). `command`/`args` launch the stdio server.
    pub async fn connect(
        id: impl Into<String>,
        command: &str,
        args: &[String],
    ) -> Result<Self, CapError> {
        let id = id.into();
        let client = Arc::new(McpClient::new(&id));
        client
            .start(command, args, &HashMap::new())
            .await
            .map_err(|e| CapError::ProviderOffline(format!("mcp '{id}' start failed: {e}")))?;
        Ok(Self { id, client })
    }

    /// Wrap an already-started client (tests / shared clients).
    pub fn from_client(id: impl Into<String>, client: Arc<McpClient>) -> Self {
        Self {
            id: id.into(),
            client,
        }
    }

    /// Gracefully stop the underlying server.
    pub async fn shutdown(&self) {
        let _ = self.client.stop().await;
    }

    /// Derive a conservative `v1` descriptor from an MCP tool definition. A thin
    /// MCP server declares no effects, so the descriptor defaults to
    /// "unknown/elevated" (the permission engine will require approval) per R3.3.
    fn descriptor_from(&self, t: &McpToolDef) -> CapabilityDescriptor {
        let mut d = CapabilityDescriptor::minimal(
            self.id.clone(),
            t.name.clone(),
            t.name.clone(),
            t.description.clone().unwrap_or_default(),
            t.input_schema.clone(),
        );
        // Declare substrate for the neutral Brain (no name branching in core).
        d.extensions.insert(
            "kind".to_string(),
            serde_json::Value::String("mcp".to_string()),
        );
        d
    }
}

#[async_trait]
impl CapabilityProvider for McpProvider {
    fn provider_id(&self) -> &ProviderId {
        &self.id
    }

    async fn negotiate(&self, client: &ClientCapabilities) -> Result<ProtocolSession, CapError> {
        // A plain MCP server supports only the mandatory facets — no lifecycle,
        // no streaming. This is the baseline the protocol was designed to accept.
        Ok(client.negotiate(
            self.id.clone(),
            ProtocolVersion::CURRENT,
            FeatureSet::mandatory(),
            serde_json::Map::new(),
        ))
    }

    async fn describe(
        &self,
        _session: &ProtocolSession,
    ) -> Result<Vec<CapabilityDescriptor>, CapError> {
        // Force a fresh tools/list so newly-exposed tools are picked up.
        let tools = self.client.refresh_tools().await.map_err(|e| {
            CapError::Discovery(format!("mcp '{}' tools/list failed: {e}", self.id))
        })?;
        Ok(tools.iter().map(|t| self.descriptor_from(t)).collect())
    }

    async fn execute(&self, req: CapabilityRequest) -> Result<CapabilityOutcome, CapError> {
        let result = self
            .client
            .call_tool(&req.capability_id, Some(req.args.clone()))
            .await
            .map_err(|e| {
                CapError::Execute(format!("mcp call '{}' failed: {e}", req.capability_id))
            })?;

        // Concatenate text content parts.
        let text = result
            .content
            .iter()
            .filter_map(|c| c.text.clone())
            .collect::<Vec<_>>()
            .join("\n");

        if result.is_error {
            return Err(CapError::Execute(if text.is_empty() {
                format!("mcp tool '{}' reported an error", req.capability_id)
            } else {
                text
            }));
        }

        // Prefer structured JSON if the tool returned it, else the raw text.
        let value = serde_json::from_str::<serde_json::Value>(&text)
            .unwrap_or_else(|_| serde_json::Value::String(text));
        Ok(CapabilityOutcome::Value(value))
    }

    async fn health(&self) -> ProviderHealth {
        match self.client.state().await {
            McpServerState::Running => ProviderHealth::Ready,
            _ => ProviderHealth::Offline,
        }
    }
}
