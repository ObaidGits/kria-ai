//! Normalized provider error system.
//!
//! All provider-specific errors are mapped to a unified error type with
//! structured classification for retry logic, user messaging, and telemetry.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Classification of provider errors for retry and handling logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderErrorKind {
    /// Authentication failure (invalid/expired API key).
    AuthFailure,
    /// Rate limit exceeded (429).
    RateLimited,
    /// Request timeout.
    Timeout,
    /// Network connectivity issue.
    NetworkError,
    /// Invalid model specified.
    InvalidModel,
    /// Context/input too large for the model.
    ContextTooLarge,
    /// Provider returned an invalid/unexpected response.
    InvalidResponse,
    /// Provider service is unavailable (5xx).
    ServiceUnavailable,
    /// Quota/billing exceeded.
    QuotaExceeded,
    /// Content was filtered/blocked by provider safety.
    ContentFiltered,
    /// The requested capability is not supported.
    UnsupportedCapability,
    /// Endpoint URL is malformed or unreachable.
    InvalidEndpoint,
    /// Provider-specific error not covered above.
    ProviderSpecific,
    /// Request was cancelled.
    Cancelled,
}

impl ProviderErrorKind {
    /// Whether this error type is retryable.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimited | Self::Timeout | Self::NetworkError | Self::ServiceUnavailable
        )
    }

    /// Suggested retry delay in milliseconds (0 = don't retry).
    pub fn suggested_retry_delay_ms(&self) -> u64 {
        match self {
            Self::RateLimited => 2000,
            Self::Timeout => 1000,
            Self::NetworkError => 500,
            Self::ServiceUnavailable => 5000,
            _ => 0,
        }
    }

    /// Human-readable category for UI display.
    pub fn user_category(&self) -> &'static str {
        match self {
            Self::AuthFailure => "Authentication Error",
            Self::RateLimited => "Rate Limited",
            Self::Timeout => "Request Timeout",
            Self::NetworkError => "Network Error",
            Self::InvalidModel => "Invalid Model",
            Self::ContextTooLarge => "Context Too Large",
            Self::InvalidResponse => "Invalid Response",
            Self::ServiceUnavailable => "Service Unavailable",
            Self::QuotaExceeded => "Quota Exceeded",
            Self::ContentFiltered => "Content Filtered",
            Self::UnsupportedCapability => "Unsupported Feature",
            Self::InvalidEndpoint => "Invalid Endpoint",
            Self::ProviderSpecific => "Provider Error",
            Self::Cancelled => "Cancelled",
        }
    }
}

/// Unified provider error with structured metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderError {
    /// Error classification.
    pub kind: ProviderErrorKind,
    /// Human-readable error message for the user.
    pub message: String,
    /// Provider that generated the error.
    pub provider: String,
    /// HTTP status code, if applicable.
    pub status_code: Option<u16>,
    /// Whether this error should be retried.
    pub retryable: bool,
    /// Suggested retry delay in milliseconds.
    pub retry_after_ms: Option<u64>,
    /// Provider-specific error code/type.
    pub provider_error_code: Option<String>,
}

impl ProviderError {
    pub fn new(
        kind: ProviderErrorKind,
        message: impl Into<String>,
        provider: impl Into<String>,
    ) -> Self {
        let retryable = kind.is_retryable();
        let retry_after_ms = if retryable {
            Some(kind.suggested_retry_delay_ms())
        } else {
            None
        };
        Self {
            kind,
            message: message.into(),
            provider: provider.into(),
            status_code: None,
            retryable,
            retry_after_ms,
            provider_error_code: None,
        }
    }

    pub fn with_status(mut self, code: u16) -> Self {
        self.status_code = Some(code);
        self
    }

    pub fn with_provider_code(mut self, code: impl Into<String>) -> Self {
        self.provider_error_code = Some(code.into());
        self
    }

    /// Classify an HTTP status code into an error kind.
    pub fn from_http_status(status: u16, body: &str, provider: &str) -> Self {
        let kind = match status {
            401 | 403 => ProviderErrorKind::AuthFailure,
            429 => ProviderErrorKind::RateLimited,
            404 => ProviderErrorKind::InvalidModel,
            413 => ProviderErrorKind::ContextTooLarge,
            402 => ProviderErrorKind::QuotaExceeded,
            500..=599 => ProviderErrorKind::ServiceUnavailable,
            _ => ProviderErrorKind::ProviderSpecific,
        };
        Self::new(kind, body, provider).with_status(status)
    }

    /// Create a timeout error.
    pub fn timeout(provider: &str) -> Self {
        Self::new(ProviderErrorKind::Timeout, "Request timed out", provider)
    }

    /// Create a network error.
    pub fn network(provider: &str, detail: &str) -> Self {
        Self::new(
            ProviderErrorKind::NetworkError,
            format!("Network error: {detail}"),
            provider,
        )
    }

    /// Create a cancelled error.
    pub fn cancelled(provider: &str) -> Self {
        Self::new(ProviderErrorKind::Cancelled, "Request cancelled", provider)
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] {}: {}",
            self.provider,
            self.kind.user_category(),
            self.message
        )
    }
}

impl std::error::Error for ProviderError {}
