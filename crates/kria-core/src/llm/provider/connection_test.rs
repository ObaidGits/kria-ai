//! Provider connection testing.
//!
//! Implements instant provider validation so users can immediately know
//! whether their credentials and endpoints are valid.

use super::config::{ProviderConfig, ProviderType};
use super::error::{ProviderError, ProviderErrorKind};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// Result of a connection test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionTestResult {
    /// Overall test status.
    pub status: ConnectionTestStatus,
    /// Human-readable message.
    pub message: String,
    /// Latency of the test request in milliseconds.
    pub latency_ms: Option<u64>,
    /// Models discovered during the test (if applicable).
    pub discovered_models: Vec<String>,
    /// Provider-specific diagnostics.
    pub diagnostics: Option<serde_json::Value>,
}

/// Connection test status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionTestStatus {
    /// Connection successful, provider is ready.
    Success,
    /// Authentication failed (invalid key).
    Unauthorized,
    /// Endpoint is unreachable.
    Unreachable,
    /// Request timed out.
    Timeout,
    /// Quota/billing issue.
    QuotaExceeded,
    /// Endpoint URL is malformed.
    InvalidEndpoint,
    /// Partial success (connected but some features unavailable).
    Degraded,
    /// Unknown error.
    Error,
}

impl ConnectionTestResult {
    pub fn success(message: impl Into<String>, latency_ms: u64) -> Self {
        Self {
            status: ConnectionTestStatus::Success,
            message: message.into(),
            latency_ms: Some(latency_ms),
            discovered_models: vec![],
            diagnostics: None,
        }
    }

    pub fn failure(status: ConnectionTestStatus, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            latency_ms: None,
            discovered_models: vec![],
            diagnostics: None,
        }
    }

    pub fn with_models(mut self, models: Vec<String>) -> Self {
        self.discovered_models = models;
        self
    }

    pub fn with_diagnostics(mut self, diag: serde_json::Value) -> Self {
        self.diagnostics = Some(diag);
        self
    }
}

/// Test a provider connection.
///
/// This performs a lightweight validation request appropriate for the provider type:
/// - Ollama: GET /api/tags (list models)
/// - OpenAI/Compatible: GET /models
/// - Gemini: GET /models
/// - Anthropic: POST /messages with minimal payload
pub async fn test_provider_connection(config: &ProviderConfig) -> ConnectionTestResult {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    let start = Instant::now();

    match config.provider_type {
        ProviderType::Ollama => test_ollama(&client, config).await,
        ProviderType::LlamaCpp => test_llama_cpp(&client, config).await,
        ProviderType::OpenAI | ProviderType::OpenAICompatible | ProviderType::OpenRouter => {
            test_openai_compatible(&client, config).await
        }
        ProviderType::Gemini => test_gemini(&client, config).await,
        ProviderType::Anthropic => test_anthropic(&client, config).await,
    }
    .map(|mut result| {
        if result.latency_ms.is_none() {
            result.latency_ms = Some(start.elapsed().as_millis() as u64);
        }
        result
    })
    .unwrap_or_else(|e| {
        let latency = start.elapsed().as_millis() as u64;
        ConnectionTestResult {
            status: error_to_status(&e),
            message: e.message.clone(),
            latency_ms: Some(latency),
            discovered_models: vec![],
            diagnostics: Some(serde_json::json!({
                "error_kind": format!("{:?}", e.kind),
                "status_code": e.status_code,
            })),
        }
    })
}

fn error_to_status(e: &ProviderError) -> ConnectionTestStatus {
    match e.kind {
        ProviderErrorKind::AuthFailure => ConnectionTestStatus::Unauthorized,
        ProviderErrorKind::Timeout => ConnectionTestStatus::Timeout,
        ProviderErrorKind::NetworkError | ProviderErrorKind::InvalidEndpoint => {
            ConnectionTestStatus::Unreachable
        }
        ProviderErrorKind::QuotaExceeded => ConnectionTestStatus::QuotaExceeded,
        _ => ConnectionTestStatus::Error,
    }
}

async fn test_ollama(
    client: &reqwest::Client,
    config: &ProviderConfig,
) -> Result<ConnectionTestResult, ProviderError> {
    let url = format!("{}/api/tags", config.endpoint.base_url.trim_end_matches('/'));

    let resp = client.get(&url).send().await.map_err(|e| {
        if e.is_timeout() {
            ProviderError::timeout("ollama")
        } else {
            ProviderError::network("ollama", &e.to_string())
        }
    })?;

    let status = resp.status().as_u16();
    if status != 200 {
        return Err(ProviderError::from_http_status(
            status,
            &format!("Ollama returned HTTP {status}"),
            "ollama",
        ));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ProviderError::new(ProviderErrorKind::InvalidResponse, e.to_string(), "ollama"))?;

    let models: Vec<String> = body["models"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|m| m["name"].as_str().map(|s| s.to_string()))
        .collect();

    let model_count = models.len();
    Ok(ConnectionTestResult::success(
        format!("Connected to Ollama ({model_count} models available)"),
        0,
    )
    .with_models(models))
}

async fn test_llama_cpp(
    client: &reqwest::Client,
    config: &ProviderConfig,
) -> Result<ConnectionTestResult, ProviderError> {
    let base = config.endpoint.base_url.trim_end_matches('/');
    // Try /v1/models first, then /health
    let url = if base.ends_with("/v1") {
        format!("{base}/models")
    } else {
        format!("{base}/v1/models")
    };

    let resp = client.get(&url).send().await.map_err(|e| {
        if e.is_timeout() {
            ProviderError::timeout("llama_cpp")
        } else {
            ProviderError::network("llama_cpp", &e.to_string())
        }
    })?;

    let status = resp.status().as_u16();
    if status != 200 {
        // Try /health as fallback
        let health_url = format!("{base}/health");
        let health_resp = client.get(&health_url).send().await.map_err(|e| {
            ProviderError::network("llama_cpp", &e.to_string())
        })?;
        if health_resp.status().is_success() {
            return Ok(ConnectionTestResult::success(
                "Connected to llama.cpp server (health OK)",
                0,
            ));
        }
        return Err(ProviderError::from_http_status(
            status,
            "llama.cpp server not responding on /v1/models or /health",
            "llama_cpp",
        ));
    }

    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    let models: Vec<String> = body["data"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
        .collect();

    let model_info = if models.is_empty() {
        "server ready".to_string()
    } else {
        format!("model: {}", models.join(", "))
    };

    Ok(ConnectionTestResult::success(
        format!("Connected to llama.cpp ({model_info})"),
        0,
    )
    .with_models(models))
}

async fn test_openai_compatible(
    client: &reqwest::Client,
    config: &ProviderConfig,
) -> Result<ConnectionTestResult, ProviderError> {
    let base = config.endpoint.base_url.trim_end_matches('/');
    let url = format!("{base}/models");
    let provider_name = config.provider_type.as_str();

    let mut req = client.get(&url);
    if !config.endpoint.api_key.is_empty() {
        req = req.bearer_auth(&config.endpoint.api_key);
    }
    for (k, v) in &config.endpoint.custom_headers {
        req = req.header(k.as_str(), v.as_str());
    }

    let resp = req.send().await.map_err(|e| {
        if e.is_timeout() {
            ProviderError::timeout(provider_name)
        } else {
            ProviderError::network(provider_name, &e.to_string())
        }
    })?;

    let status = resp.status().as_u16();
    if status != 200 {
        let body = resp.text().await.unwrap_or_default();
        return Err(ProviderError::from_http_status(status, &body, provider_name));
    }

    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    let models: Vec<String> = body["data"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
        .collect();

    let model_count = models.len();
    Ok(ConnectionTestResult::success(
        format!("Connected ({model_count} models available)"),
        0,
    )
    .with_models(models))
}

async fn test_gemini(
    client: &reqwest::Client,
    config: &ProviderConfig,
) -> Result<ConnectionTestResult, ProviderError> {
    let base = config.endpoint.base_url.trim_end_matches('/');
    let url = format!("{base}/models?key={}", config.endpoint.api_key);

    let resp = client.get(&url).send().await.map_err(|e| {
        if e.is_timeout() {
            ProviderError::timeout("gemini")
        } else {
            ProviderError::network("gemini", &e.to_string())
        }
    })?;

    let status = resp.status().as_u16();
    if status != 200 {
        let body = resp.text().await.unwrap_or_default();
        return Err(ProviderError::from_http_status(status, &body, "gemini"));
    }

    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    let models: Vec<String> = body["models"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|m| m["name"].as_str().map(|s| {
            // Strip "models/" prefix
            s.strip_prefix("models/").unwrap_or(s).to_string()
        }))
        .collect();

    let model_count = models.len();
    Ok(ConnectionTestResult::success(
        format!("Connected to Gemini ({model_count} models available)"),
        0,
    )
    .with_models(models))
}

async fn test_anthropic(
    client: &reqwest::Client,
    config: &ProviderConfig,
) -> Result<ConnectionTestResult, ProviderError> {
    // Anthropic doesn't have a /models endpoint, so we send a minimal message
    let base = config.endpoint.base_url.trim_end_matches('/');
    let url = format!("{base}/messages");

    // Use a model for testing — if none configured, use claude-sonnet
    let test_model = if config.active_model.is_empty() {
        "claude-sonnet-4-20250514"
    } else {
        &config.active_model
    };

    let payload = serde_json::json!({
        "model": test_model,
        "max_tokens": 1,
        "messages": [{"role": "user", "content": "hi"}]
    });

    let resp = client
        .post(&url)
        .header("x-api-key", &config.endpoint.api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                ProviderError::timeout("anthropic")
            } else {
                ProviderError::network("anthropic", &e.to_string())
            }
        })?;

    let status = resp.status().as_u16();
    if status == 401 || status == 403 {
        let body = resp.text().await.unwrap_or_default();
        return Err(ProviderError::from_http_status(status, &body, "anthropic"));
    }

    // Any 2xx or even 400 (bad request but auth worked) means connection is valid
    if status == 200 || status == 400 {
        // Known Anthropic models
        let models = vec![
            "claude-sonnet-4-20250514".to_string(),
            "claude-opus-4-20250514".to_string(),
            "claude-3-5-haiku-20241022".to_string(),
        ];
        return Ok(ConnectionTestResult::success("Connected to Anthropic API", 0)
            .with_models(models));
    }

    let body = resp.text().await.unwrap_or_default();
    Err(ProviderError::from_http_status(status, &body, "anthropic"))
}
