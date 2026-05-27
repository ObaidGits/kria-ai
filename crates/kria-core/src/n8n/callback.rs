use super::catalog::{N8nCatalog, N8nCatalogError};
use super::client::{sign_payload, N8nClientError};
use super::types::{N8nCallbackEnvelope, N8N_CALLBACK_SCHEMA_VERSION};

#[derive(Debug, thiserror::Error)]
pub enum N8nCallbackError {
    #[error("n8n callback signature is missing")]
    MissingSignature,
    #[error("n8n callback signature is invalid")]
    InvalidSignature,
    #[error("n8n callback body is not valid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("n8n callback schema version mismatch: expected {expected}, got {actual}")]
    SchemaVersionMismatch { expected: String, actual: String },
    #[error("{0}")]
    Catalog(#[from] N8nCatalogError),
    #[error("failed to sign n8n callback payload: {0}")]
    Signing(#[from] N8nClientError),
}

pub fn parse_and_verify_callback(
    catalog: &N8nCatalog,
    body: &[u8],
    signature: &str,
) -> Result<N8nCallbackEnvelope, N8nCallbackError> {
    verify_callback_signature(catalog.config().signing_secret.as_bytes(), body, signature)?;

    let envelope: N8nCallbackEnvelope = serde_json::from_slice(body)?;
    if envelope.schema_version != N8N_CALLBACK_SCHEMA_VERSION {
        return Err(N8nCallbackError::SchemaVersionMismatch {
            expected: N8N_CALLBACK_SCHEMA_VERSION.into(),
            actual: envelope.schema_version,
        });
    }

    let workflow = catalog
        .get(&envelope.workflow_id)
        .ok_or_else(|| N8nCatalogError::UnknownWorkflow(envelope.workflow_id.clone()))?;

    if workflow.workflow_version != envelope.workflow_version {
        return Err(N8nCatalogError::WorkflowVersionMismatch {
            workflow_id: envelope.workflow_id.clone(),
            expected: workflow.workflow_version.clone(),
            actual: envelope.workflow_version.clone(),
        }
        .into());
    }

    Ok(envelope)
}

pub fn verify_callback_signature(
    secret: &[u8],
    body: &[u8],
    signature: &str,
) -> Result<(), N8nCallbackError> {
    if signature.trim().is_empty() {
        return Err(N8nCallbackError::MissingSignature);
    }

    let expected = sign_payload(secret, body)?;
    if constant_time_eq(expected.as_bytes(), signature.as_bytes()) {
        Ok(())
    } else {
        Err(N8nCallbackError::InvalidSignature)
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }

    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}
