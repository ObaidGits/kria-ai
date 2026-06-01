use super::catalog::{N8nCatalog, N8nCatalogError};
use super::matching::WorkflowConfirmationFlow;
use super::schema::{validate_n8n_input_payload, N8nSchemaValidationError};
use super::types::{N8nCommandEnvelope, N8nInvocationResult, N8nToolRequest};
use hmac::{Hmac, Mac};
use reqwest::StatusCode;
use sha2::Sha256;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, thiserror::Error)]
pub enum N8nClientError {
    #[error("{0}")]
    Catalog(#[from] N8nCatalogError),
    #[error("n8n command payload exceeds max_payload_bytes ({actual} > {max})")]
    PayloadTooLarge { actual: usize, max: usize },
    #[error("{0}")]
    SchemaValidation(#[from] N8nSchemaValidationError),
    #[error("failed to serialize n8n command envelope: {0}")]
    Serialize(serde_json::Error),
    #[error("failed to sign n8n command envelope")]
    Signing,
    #[error("n8n HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("n8n response was not valid JSON: {0}")]
    ResponseJson(serde_json::Error),
    #[error("n8n workflow returned failure status {status}: {body}")]
    BadStatus { status: StatusCode, body: String },
}

#[derive(Clone)]
pub struct N8nClient {
    catalog: Arc<N8nCatalog>,
    http: reqwest::Client,
}

impl N8nClient {
    pub fn new(catalog: Arc<N8nCatalog>) -> Result<Self, N8nClientError> {
        let timeout = Duration::from_secs(catalog.config().request_timeout_secs.max(1));
        let http = reqwest::Client::builder().timeout(timeout).build()?;
        Ok(Self { catalog, http })
    }

    pub fn catalog(&self) -> &N8nCatalog {
        &self.catalog
    }

    pub async fn invoke(
        &self,
        request: N8nToolRequest,
    ) -> Result<N8nInvocationResult, N8nClientError> {
        let workflow = self
            .catalog
            .resolve(&request.workflow_id, request.workflow_version.as_deref())?;
        validate_n8n_input_payload(workflow, &request.input_payload)?;
        if WorkflowConfirmationFlow::workflow_requires_confirmation(workflow)
            && !payload_bool_true(&request.input_payload, "confirmed_by_user")
        {
            return Err(N8nClientError::SchemaValidation(
                N8nSchemaValidationError::Payload {
                    schema_ref: workflow.input_schema_ref.clone(),
                    issues: "confirmed_by_user must be true for required-review workflow".into(),
                },
            ));
        }

        let correlation_id = request
            .correlation_id
            .unwrap_or_else(|| Uuid::now_v7().to_string());
        let causation_id = request
            .causation_id
            .unwrap_or_else(|| correlation_id.clone());
        let idempotency_key = request
            .idempotency_key
            .unwrap_or_else(|| Uuid::now_v7().to_string());
        let requested_by = request
            .requested_by
            .unwrap_or_else(|| self.catalog.config().default_requested_by.clone());
        let deadline_ms = self
            .catalog
            .config()
            .request_timeout_secs
            .saturating_mul(1000);

        let envelope = N8nCommandEnvelope::new(
            workflow,
            request.input_payload,
            correlation_id,
            causation_id,
            idempotency_key,
            requested_by,
            deadline_ms,
        );

        let body = serde_json::to_vec(&envelope).map_err(N8nClientError::Serialize)?;
        let max = self.catalog.config().max_payload_bytes;
        if body.len() > max {
            return Err(N8nClientError::PayloadTooLarge {
                actual: body.len(),
                max,
            });
        }

        let signature = sign_payload(self.catalog.config().signing_secret.as_bytes(), &body)?;
        let url = self.catalog.endpoint_url(workflow)?;
        let api_key = self.catalog.config().api_key.clone();

        // Retry with exponential backoff (3 attempts: 0ms, 500ms, 1500ms)
        const MAX_RETRIES: u32 = 3;
        let mut last_error: Option<N8nClientError> = None;

        for attempt in 0..MAX_RETRIES {
            if attempt > 0 {
                let backoff_ms = match attempt {
                    1 => 500,
                    2 => 1500,
                    _ => 3000,
                };
                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                tracing::info!(
                    target: "n8n_client",
                    attempt,
                    backoff_ms,
                    workflow_id = %envelope.workflow_id,
                    "Retrying n8n invocation after transient failure"
                );
            }

            let mut retry_req = self
                .http
                .post(&url)
                .header("content-type", "application/json")
                .header("x-kria-signature", &signature)
                .header("x-kria-correlation-id", &envelope.correlation_id)
                .header("x-kria-idempotency-key", &envelope.idempotency_key)
                .body(body.clone());

            if !api_key.trim().is_empty() {
                retry_req = retry_req.bearer_auth(api_key.trim());
            }

            match retry_req.send().await {
                Ok(response) => {
                    let status = response.status();
                    let text = response.text().await.unwrap_or_default();

                    if status.is_success() {
                        let response_json = if text.trim().is_empty() {
                            serde_json::json!({})
                        } else {
                            serde_json::from_str(&text).map_err(N8nClientError::ResponseJson)?
                        };

                        return Ok(N8nInvocationResult {
                            workflow_id: envelope.workflow_id,
                            workflow_version: envelope.workflow_version,
                            correlation_id: envelope.correlation_id,
                            idempotency_key: envelope.idempotency_key,
                            status_code: status.as_u16(),
                            accepted: true,
                            response: response_json,
                        });
                    }

                    // Non-retryable status codes (4xx client errors except 429)
                    if status.is_client_error() && status.as_u16() != 429 {
                        return Err(N8nClientError::BadStatus { status, body: text });
                    }

                    // Retryable: 429, 5xx
                    last_error = Some(N8nClientError::BadStatus { status, body: text });
                }
                Err(e) => {
                    // Network errors are retryable
                    last_error = Some(N8nClientError::Http(e));
                }
            }
        }

        Err(last_error.unwrap_or(N8nClientError::BadStatus {
            status: reqwest::StatusCode::GATEWAY_TIMEOUT,
            body: "all retries exhausted".to_string(),
        }))
    }
}

fn payload_bool_true(value: &serde_json::Value, key: &str) -> bool {
    match value {
        serde_json::Value::Object(map) => map.iter().any(|(field, value)| {
            (field == key && value.as_bool().unwrap_or(false)) || payload_bool_true(value, key)
        }),
        serde_json::Value::Array(items) => items.iter().any(|item| payload_bool_true(item, key)),
        _ => false,
    }
}

pub fn sign_payload(secret: &[u8], payload: &[u8]) -> Result<String, N8nClientError> {
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| N8nClientError::Signing)?;
    mac.update(payload);
    Ok(format!(
        "sha256={}",
        hex::encode(mac.finalize().into_bytes())
    ))
}
