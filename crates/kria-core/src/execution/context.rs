//! A7.7 Shared Execution Context — ONE context every executor reads/writes through.
//!
//! Holds the goal, variables, intermediate outputs, artifacts, execution metadata,
//! correlation IDs and cancellation. Backend-agnostic: no executor-specific fields.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

/// An artifact produced during execution (file path, blob handle, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub kind: String,
    pub uri: String,
}

/// Inner mutable state, guarded by a single RwLock.
#[derive(Default)]
struct ContextInner {
    /// Free-form variables set by the planner or executors.
    variables: HashMap<String, serde_json::Value>,
    /// Node output keyed by node id (intermediate outputs).
    outputs: HashMap<String, serde_json::Value>,
    /// Produced artifacts.
    artifacts: Vec<Artifact>,
    /// Arbitrary execution metadata.
    metadata: HashMap<String, String>,
}

/// The shared execution context (A7.7). Cheaply cloneable (Arc inside).
#[derive(Clone)]
pub struct ExecutionContext {
    /// Goal identifier this execution serves.
    pub goal_id: String,
    /// Correlation id tying planning → execution → events → audit.
    pub correlation_id: String,
    /// Cancellation token propagated to every executor.
    pub cancellation: CancellationToken,
    inner: Arc<RwLock<ContextInner>>,
}

impl ExecutionContext {
    pub fn new(goal_id: impl Into<String>, correlation_id: impl Into<String>) -> Self {
        Self {
            goal_id: goal_id.into(),
            correlation_id: correlation_id.into(),
            cancellation: CancellationToken::new(),
            inner: Arc::new(RwLock::new(ContextInner::default())),
        }
    }

    pub fn with_cancellation(mut self, token: CancellationToken) -> Self {
        self.cancellation = token;
        self
    }

    pub async fn set_var(&self, key: impl Into<String>, value: serde_json::Value) {
        self.inner.write().await.variables.insert(key.into(), value);
    }

    pub async fn get_var(&self, key: &str) -> Option<serde_json::Value> {
        self.inner.read().await.variables.get(key).cloned()
    }

    pub async fn set_output(&self, node_id: impl Into<String>, value: serde_json::Value) {
        self.inner
            .write()
            .await
            .outputs
            .insert(node_id.into(), value);
    }

    pub async fn get_output(&self, node_id: &str) -> Option<serde_json::Value> {
        self.inner.read().await.outputs.get(node_id).cloned()
    }

    pub async fn all_outputs(&self) -> HashMap<String, serde_json::Value> {
        self.inner.read().await.outputs.clone()
    }

    pub async fn add_artifact(&self, artifact: Artifact) {
        self.inner.write().await.artifacts.push(artifact);
    }

    pub async fn artifacts(&self) -> Vec<Artifact> {
        self.inner.read().await.artifacts.clone()
    }

    pub async fn set_meta(&self, key: impl Into<String>, value: impl Into<String>) {
        self.inner
            .write()
            .await
            .metadata
            .insert(key.into(), value.into());
    }

    pub async fn get_meta(&self, key: &str) -> Option<String> {
        self.inner.read().await.metadata.get(key).cloned()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
}
