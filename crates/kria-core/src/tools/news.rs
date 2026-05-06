//! News tools — delegate to the Python sidecar news processor.
//!
//! GREEN tools (auto-execute, no approval needed):
//!   search_news       → keyword search across deduplicated, trust-scored articles
//!   fetch_article     → extract full text from a news URL
//!   list_news_sources → which sources are being polled and when
//!   news_status       → poller health + DB stats

use crate::infra::ToolResult;
use crate::safety::RiskLevel;
use crate::sidecar::SidecarBridge;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;

const SIDECAR_REQUEST_TIMEOUT_SECS: u64 = 20;

fn parse_input<T: DeserializeOwned>(params: serde_json::Value) -> Result<T, ToolResult> {
    let normalized = if params.is_null() {
        serde_json::json!({})
    } else {
        params
    };

    serde_json::from_value(normalized)
        .map_err(|error| ToolResult::err(format!("invalid parameters: {error}")))
}

fn require_non_empty(value: &str, field: &str) -> Result<(), ToolResult> {
    if value.trim().is_empty() {
        return Err(ToolResult::err(format!("{field} is required")));
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
enum ToolExecutionError {
    #[error("news sidecar request timed out for method '{method}' after {timeout_secs}s")]
    SidecarTimeout { method: String, timeout_secs: u64 },
    #[error("news sidecar request failed for method '{method}': {reason}")]
    SidecarRequest { method: String, reason: String },
}

fn sidecar_error_result(error: ToolExecutionError) -> ToolResult {
    ToolResult::err(error.to_string())
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetNewsInput {
    query: String,
    #[serde(default)]
    hours: Option<u64>,
    #[serde(default)]
    freshness_mode: Option<String>,
    #[serde(default)]
    min_trust: Option<u64>,
    #[serde(default)]
    limit: Option<u64>,
    #[serde(default)]
    use_gdelt: Option<bool>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    region: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    source_profile: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FetchArticleInput {
    url: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EmptyInput {}

fn param(name: &str, ty: &str, desc: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        param_type: ty.into(),
        description: desc.into(),
        required,
        default: None,
    }
}

/// Shared sidecar handle, cloned cheaply into each handler.
#[derive(Clone)]
struct Sidecar(Arc<SidecarBridge>);

impl Sidecar {
    async fn call(
        &self,
        method: &'static str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ToolExecutionError> {
        match tokio::time::timeout(
            Duration::from_secs(SIDECAR_REQUEST_TIMEOUT_SECS),
            self.0.request(method, params),
        )
        .await
        {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(ToolExecutionError::SidecarRequest {
                method: method.to_string(),
                reason: error.to_string(),
            }),
            Err(_) => Err(ToolExecutionError::SidecarTimeout {
                method: method.to_string(),
                timeout_secs: SIDECAR_REQUEST_TIMEOUT_SECS,
            }),
        }
    }
}

// ── search_news ────────────────────────────────────────────────────────────────

struct SearchNews(Sidecar);

#[async_trait]
impl ToolHandler for SearchNews {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: GetNewsInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.query, "query") {
            return error;
        }

        let payload = match serde_json::to_value(input) {
            Ok(payload) => payload,
            Err(error) => {
                return ToolResult::err(format!(
                    "failed to serialize search_news input payload: {error}"
                ));
            }
        };

        match self.0.call("news.search", payload).await {
            Ok(value) => ToolResult::ok(value),
            Err(error) => sidecar_error_result(error),
        }
    }
}

// ── fetch_article ──────────────────────────────────────────────────────────────

struct FetchArticle(Sidecar);

#[async_trait]
impl ToolHandler for FetchArticle {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: FetchArticleInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.url, "url") {
            return error;
        }

        let payload = match serde_json::to_value(input) {
            Ok(payload) => payload,
            Err(error) => {
                return ToolResult::err(format!(
                    "failed to serialize fetch_article input payload: {error}"
                ));
            }
        };

        match self.0.call("news.fetch_article", payload).await {
            Ok(value) => ToolResult::ok(value),
            Err(error) => sidecar_error_result(error),
        }
    }
}

// ── list_news_sources ──────────────────────────────────────────────────────────

struct ListNewsSources(Sidecar);

#[async_trait]
impl ToolHandler for ListNewsSources {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let _input: EmptyInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        match self.0.call("news.list_sources", serde_json::json!({})).await {
            Ok(value) => ToolResult::ok(value),
            Err(error) => sidecar_error_result(error),
        }
    }
}

// ── news_status ────────────────────────────────────────────────────────────────

struct NewsStatus(Sidecar);

#[async_trait]
impl ToolHandler for NewsStatus {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let _input: EmptyInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        match self.0.call("news.get_status", serde_json::json!({})).await {
            Ok(value) => ToolResult::ok(value),
            Err(error) => sidecar_error_result(error),
        }
    }
}

// ── Register ───────────────────────────────────────────────────────────────────

pub fn register(reg: &ToolRegistry, bridge: Arc<SidecarBridge>) {
    let sc = Sidecar(bridge);

    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        (
            ToolDef {
                name: "search_news".into(),
                description: "Search recent news articles for any topic. Returns deduplicated, \
                    trust-scored results from curated RSS sources plus optional GDELT coverage. \
                    Supports freshness-aware ranking and regional source preference (for example \
                    India-focused authentic coverage). Results are clustered by story so you see \
                    one entry per event with a cross-reference count. Always use this before \
                    summarising news.".into(),
                category: "news".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("query",     "string",  "Topic, keywords, or question to search for", true),
                    param("hours",     "integer", "How many hours back to search (default depends on freshness_mode: live=6, recent=24, archive=168; max: 336)", false),
                    param("freshness_mode", "string", "Freshness policy: live | recent | archive (default: recent)", false),
                    param("min_trust", "integer", "Minimum source tier: 1=wire services only, 2=major outlets, 3=all sources including GDELT (default: 3)", false),
                    param("limit",     "integer", "Max number of stories to return (default: 10)", false),
                    param("use_gdelt", "boolean", "Also query GDELT live for broader coverage (default: true)", false),
                    param("country",   "string",  "Optional preferred country ISO code (e.g. IN, US)", false),
                    param("region",    "string",  "Optional preferred region tag (e.g. south-asia, europe)", false),
                    param("language",  "string",  "Optional preferred language code (e.g. en)", false),
                    param("source_profile", "string", "Source profile: balanced | authentic | global_authentic | india | india_authentic", false),
                ],
            },
            Arc::new(SearchNews(sc.clone())),
        ),
        (
            ToolDef {
                name: "fetch_article".into(),
                description: "Fetch and extract the full text of a news article from a URL. \
                    Use this after search_news to read the complete story from a result's URL. \
                    Returns clean article text, author, date, and metadata.".into(),
                category: "news".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("url", "string", "Full URL of the article to fetch", true),
                ],
            },
            Arc::new(FetchArticle(sc.clone())),
        ),
        (
            ToolDef {
                name: "list_news_sources".into(),
                description: "List all news sources being monitored, their trust tier, \
                    and when they were last polled. Useful for transparency about where \
                    news data comes from.".into(),
                category: "news".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(ListNewsSources(sc.clone())),
        ),
        (
            ToolDef {
                name: "news_status".into(),
                description: "Get news poller status: total articles indexed, how many from \
                    the last 24h, and DB health. Use to check if the news system is working.".into(),
                category: "news".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(NewsStatus(sc.clone())),
        ),
    ];

    for (def, handler) in tools {
        reg.register(def, handler);
    }
}
