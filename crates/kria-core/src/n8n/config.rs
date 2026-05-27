use super::types::N8nWorkflowConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct N8nConfig {
    pub enabled: bool,
    pub base_url: String,
    pub api_key: String,
    pub signing_secret: String,
    pub request_timeout_secs: u64,
    pub max_payload_bytes: usize,
    pub default_requested_by: String,
    pub workflows: Vec<N8nWorkflowConfig>,
}

impl Default for N8nConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: String::new(),
            api_key: String::new(),
            signing_secret: String::new(),
            request_timeout_secs: 30,
            max_payload_bytes: 64 * 1024,
            default_requested_by: "local-user".into(),
            workflows: Vec::new(),
        }
    }
}
