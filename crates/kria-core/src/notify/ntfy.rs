//! ntfy push client (HTTP pub-sub).
//!
//! ntfy (<https://ntfy.sh>, self-hostable) is a simple HTTP publish/subscribe
//! service with official mobile apps. KRIA publishes to a user-chosen topic;
//! the phone subscribes. Keep the topic private (it acts as a shared secret)
//! and prefer a self-hosted instance for anything sensitive.

use serde::{Deserialize, Serialize};

/// ntfy notification priority (maps to the `Priority` header, 1–5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NtfyPriority {
    Min,
    Low,
    Default,
    High,
    Max,
}

impl NtfyPriority {
    fn header_value(&self) -> &'static str {
        match self {
            NtfyPriority::Min => "1",
            NtfyPriority::Low => "2",
            NtfyPriority::Default => "3",
            NtfyPriority::High => "4",
            NtfyPriority::Max => "5",
        }
    }
}

impl Default for NtfyPriority {
    fn default() -> Self {
        NtfyPriority::Default
    }
}

/// Configuration for the ntfy push integration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NtfyConfig {
    pub enabled: bool,
    /// Base server URL, e.g. `https://ntfy.sh` or a self-hosted instance.
    pub server_url: String,
    /// Topic to publish to (treat as a private secret).
    pub topic: String,
    /// Optional bearer/access token for protected (self-hosted) instances.
    pub auth_token: String,
    pub default_priority: NtfyPriority,
}

impl Default for NtfyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            server_url: "https://ntfy.sh".to_string(),
            topic: String::new(),
            auth_token: String::new(),
            default_priority: NtfyPriority::Default,
        }
    }
}

/// A single push message. Body should be a short human-readable summary only.
#[derive(Debug, Clone)]
pub struct NtfyMessage {
    pub title: String,
    pub body: String,
    pub priority: NtfyPriority,
    pub tags: Vec<String>,
    /// Optional URL opened when the notification is tapped.
    pub click: Option<String>,
}

impl NtfyMessage {
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            priority: NtfyPriority::Default,
            tags: Vec::new(),
            click: None,
        }
    }

    pub fn with_priority(mut self, p: NtfyPriority) -> Self {
        self.priority = p;
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }
}

/// Pure request description (URL + headers + body), independent of the HTTP
/// client — kept separate so it can be unit-tested without a network call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestParts {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// Client that publishes notifications to an ntfy server.
#[derive(Clone)]
pub struct NtfyClient {
    config: NtfyConfig,
    http: reqwest::Client,
}

impl NtfyClient {
    pub fn new(config: NtfyConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.config.enabled && !self.config.topic.is_empty()
    }

    /// Build the HTTP request parts for a message (testable, no I/O).
    pub fn build_request_parts(&self, msg: &NtfyMessage) -> RequestParts {
        let url = format!(
            "{}/{}",
            self.config.server_url.trim_end_matches('/'),
            self.config.topic
        );
        let mut headers = vec![
            ("Title".to_string(), sanitize_header(&msg.title)),
            ("Priority".to_string(), msg.priority.header_value().to_string()),
        ];
        if !msg.tags.is_empty() {
            headers.push(("Tags".to_string(), msg.tags.join(",")));
        }
        if let Some(click) = &msg.click {
            headers.push(("Click".to_string(), click.clone()));
        }
        if !self.config.auth_token.is_empty() {
            headers.push((
                "Authorization".to_string(),
                format!("Bearer {}", self.config.auth_token),
            ));
        }
        RequestParts {
            url,
            headers,
            body: msg.body.clone(),
        }
    }

    /// Publish a notification. No-op (Ok) when disabled or unconfigured.
    pub async fn publish(&self, msg: &NtfyMessage) -> Result<(), String> {
        if !self.is_enabled() {
            return Ok(());
        }
        let parts = self.build_request_parts(msg);
        let mut req = self.http.post(&parts.url).body(parts.body);
        for (k, v) in &parts.headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("ntfy send failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("ntfy returned status {}", resp.status()));
        }
        Ok(())
    }
}

/// Strip newlines from header values (ntfy headers are single-line).
fn sanitize_header(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> NtfyClient {
        NtfyClient::new(NtfyConfig {
            enabled: true,
            server_url: "https://ntfy.example.com/".to_string(),
            topic: "kria-private-abc".to_string(),
            auth_token: "tok_123".to_string(),
            default_priority: NtfyPriority::Default,
        })
    }

    #[test]
    fn builds_url_without_double_slash() {
        let parts = client().build_request_parts(&NtfyMessage::new("t", "b"));
        assert_eq!(parts.url, "https://ntfy.example.com/kria-private-abc");
    }

    #[test]
    fn includes_priority_title_and_auth() {
        let msg = NtfyMessage::new("Task done", "Report ready")
            .with_priority(NtfyPriority::High)
            .with_tags(vec!["white_check_mark".to_string()]);
        let parts = client().build_request_parts(&msg);
        assert!(parts
            .headers
            .iter()
            .any(|(k, v)| k == "Priority" && v == "4"));
        assert!(parts
            .headers
            .iter()
            .any(|(k, v)| k == "Title" && v == "Task done"));
        assert!(parts
            .headers
            .iter()
            .any(|(k, v)| k == "Authorization" && v == "Bearer tok_123"));
        assert!(parts.headers.iter().any(|(k, v)| k == "Tags" && v == "white_check_mark"));
        assert_eq!(parts.body, "Report ready");
    }

    #[test]
    fn disabled_when_topic_empty() {
        let c = NtfyClient::new(NtfyConfig {
            enabled: true,
            topic: String::new(),
            ..Default::default()
        });
        assert!(!c.is_enabled());
    }

    #[test]
    fn sanitizes_multiline_title() {
        assert_eq!(sanitize_header("a\nb\rc"), "a b c");
    }
}
