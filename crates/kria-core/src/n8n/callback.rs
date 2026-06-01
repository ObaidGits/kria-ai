use super::catalog::{N8nCatalog, N8nCatalogError};
use super::client::{sign_payload, N8nClientError};
use super::types::{N8nCallbackEnvelope, N8N_CALLBACK_SCHEMA_VERSION};
use std::time::{SystemTime, UNIX_EPOCH};

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
    #[error("n8n callback is too old: age_ms={age_ms}, max_age_ms={max_age_ms}")]
    CallbackTooOld { age_ms: u128, max_age_ms: u128 },
    #[error(
        "n8n callback timestamp is too far in the future: skew_ms={skew_ms}, max_skew_ms={max_skew_ms}"
    )]
    CallbackFromFuture { skew_ms: u128, max_skew_ms: u128 },
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
    validate_callback_freshness(
        &envelope,
        current_unix_ms(),
        u128::from(
            catalog
                .config()
                .callback_freshness_window_secs
                .saturating_mul(1000),
        ),
        u128::from(
            catalog
                .config()
                .future_callback_skew_secs
                .saturating_mul(1000),
        ),
    )?;

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

fn validate_callback_freshness(
    envelope: &N8nCallbackEnvelope,
    now_ms: u128,
    max_age_ms: u128,
    max_future_skew_ms: u128,
) -> Result<(), N8nCallbackError> {
    if envelope.occurred_at_ms > now_ms {
        let skew_ms = envelope.occurred_at_ms - now_ms;
        if max_future_skew_ms > 0 && skew_ms > max_future_skew_ms {
            return Err(N8nCallbackError::CallbackFromFuture {
                skew_ms,
                max_skew_ms: max_future_skew_ms,
            });
        }
        return Ok(());
    }

    let age_ms = now_ms - envelope.occurred_at_ms;
    if max_age_ms > 0 && age_ms > max_age_ms {
        return Err(N8nCallbackError::CallbackTooOld { age_ms, max_age_ms });
    }

    Ok(())
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

fn current_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default()
}
