use super::config::N8nConfig;
use super::types::{N8nWorkflowConfig, N8nWorkflowStatus};
use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub enum N8nCatalogError {
    #[error("n8n integration is disabled")]
    Disabled,
    #[error("n8n base_url is empty")]
    MissingBaseUrl,
    #[error("n8n signing_secret is empty")]
    MissingSigningSecret,
    #[error("unknown n8n workflow '{0}'")]
    UnknownWorkflow(String),
    #[error("n8n workflow '{workflow_id}' is not approved for execution (status={status:?})")]
    WorkflowNotApproved {
        workflow_id: String,
        status: N8nWorkflowStatus,
    },
    #[error("n8n workflow '{workflow_id}' version mismatch: expected {expected}, got {actual}")]
    WorkflowVersionMismatch {
        workflow_id: String,
        expected: String,
        actual: String,
    },
    #[error("n8n workflow '{0}' has an empty endpoint_path")]
    MissingEndpointPath(String),
    #[error("duplicate n8n workflow id '{0}'")]
    DuplicateWorkflow(String),
}

#[derive(Debug, Clone)]
pub struct N8nCatalog {
    config: N8nConfig,
    workflows: HashMap<String, N8nWorkflowConfig>,
}

impl N8nCatalog {
    pub fn new(config: N8nConfig) -> Result<Self, N8nCatalogError> {
        if !config.enabled {
            return Err(N8nCatalogError::Disabled);
        }
        if config.base_url.trim().is_empty() {
            return Err(N8nCatalogError::MissingBaseUrl);
        }
        if config.signing_secret.trim().is_empty() {
            return Err(N8nCatalogError::MissingSigningSecret);
        }

        let mut workflows = HashMap::new();
        for workflow in &config.workflows {
            if workflows.contains_key(&workflow.workflow_id) {
                return Err(N8nCatalogError::DuplicateWorkflow(
                    workflow.workflow_id.clone(),
                ));
            }
            workflows.insert(workflow.workflow_id.clone(), workflow.clone());
        }

        Ok(Self { config, workflows })
    }

    pub fn config(&self) -> &N8nConfig {
        &self.config
    }

    pub fn get(&self, workflow_id: &str) -> Option<&N8nWorkflowConfig> {
        self.workflows.get(workflow_id)
    }

    pub fn workflows(&self) -> Vec<N8nWorkflowConfig> {
        let mut workflows = self.workflows.values().cloned().collect::<Vec<_>>();
        workflows.sort_by(|a, b| a.workflow_id.cmp(&b.workflow_id));
        workflows
    }

    pub fn resolve(
        &self,
        workflow_id: &str,
        requested_version: Option<&str>,
    ) -> Result<&N8nWorkflowConfig, N8nCatalogError> {
        let workflow = self
            .workflows
            .get(workflow_id)
            .ok_or_else(|| N8nCatalogError::UnknownWorkflow(workflow_id.to_string()))?;

        if !workflow.is_approved_for_execution() {
            return Err(N8nCatalogError::WorkflowNotApproved {
                workflow_id: workflow.workflow_id.clone(),
                status: workflow.status.clone(),
            });
        }

        if workflow.requires_direct_endpoint_path() && workflow.endpoint_path.trim().is_empty() {
            return Err(N8nCatalogError::MissingEndpointPath(
                workflow.workflow_id.clone(),
            ));
        }

        if let Some(actual) = requested_version {
            if actual != workflow.workflow_version {
                return Err(N8nCatalogError::WorkflowVersionMismatch {
                    workflow_id: workflow.workflow_id.clone(),
                    expected: workflow.workflow_version.clone(),
                    actual: actual.to_string(),
                });
            }
        }

        Ok(workflow)
    }

    pub fn endpoint_url(&self, workflow: &N8nWorkflowConfig) -> Result<String, N8nCatalogError> {
        let base = self.config.base_url.trim_end_matches('/');
        let path = workflow.endpoint_path.trim_start_matches('/');
        Ok(format!("{base}/{path}"))
    }
}
