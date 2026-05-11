use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub type CleanupFuture = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;
pub type CleanupHook = Box<dyn FnOnce() -> CleanupFuture + Send + 'static>;

/// Result envelope returned by every tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub data: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ToolResult {
    pub fn ok(data: serde_json::Value) -> Self {
        Self {
            success: true,
            data,
            error: None,
        }
    }

    pub fn ok_text(msg: impl Into<String>) -> Self {
        Self::ok(serde_json::Value::String(msg.into()))
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            data: serde_json::Value::Null,
            error: Some(msg.into()),
        }
    }

    pub fn err_with_data(msg: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            success: false,
            data,
            error: Some(msg.into()),
        }
    }
}

/// Execute a tool function with timeout and panic isolation.
pub async fn run_isolated<F, Fut>(
    name: &str,
    timeout: Duration,
    cancel_token: CancellationToken,
    cleanup: Option<CleanupHook>,
    f: F,
) -> ToolResult
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ToolResult> + Send + 'static,
{
    let tool_name = name.to_string();
    let mut cleanup = cleanup;
    let mut handle = tokio::spawn(async move { tokio::time::timeout(timeout, f()).await });

    tokio::select! {
        biased;
        _ = cancel_token.cancelled() => {
            tracing::info!(tool = %tool_name, "tool execution cancelled");
            handle.abort();
            let _ = handle.await;

            if let Some(cleanup_hook) = cleanup.take() {
                let _ = tokio::time::timeout(Duration::from_secs(5), cleanup_hook()).await;
            }

            ToolResult::err(format!("tool '{tool_name}' cancelled"))
        }
        join_result = &mut handle => {
            match join_result {
                Ok(Ok(result)) => result,
                Ok(Err(_elapsed)) => {
                    tracing::warn!(tool = %tool_name, "tool execution timed out");
                    ToolResult::err(format!(
                        "tool '{tool_name}' timed out after {}s",
                        timeout.as_secs()
                    ))
                }
                Err(join_err) => {
                    tracing::error!(tool = %tool_name, error = %join_err, "tool panicked");
                    ToolResult::err(format!("tool '{tool_name}' panicked: {join_err}"))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn cancellation_aborts_long_running_tool() {
        let cancel_token = CancellationToken::new();
        let completed = Arc::new(AtomicBool::new(false));
        let completed_clone = completed.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();

        let run = tokio::spawn(run_isolated(
            "fake-long-tool",
            Duration::from_secs(30),
            cancel_token.clone(),
            None,
            move || async move {
                let _ = started_tx.send(());
                tokio::time::sleep(Duration::from_secs(10)).await;
                completed_clone.store(true, Ordering::SeqCst);
                ToolResult::ok(serde_json::json!({"done": true}))
            },
        ));

        let _ = started_rx.await;
        cancel_token.cancel();

        let result = run.await.expect("run_isolated task should complete");
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("cancelled"));

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!completed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn cancellation_runs_cleanup_hook() {
        let cancel_token = CancellationToken::new();
        let cleaned = Arc::new(AtomicBool::new(false));
        let cleaned_clone = cleaned.clone();
        let cleanup: CleanupHook = Box::new(move || {
            Box::pin(async move {
                cleaned_clone.store(true, Ordering::SeqCst);
            })
        });

        let runner = tokio::spawn(run_isolated(
            "cleanup-tool",
            Duration::from_secs(30),
            cancel_token.clone(),
            Some(cleanup),
            move || async move {
                tokio::time::sleep(Duration::from_secs(10)).await;
                ToolResult::ok(serde_json::json!({"done": true}))
            },
        ));

        cancel_token.cancel();
        let result = runner.await.expect("run_isolated should complete");

        assert!(!result.success);
        assert!(cleaned.load(Ordering::SeqCst));
    }
}
