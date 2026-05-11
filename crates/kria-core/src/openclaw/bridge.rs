//! Content-Length framed MCP bridge communication.
//!
//! Communicates with the OpenClaw substrate's MCP bridge via stdin/stdout
//! using Content-Length framed JSON-RPC (standard MCP transport).
//!
//! # Protocol
//!
//! ```text
//! Content-Length: <n>\r\n
//! \r\n
//! <JSON-RPC message of n bytes>
//! ```
//!
//! # Thread Safety
//!
//! Each `McpBridge` instance owns a single container's stdio streams.
//! It is `Send` but not `Clone` — one bridge per container.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};

/// Errors from MCP bridge communication.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("tool call failed: {0}")]
    ToolCallFailed(String),
    #[error("timeout after {0}ms")]
    Timeout(u64),
    #[error("bridge not initialized")]
    NotInitialized,
}

/// JSON-RPC request.
#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<serde_json::Value>,
}

/// JSON-RPC response.
#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    id: u64,
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    error: Option<JsonRpcError>,
}

/// JSON-RPC error object.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(default)]
    data: Option<serde_json::Value>,
}

/// MCP bridge client for communicating with a container's MCP bridge.
pub struct McpBridge {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    /// Pending responses keyed by request ID.
    initialized: bool,
}

impl McpBridge {
    /// Create a new MCP bridge from a container's child process.
    pub fn new(child: &mut Child) -> Result<Self, BridgeError> {
        let stdin = child.stdin.take().ok_or_else(|| {
            BridgeError::Protocol("child process has no stdin".into())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            BridgeError::Protocol("child process has no stdout".into())
        })?;

        Ok(Self {
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            initialized: false,
        })
    }

    /// Initialize the MCP bridge (perform the `initialize` handshake).
    pub async fn initialize(&mut self) -> Result<serde_json::Value, BridgeError> {
        let response = self
            .send_request(
                "initialize",
                Some(serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": { "tools": {} },
                    "clientInfo": { "name": "kria", "version": "1.0.0" }
                })),
            )
            .await?;

        self.initialized = true;

        // Send initialized notification
        self.send_notification("notifications/initialized", None).await?;

        Ok(response)
    }

    /// List available tools from the bridge.
    pub async fn list_tools(&mut self) -> Result<Vec<McpToolDef>, BridgeError> {
        let response = self.send_request("tools/list", None).await?;

        let tools = response
            .get("tools")
            .and_then(|t| t.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| serde_json::from_value(v.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();

        Ok(tools)
    }

    /// Call a tool via the bridge.
    pub async fn call_tool(
        &mut self,
        name: &str,
        arguments: Option<serde_json::Value>,
        timeout: Duration,
    ) -> Result<ToolCallResult, BridgeError> {
        let params = serde_json::json!({
            "name": name,
            "arguments": arguments,
        });

        let response = tokio::time::timeout(
            timeout,
            self.send_request("tools/call", Some(params)),
        )
        .await
        .map_err(|_| BridgeError::Timeout(timeout.as_millis() as u64))??;

        // Check for error
        if let Some(error) = response.get("error") {
            let message = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            return Err(BridgeError::ToolCallFailed(message.to_string()));
        }

        // Parse the result
        let is_error = response
            .get("isError")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let content = response
            .get("content")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        Some(ContentBlock {
                            block_type: item.get("type")?.as_str()?.to_string(),
                            text: item.get("text").and_then(|t| t.as_str()).map(String::from),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(ToolCallResult { is_error, content })
    }

    /// Send a JSON-RPC request and wait for the response.
    async fn send_request(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, BridgeError> {
        let id = self.next_id;
        self.next_id += 1;

        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        };

        // Write Content-Length framed message
        let body = serde_json::to_string(&request)?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        self.stdin.write_all(header.as_bytes()).await?;
        self.stdin.write_all(body.as_bytes()).await?;
        self.stdin.flush().await?;

        // Read response
        let response = self.read_response().await?;

        if response.id != id {
            return Err(BridgeError::Protocol(format!(
                "response ID mismatch: expected {}, got {}",
                id, response.id
            )));
        }

        if let Some(error) = response.error {
            return Err(BridgeError::ToolCallFailed(format!(
                "[{}] {}",
                error.code, error.message
            )));
        }

        response
            .result
            .ok_or_else(|| BridgeError::Protocol("response has no result".into()))
    }

    /// Send a JSON-RPC notification (no response expected).
    async fn send_notification(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<(), BridgeError> {
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });

        let body = serde_json::to_string(&notification)?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        self.stdin.write_all(header.as_bytes()).await?;
        self.stdin.write_all(body.as_bytes()).await?;
        self.stdin.flush().await?;

        Ok(())
    }

    /// Read a Content-Length framed response from stdout.
    async fn read_response(&mut self) -> Result<JsonRpcResponse, BridgeError> {
        // Read headers until empty line
        let mut content_length: Option<usize> = None;

        loop {
            let mut line = String::new();
            self.stdout.read_line(&mut line).await?;

            let trimmed = line.trim();
            if trimmed.is_empty() {
                break;
            }

            if let Some(value) = trimmed.strip_prefix("Content-Length:") {
                content_length = Some(value.trim().parse().map_err(|_| {
                    BridgeError::Protocol(format!("invalid Content-Length: {}", value))
                })?);
            }
        }

        let length = content_length.ok_or_else(|| {
            BridgeError::Protocol("missing Content-Length header".into())
        })?;

        // Read exactly `length` bytes
        let mut body = vec![0u8; length];
        self.stdout.read_exact(&mut body).await?;

        let response: JsonRpcResponse = serde_json::from_slice(&body)?;
        Ok(response)
    }
}

/// MCP tool definition from tools/list.
#[derive(Debug, Clone, Deserialize)]
pub struct McpToolDef {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub input_schema: Option<serde_json::Value>,
}

/// Result of a tool call.
#[derive(Debug, Clone)]
pub struct ToolCallResult {
    pub is_error: bool,
    pub content: Vec<ContentBlock>,
}

/// A content block in a tool call result.
#[derive(Debug, Clone)]
pub struct ContentBlock {
    pub block_type: String,
    pub text: Option<String>,
}

impl ToolCallResult {
    /// Get the concatenated text from all content blocks.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|b| b.text.as_deref())
            .collect::<Vec<_>>()
            .join("\n")
    }
}
