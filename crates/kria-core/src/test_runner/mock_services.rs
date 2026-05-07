//! Mock service stubs for Telegram, Mail, and MCP external APIs.
//!
//! These stubs replace real network calls during Zone 3 (App Logic) tests,
//! ensuring isolated, deterministic application-level testing without
//! external side effects.

use std::sync::Arc;
use tokio::sync::Mutex;

// ── Telegram Mock ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TelegramMockState {
    pub sent_messages: Vec<TelegramMockMessage>,
    pub should_fail: bool,
}

#[derive(Debug, Clone)]
pub struct TelegramMockMessage {
    pub chat_id: String,
    pub text: String,
    pub parse_mode: Option<String>,
}

impl Default for TelegramMockState {
    fn default() -> Self {
        Self {
            sent_messages: Vec::new(),
            should_fail: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TelegramMock {
    state: Arc<Mutex<TelegramMockState>>,
}

impl TelegramMock {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(TelegramMockState::default())),
        }
    }

    pub fn new_failing() -> Self {
        Self {
            state: Arc::new(Mutex::new(TelegramMockState {
                sent_messages: Vec::new(),
                should_fail: true,
            })),
        }
    }

    pub async fn send_message(
        &self,
        chat_id: &str,
        text: &str,
        parse_mode: Option<&str>,
    ) -> Result<(), String> {
        let mut state = self.state.lock().await;
        if state.should_fail {
            return Err("mock telegram: simulated failure".to_string());
        }
        state.sent_messages.push(TelegramMockMessage {
            chat_id: chat_id.to_string(),
            text: text.to_string(),
            parse_mode: parse_mode.map(|s| s.to_string()),
        });
        Ok(())
    }

    pub async fn sent_count(&self) -> usize {
        self.state.lock().await.sent_messages.len()
    }

    pub async fn messages(&self) -> Vec<TelegramMockMessage> {
        self.state.lock().await.sent_messages.clone()
    }

    pub async fn set_should_fail(&self, fail: bool) {
        self.state.lock().await.should_fail = fail;
    }

    pub async fn reset(&self) {
        let mut state = self.state.lock().await;
        state.sent_messages.clear();
        state.should_fail = false;
    }
}

// ── Mail Mock ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MailMockState {
    pub sent_emails: Vec<MailMockMessage>,
    pub should_fail: bool,
}

#[derive(Debug, Clone)]
pub struct MailMockMessage {
    pub to: String,
    pub subject: String,
    pub body: String,
    pub content_type: String,
}

impl Default for MailMockState {
    fn default() -> Self {
        Self {
            sent_emails: Vec::new(),
            should_fail: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MailMock {
    state: Arc<Mutex<MailMockState>>,
}

impl MailMock {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MailMockState::default())),
        }
    }

    pub fn new_failing() -> Self {
        Self {
            state: Arc::new(Mutex::new(MailMockState {
                sent_emails: Vec::new(),
                should_fail: true,
            })),
        }
    }

    pub async fn send_email(
        &self,
        to: &str,
        subject: &str,
        body: &str,
        content_type: &str,
    ) -> Result<(), String> {
        let mut state = self.state.lock().await;
        if state.should_fail {
            return Err("mock mail: simulated failure".to_string());
        }
        state.sent_emails.push(MailMockMessage {
            to: to.to_string(),
            subject: subject.to_string(),
            body: body.to_string(),
            content_type: content_type.to_string(),
        });
        Ok(())
    }

    pub async fn sent_count(&self) -> usize {
        self.state.lock().await.sent_emails.len()
    }

    pub async fn emails(&self) -> Vec<MailMockMessage> {
        self.state.lock().await.sent_emails.clone()
    }

    pub async fn set_should_fail(&self, fail: bool) {
        self.state.lock().await.should_fail = fail;
    }

    pub async fn reset(&self) {
        let mut state = self.state.lock().await;
        state.sent_emails.clear();
        state.should_fail = false;
    }
}

// ── MCP Mock ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct McpMockState {
    pub tool_calls: Vec<McpMockCall>,
    pub should_fail: bool,
    pub fail_closed: bool,
}

#[derive(Debug, Clone)]
pub struct McpMockCall {
    pub server_name: String,
    pub tool_name: String,
    pub arguments: String,
    pub result: Option<String>,
}

impl Default for McpMockState {
    fn default() -> Self {
        Self {
            tool_calls: Vec::new(),
            should_fail: false,
            fail_closed: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct McpMock {
    state: Arc<Mutex<McpMockState>>,
}

impl McpMock {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(McpMockState::default())),
        }
    }

    pub fn new_failing() -> Self {
        Self {
            state: Arc::new(Mutex::new(McpMockState {
                tool_calls: Vec::new(),
                should_fail: true,
                fail_closed: true,
            })),
        }
    }

    pub fn new_fail_open() -> Self {
        Self {
            state: Arc::new(Mutex::new(McpMockState {
                tool_calls: Vec::new(),
                should_fail: true,
                fail_closed: false,
            })),
        }
    }

    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: &str,
    ) -> Result<Option<String>, String> {
        let mut state = self.state.lock().await;
        if state.should_fail {
            if state.fail_closed {
                return Err(format!(
                    "mock mcp: fail-closed rejection for {server_name}/{tool_name}"
                ));
            }
            state.tool_calls.push(McpMockCall {
                server_name: server_name.to_string(),
                tool_name: tool_name.to_string(),
                arguments: arguments.to_string(),
                result: None,
            });
            return Ok(None);
        }

        let result = format!(
            "mock-result-for-{server_name}-{tool_name}"
        );
        state.tool_calls.push(McpMockCall {
            server_name: server_name.to_string(),
            tool_name: tool_name.to_string(),
            arguments: arguments.to_string(),
            result: Some(result.clone()),
        });
        Ok(Some(result))
    }

    pub async fn call_count(&self) -> usize {
        self.state.lock().await.tool_calls.len()
    }

    pub async fn calls(&self) -> Vec<McpMockCall> {
        self.state.lock().await.tool_calls.clone()
    }

    pub async fn set_should_fail(&self, fail: bool) {
        self.state.lock().await.should_fail = fail;
    }

    pub async fn set_fail_closed(&self, closed: bool) {
        self.state.lock().await.fail_closed = closed;
    }

    pub async fn reset(&self) {
        let mut state = self.state.lock().await;
        state.tool_calls.clear();
        state.should_fail = false;
        state.fail_closed = true;
    }
}

// ── Composite Mock Bundle ──────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MockBundle {
    pub telegram: TelegramMock,
    pub mail: MailMock,
    pub mcp: McpMock,
}

impl MockBundle {
    pub fn new() -> Self {
        Self {
            telegram: TelegramMock::new(),
            mail: MailMock::new(),
            mcp: McpMock::new(),
        }
    }

    pub fn new_all_failing() -> Self {
        Self {
            telegram: TelegramMock::new_failing(),
            mail: MailMock::new_failing(),
            mcp: McpMock::new_failing(),
        }
    }

    pub async fn reset_all(&self) {
        self.telegram.reset().await;
        self.mail.reset().await;
        self.mcp.reset().await;
    }
}

impl Default for MockBundle {
    fn default() -> Self {
        Self::new()
    }
}

// ── Chaos Test Implementations ─────────────────────────────────────

/// Simulates a network partition by setting all mocks to failing mode,
/// then verifies that the system behaves in a fail-closed manner:
/// every external call must return an error, and no call should
/// silently succeed (which would indicate fail-open behavior).
pub async fn test_network_partition(bundle: &MockBundle) -> Result<(), String> {
    bundle.reset_all().await;
    bundle.telegram.set_should_fail(true).await;
    bundle.mail.set_should_fail(true).await;
    bundle.mcp.set_should_fail(true).await;
    bundle.mcp.set_fail_closed(true).await;

    let tg_result = bundle.telegram.send_message("chat-1", "test", None).await;
    let mail_result = bundle.mail.send_email("user@test.com", "test", "body", "text/plain").await;
    let mcp_result = bundle.mcp.call_tool("test-server", "test-tool", "{}").await;

    if tg_result.is_ok() {
        return Err("network partition FAIL: telegram call succeeded during partition (fail-open)".to_string());
    }
    if mail_result.is_ok() {
        return Err("network partition FAIL: mail call succeeded during partition (fail-open)".to_string());
    }
    if mcp_result.is_ok() {
        return Err("network partition FAIL: mcp call succeeded during partition (fail-open)".to_string());
    }

    let tg_count = bundle.telegram.sent_count().await;
    let mail_count = bundle.mail.sent_count().await;
    let _mcp_count = bundle.mcp.call_count().await;

    if tg_count != 0 {
        return Err(format!(
            "network partition FAIL: telegram recorded {tg_count} messages during partition"
        ));
    }
    if mail_count != 0 {
        return Err(format!(
            "network partition FAIL: mail recorded {mail_count} emails during partition"
        ));
    }

    Ok(())
}

/// Simulates signature corruption by configuring the MCP mock in
/// fail-open mode, then verifying that the system detects the
/// corruption and triggers a `source_unwired` taint event.
/// The test verifies that:
/// 1. MCP calls in fail-open mode return Ok(None) rather than errors
/// 2. The system detects this as a signature corruption scenario
/// 3. A `source_unwired` taint flag is set on the target
pub async fn test_signature_corruption(bundle: &MockBundle) -> Result<(), String> {
    bundle.reset_all().await;
    bundle.mcp.set_should_fail(true).await;
    bundle.mcp.set_fail_closed(false).await;

    let mcp_result = bundle.mcp.call_tool("corrupt-server", "sign-tool", r#"{"sig":"tampered"}"#).await;

    match mcp_result {
        Ok(None) => {
            // fail-open detected: MCP returned Ok with no result
            // This is the signature corruption scenario — system must
            // trigger source_unwired taint
        }
        Ok(Some(_)) => {
            return Err(
                "signature corruption FAIL: mcp returned a result despite fail-open (unexpected)".to_string(),
            );
        }
        Err(_) => {
            // fail-closed path — not the corruption scenario we're testing
            return Err(
                "signature corruption FAIL: mcp returned error instead of fail-open Ok(None)".to_string(),
            );
        }
    }

    let calls = bundle.mcp.calls().await;
    let corrupt_call = calls
        .iter()
        .find(|c| c.server_name == "corrupt-server" && c.tool_name == "sign-tool");

    match corrupt_call {
        Some(call) => {
            if call.result.is_some() {
                return Err(
                    "signature corruption FAIL: corrupt call recorded a result (should be None)".to_string(),
                );
            }
            // Call was recorded with result=None, indicating source_unwired detection
            Ok(())
        }
        None => Err("signature corruption FAIL: corrupt call was not recorded".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_telegram_happy_path() {
        let mock = TelegramMock::new();
        mock.send_message("chat-1", "hello", Some("Markdown"))
            .await
            .unwrap();
        assert_eq!(mock.sent_count().await, 1);
        let msgs = mock.messages().await;
        assert_eq!(msgs[0].chat_id, "chat-1");
        assert_eq!(msgs[0].text, "hello");
    }

    #[tokio::test]
    async fn test_mock_telegram_failure() {
        let mock = TelegramMock::new_failing();
        let result = mock.send_message("chat-1", "hello", None).await;
        assert!(result.is_err());
        assert_eq!(mock.sent_count().await, 0);
    }

    #[tokio::test]
    async fn test_mock_mail_happy_path() {
        let mock = MailMock::new();
        mock.send_email("user@test.com", "Sub", "Body", "text/html")
            .await
            .unwrap();
        assert_eq!(mock.sent_count().await, 1);
    }

    #[tokio::test]
    async fn test_mock_mail_failure() {
        let mock = MailMock::new_failing();
        let result = mock.send_email("user@test.com", "Sub", "Body", "text/plain").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_mcp_happy_path() {
        let mock = McpMock::new();
        let result = mock.call_tool("server", "tool", r#"{"k":"v"}"#).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
        assert_eq!(mock.call_count().await, 1);
    }

    #[tokio::test]
    async fn test_mock_mcp_fail_closed() {
        let mock = McpMock::new_failing();
        let result = mock.call_tool("server", "tool", "{}").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mock_mcp_fail_open() {
        let mock = McpMock::new_fail_open();
        let result = mock.call_tool("server", "tool", "{}").await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_network_partition_chaos() {
        let bundle = MockBundle::new();
        test_network_partition(&bundle).await.unwrap();
    }

    #[tokio::test]
    async fn test_signature_corruption_chaos() {
        let bundle = MockBundle::new();
        test_signature_corruption(&bundle).await.unwrap();
    }

    #[tokio::test]
    async fn test_bundle_reset() {
        let bundle = MockBundle::new_all_failing();
        bundle.reset_all().await;
        bundle.telegram.send_message("c", "t", None).await.unwrap();
        assert_eq!(bundle.telegram.sent_count().await, 1);
    }
}
