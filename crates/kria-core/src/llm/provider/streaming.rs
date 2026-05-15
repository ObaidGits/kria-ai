//! Unified streaming abstraction.
//!
//! Normalizes token streaming across all providers into a single interface.
//! Handles provider-specific SSE formats, interruption, cancellation, and
//! reconnection without leaking implementation details upward.

use futures::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio_util::sync::CancellationToken;

/// A single chunk from a streaming response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChunk {
    /// The text content of this chunk.
    pub text: String,
    /// Whether this is the final chunk.
    pub is_final: bool,
    /// Tool call deltas (accumulated across chunks).
    pub tool_call_delta: Option<serde_json::Value>,
    /// Finish reason (only on final chunk).
    pub finish_reason: Option<String>,
    /// Token usage (only on final chunk, if provider reports it).
    pub usage: Option<StreamUsage>,
}

/// Token usage reported at end of stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// Unified stream wrapper that normalizes all provider streams.
///
/// Supports:
/// - Cancellation via `CancellationToken`
/// - Provider-independent chunk format
/// - Graceful termination on errors
pub struct UnifiedStream {
    inner: Pin<Box<dyn Stream<Item = Result<StreamChunk, StreamError>> + Send>>,
    cancel_token: Option<CancellationToken>,
    finished: bool,
}

/// Errors that can occur during streaming.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamError {
    /// Connection was lost mid-stream.
    ConnectionLost(String),
    /// Stream was cancelled.
    Cancelled,
    /// Provider returned an error mid-stream.
    ProviderError(String),
    /// Malformed chunk data.
    ParseError(String),
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConnectionLost(msg) => write!(f, "Connection lost: {msg}"),
            Self::Cancelled => write!(f, "Stream cancelled"),
            Self::ProviderError(msg) => write!(f, "Provider error: {msg}"),
            Self::ParseError(msg) => write!(f, "Parse error: {msg}"),
        }
    }
}

impl UnifiedStream {
    /// Create a new unified stream from a provider-specific stream.
    pub fn new(
        inner: Pin<Box<dyn Stream<Item = Result<StreamChunk, StreamError>> + Send>>,
        cancel_token: Option<CancellationToken>,
    ) -> Self {
        Self {
            inner,
            cancel_token,
            finished: false,
        }
    }

    /// Create a stream from a simple text token stream (legacy compatibility).
    pub fn from_text_stream(
        stream: Pin<Box<dyn Stream<Item = String> + Send>>,
        cancel_token: Option<CancellationToken>,
    ) -> Self {
        use futures::StreamExt;

        let mapped = stream.map(|text| {
            Ok(StreamChunk {
                text,
                is_final: false,
                tool_call_delta: None,
                finish_reason: None,
                usage: None,
            })
        });

        Self {
            inner: Box::pin(mapped),
            cancel_token,
            finished: false,
        }
    }

    /// Convert to a simple text stream for backward compatibility with `LlmBackend`.
    pub fn into_text_stream(self) -> Pin<Box<dyn Stream<Item = String> + Send>> {
        use futures::StreamExt;

        let stream = self.inner.filter_map(|result| async move {
            match result {
                Ok(chunk) if !chunk.text.is_empty() => Some(chunk.text),
                _ => None,
            }
        });

        Box::pin(stream)
    }

    /// Check if the stream has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancel_token
            .as_ref()
            .map(|t| t.is_cancelled())
            .unwrap_or(false)
    }
}

impl Stream for UnifiedStream {
    type Item = Result<StreamChunk, StreamError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.finished {
            return Poll::Ready(None);
        }

        // Check cancellation
        if self.is_cancelled() {
            self.finished = true;
            return Poll::Ready(Some(Err(StreamError::Cancelled)));
        }

        match self.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                if chunk.is_final {
                    self.finished = true;
                }
                Poll::Ready(Some(Ok(chunk)))
            }
            Poll::Ready(Some(Err(e))) => {
                self.finished = true;
                Poll::Ready(Some(Err(e)))
            }
            Poll::Ready(None) => {
                self.finished = true;
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
