use crate::infra::ToolResult;
use crate::safety::RiskLevel;
use crate::tools::exec::{CommandOutput, ExecWrapper, ToolExecutionError as ExecToolExecutionError};
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

const WEB_RETRY_ATTEMPTS: usize = 3;
const MAX_OUTPUT_BYTES: usize = 100 * 1024;
const COMMAND_TIMEOUT_SECS: u64 = 20;

fn backoff_delay(attempt: usize) -> Duration {
    let shift = attempt.min(4) as u32;
    let ms = 250u64.saturating_mul(1u64 << shift);
    Duration::from_millis(ms)
}

fn is_private_or_sensitive_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.octets()[0] == 0
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_multicast()
                || v6.is_unspecified()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum ToolExecutionError {
    #[error("{operation} unsafe url '{url}': {reason}")]
    UnsafeUrl {
        operation: &'static str,
        url: String,
        reason: String,
    },
    #[error("{operation} failed for '{url}': HTTP status {status}")]
    HttpStatus {
        operation: &'static str,
        url: String,
        status: u16,
    },
    #[error("{operation} request failed for '{url}': {reason}")]
    HttpRequest {
        operation: &'static str,
        url: String,
        reason: String,
    },
    #[error("{operation} failed for '{path}': {reason}")]
    Io {
        operation: &'static str,
        path: String,
        reason: String,
    },
    #[error("{operation} command '{command}' failed: {reason}")]
    Command {
        operation: &'static str,
        command: String,
        reason: String,
    },
    #[error("{operation} failed: {reason}")]
    Operation {
        operation: &'static str,
        reason: String,
    },
}

fn parse_input<T: DeserializeOwned>(params: serde_json::Value) -> Result<T, ToolResult> {
    serde_json::from_value(params)
        .map_err(|error| ToolResult::err(format!("invalid parameters: {error}")))
}

fn tool_error(error: ToolExecutionError) -> ToolResult {
    ToolResult::err(error.to_string())
}

fn op_error(operation: &'static str, reason: impl Into<String>) -> ToolResult {
    tool_error(ToolExecutionError::Operation {
        operation,
        reason: reason.into(),
    })
}

fn io_error(operation: &'static str, path: impl Into<String>, error: std::io::Error) -> ToolResult {
    tool_error(ToolExecutionError::Io {
        operation,
        path: path.into(),
        reason: error.to_string(),
    })
}

fn require_non_empty(value: &str, field: &'static str) -> Result<(), ToolResult> {
    if value.trim().is_empty() {
        return Err(ToolResult::err(format!("{field} is required")));
    }
    Ok(())
}

fn local_test_urls_enabled() -> bool {
    if !cfg!(debug_assertions) {
        return false;
    }
    std::env::var("KRIA_ALLOW_LOCAL_TEST_URLS")
        .ok()
        .map(|value| {
            let lower = value.trim().to_ascii_lowercase();
            lower == "1" || lower == "true" || lower == "yes"
        })
        .unwrap_or(false)
}

fn validate_safe_url(operation: &'static str, raw_url: &str) -> Result<reqwest::Url, ToolExecutionError> {
    let parsed = reqwest::Url::parse(raw_url).map_err(|error| ToolExecutionError::UnsafeUrl {
        operation,
        url: raw_url.to_string(),
        reason: format!("invalid url: {error}"),
    })?;

    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(ToolExecutionError::UnsafeUrl {
            operation,
            url: raw_url.to_string(),
            reason: "unsupported URL scheme (only http/https allowed)".to_string(),
        });
    }

    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(ToolExecutionError::UnsafeUrl {
            operation,
            url: raw_url.to_string(),
            reason: "URLs with embedded credentials are not allowed".to_string(),
        });
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| ToolExecutionError::UnsafeUrl {
            operation,
            url: raw_url.to_string(),
            reason: "URL host is missing".to_string(),
        })?
        .to_ascii_lowercase();

    let local_allowed = local_test_urls_enabled();
    if !local_allowed
        && (host == "localhost"
            || host.ends_with(".local")
            || host.ends_with(".internal")
            || host.ends_with(".localhost"))
    {
        return Err(ToolExecutionError::UnsafeUrl {
            operation,
            url: raw_url.to_string(),
            reason: "local/internal hosts are blocked".to_string(),
        });
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        if !local_allowed && is_private_or_sensitive_ip(ip) {
            return Err(ToolExecutionError::UnsafeUrl {
                operation,
                url: raw_url.to_string(),
                reason: "private/internal IP ranges are blocked".to_string(),
            });
        }
    }

    Ok(parsed)
}

fn build_http_client(
    operation: &'static str,
    timeout_secs: u64,
    follow_redirects: bool,
) -> Result<reqwest::Client, ToolResult> {
    if std::env::var("KRIA_EVAL_MODE").is_ok() {
        return Err(op_error(
            operation,
            "KRIA_EVAL_MODE active: HTTP mocking not yet implemented for requested URL",
        ));
    }

    let policy = if follow_redirects {
        reqwest::redirect::Policy::limited(5)
    } else {
        reqwest::redirect::Policy::none()
    };

    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(timeout_secs))
        .redirect(policy)
        .build()
        .map_err(|error| {
            op_error(
                operation,
                format!("http client initialization failed: {error}"),
            )
        })
}

fn ensure_status_below_400(
    operation: &'static str,
    url: &reqwest::Url,
    status: reqwest::StatusCode,
) -> Result<(), ToolExecutionError> {
    if status.as_u16() > 399 {
        return Err(ToolExecutionError::HttpStatus {
            operation,
            url: url.to_string(),
            status: status.as_u16(),
        });
    }
    Ok(())
}

fn map_http_request_error(
    operation: &'static str,
    url: &reqwest::Url,
    error: reqwest::Error,
) -> ToolExecutionError {
    ToolExecutionError::HttpRequest {
        operation,
        url: url.to_string(),
        reason: error.to_string(),
    }
}

fn param(name: &str, ty: &str, desc: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        param_type: ty.into(),
        description: desc.into(),
        required,
        default: None,
    }
}

fn default_web_max_results() -> usize {
    5
}

fn default_web_max_chars() -> usize {
    20_000
}

fn default_ping_count() -> u64 {
    4
}

fn default_download_max_size_mb() -> u64 {
    500
}

fn default_searxng_instance_url() -> String {
    "http://localhost:8888".to_string()
}

fn default_timezone() -> String {
    "UTC".to_string()
}

fn default_weather_location() -> String {
    "Berlin".to_string()
}

fn default_feed_url() -> String {
    "https://hnrss.org/frontpage".to_string()
}

fn default_news_max_items() -> usize {
    10
}

fn default_base_currency() -> String {
    "USD".to_string()
}

fn default_target_currency() -> String {
    "EUR".to_string()
}

fn default_amount() -> f64 {
    1.0
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchWebInput {
    query: String,
    #[serde(default = "default_web_max_results")]
    max_results: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetWebContentInput {
    url: String,
    #[serde(default = "default_web_max_chars")]
    max_chars: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CheckUrlStatusInput {
    url: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EmptyInput {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PingInput {
    host: String,
    #[serde(default = "default_ping_count")]
    count: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DnsLookupInput {
    domain: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DownloadFileInput {
    url: String,
    destination: String,
    #[serde(default = "default_download_max_size_mb")]
    max_size_mb: u64,
    #[serde(default)]
    overwrite: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearxngSearchInput {
    query: String,
    #[serde(default = "default_web_max_results")]
    max_results: usize,
    #[serde(default = "default_searxng_instance_url")]
    instance_url: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetCurrentTimeInput {
    #[serde(default = "default_timezone")]
    timezone: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetWeatherInput {
    #[serde(default = "default_weather_location")]
    location: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetNewsInput {
    #[serde(default = "default_feed_url")]
    feed_url: String,
    #[serde(default = "default_news_max_items")]
    max_items: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetExchangeRateInput {
    #[serde(default = "default_base_currency")]
    base_currency: String,
    #[serde(default = "default_target_currency")]
    target_currency: String,
    #[serde(default = "default_amount")]
    amount: f64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CalculateInput {
    expression: String,
}

fn exec_wrapper(timeout_secs: u64) -> ExecWrapper {
    ExecWrapper::new()
        .with_timeout(Duration::from_secs(timeout_secs))
        .with_max_output_bytes(MAX_OUTPUT_BYTES)
}

fn preferred_output(output: &CommandOutput) -> String {
    if output.stdout.trim().is_empty() {
        output.stderr.trim().to_string()
    } else {
        output.stdout.trim().to_string()
    }
}

fn non_zero_details(stderr: String, stdout: String) -> String {
    let stderr_trimmed = stderr.trim().to_string();
    if stderr_trimmed.is_empty() {
        stdout.trim().to_string()
    } else {
        stderr_trimmed
    }
}

fn format_exec_error(error: ExecToolExecutionError) -> String {
    match error {
        ExecToolExecutionError::NonZeroExit { stderr, stdout, .. } => {
            let details = non_zero_details(stderr, stdout);
            if details.is_empty() {
                "command exited with non-zero status".to_string()
            } else {
                details
            }
        }
        ExecToolExecutionError::TimedOut {
            timeout_secs,
            stderr,
            stdout,
            ..
        } => {
            let details = non_zero_details(stderr, stdout);
            if details.is_empty() {
                format!("command timed out after {timeout_secs}s")
            } else {
                format!("command timed out after {timeout_secs}s: {details}")
            }
        }
        other => other.to_string(),
    }
}

async fn run_command(operation: &'static str, program: &str, args: &[&str]) -> Result<CommandOutput, ToolResult> {
    exec_wrapper(COMMAND_TIMEOUT_SECS)
        .execute(program, args)
        .await
        .map_err(|error| {
            tool_error(ToolExecutionError::Command {
                operation,
                command: format!("{} {}", program, args.join(" ")),
                reason: format_exec_error(error),
            })
        })
}

async fn search_duckduckgo_lite(query: &str, max_results: usize) -> Result<Vec<String>, ToolExecutionError> {
    let operation = "web_search";
    if std::env::var("KRIA_EVAL_MODE").is_ok() {
        return Err(ToolExecutionError::Operation {
            operation,
            reason: "KRIA_EVAL_MODE active: HTTP mocking not yet implemented for requested URL"
                .to_string(),
        });
    }

    let endpoint = reqwest::Url::parse("https://lite.duckduckgo.com/lite/")
        .map_err(|error| ToolExecutionError::Operation {
            operation,
            reason: format!("invalid search endpoint: {error}"),
        })?;

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|error| ToolExecutionError::Operation {
            operation,
            reason: format!("client initialization failed: {error}"),
        })?;

    let mut last_error: Option<ToolExecutionError> = None;
    for attempt in 0..WEB_RETRY_ATTEMPTS {
        let response = client
            .get(endpoint.clone())
            .query(&[("q", query)])
            .header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
            .send()
            .await;

        match response {
            Ok(response) => {
                if let Err(status_error) =
                    ensure_status_below_400(operation, response.url(), response.status())
                {
                    last_error = Some(status_error);
                } else {
                    let text = response
                        .text()
                        .await
                        .map_err(|error| map_http_request_error(operation, &endpoint, error))?;

                    let document = scraper::Html::parse_document(&text);
                    let selector = scraper::Selector::parse("a.result-link, .result-snippet").ok();
                    let mut results = Vec::new();

                    if let Some(selector) = selector {
                        for element in document.select(&selector).take(max_results) {
                            let row = element.text().collect::<String>().trim().to_string();
                            if !row.is_empty() {
                                results.push(row);
                            }
                        }
                    }

                    return Ok(results);
                }
            }
            Err(error) => {
                last_error = Some(map_http_request_error(operation, &endpoint, error));
            }
        }

        if attempt + 1 < WEB_RETRY_ATTEMPTS {
            tokio::time::sleep(backoff_delay(attempt)).await;
        }
    }

    Err(last_error.unwrap_or(ToolExecutionError::Operation {
        operation,
        reason: "search failed after retries".to_string(),
    }))
}

struct WebSearch;
#[async_trait]
impl ToolHandler for WebSearch {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: SearchWebInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        let query = input.query.trim().to_string();
        if let Err(error) = require_non_empty(&query, "query") {
            return error;
        }

        let max_results = input.max_results.clamp(1, 50);
        match search_duckduckgo_lite(&query, max_results).await {
            Ok(results) => ToolResult::ok(serde_json::json!({
                "query": query,
                "results": results,
            })),
            Err(error) => tool_error(error),
        }
    }
}

struct FetchWebpage;
#[async_trait]
impl ToolHandler for FetchWebpage {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: GetWebContentInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        let raw_url = input.url.trim().to_string();
        if let Err(error) = require_non_empty(&raw_url, "url") {
            return error;
        }

        let safe_url = match validate_safe_url("fetch_webpage", &raw_url) {
            Ok(url) => url,
            Err(error) => return tool_error(error),
        };

        let max_chars = input.max_chars.clamp(1, 200_000);
        let content_limit =
            ((max_chars as u64).saturating_mul(8)).clamp(128 * 1024, 3 * 1024 * 1024);

        let client = match build_http_client("fetch_webpage", 15, true) {
            Ok(client) => client,
            Err(error) => return error,
        };

        let mut last_error: Option<ToolExecutionError> = None;
        for attempt in 0..WEB_RETRY_ATTEMPTS {
            let response = client
                .get(safe_url.clone())
                .header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
                .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
                .header("Accept-Language", "en-US,en;q=0.5")
                .send()
                .await;

            match response {
                Ok(response) => {
                    if let Err(status_error) =
                        ensure_status_below_400("fetch_webpage", response.url(), response.status())
                    {
                        last_error = Some(status_error);
                    } else {
                        let content_type = response
                            .headers()
                            .get(reqwest::header::CONTENT_TYPE)
                            .and_then(|value| value.to_str().ok())
                            .unwrap_or("")
                            .to_ascii_lowercase();

                        let is_binary = !content_type.is_empty()
                            && (content_type.starts_with("image/")
                                || content_type.starts_with("audio/")
                                || content_type.starts_with("video/")
                                || content_type.starts_with("font/"));
                        if is_binary {
                            return op_error(
                                "fetch_webpage",
                                format!("unsupported binary content type: {content_type}"),
                            );
                        }

                        if let Some(content_length) = response.content_length() {
                            if content_length > content_limit {
                                return op_error(
                                    "fetch_webpage",
                                    format!(
                                        "response too large: {} bytes (limit {} bytes)",
                                        content_length, content_limit
                                    ),
                                );
                            }
                        }

                        let response_url = response.url().clone();
                        let text = match response.text().await {
                            Ok(text) => text,
                            Err(error) => {
                                return tool_error(map_http_request_error(
                                    "fetch_webpage",
                                    &response_url,
                                    error,
                                ));
                            }
                        };

                        let document = scraper::Html::parse_document(&text);
                        let body_selector = scraper::Selector::parse("body").ok();
                        let body_text = body_selector
                            .and_then(|selector| {
                                document
                                    .select(&selector)
                                    .next()
                                    .map(|element| element.text().collect::<String>())
                            })
                            .unwrap_or(text);

                        let content = if body_text.len() > max_chars {
                            body_text[..max_chars].to_string()
                        } else {
                            body_text.clone()
                        };

                        return ToolResult::ok(serde_json::json!({
                            "url": raw_url,
                            "content": content.trim(),
                            "truncated": body_text.len() > max_chars,
                        }));
                    }
                }
                Err(error) => {
                    last_error = Some(map_http_request_error("fetch_webpage", &safe_url, error));
                }
            }

            if attempt + 1 < WEB_RETRY_ATTEMPTS {
                tokio::time::sleep(backoff_delay(attempt)).await;
            }
        }

        tool_error(last_error.unwrap_or(ToolExecutionError::Operation {
            operation: "fetch_webpage",
            reason: "request failed after retries".to_string(),
        }))
    }
}

struct CheckUrlStatus;
#[async_trait]
impl ToolHandler for CheckUrlStatus {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: CheckUrlStatusInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        let raw_url = input.url.trim().to_string();
        if let Err(error) = require_non_empty(&raw_url, "url") {
            return error;
        }

        let safe_url = match validate_safe_url("check_url_status", &raw_url) {
            Ok(url) => url,
            Err(error) => {
                return ToolResult::ok(serde_json::json!({
                    "url": raw_url,
                    "reachable": false,
                    "error": error.to_string(),
                }))
            }
        };

        let client = match build_http_client("check_url_status", 10, false) {
            Ok(client) => client,
            Err(error) => {
                return ToolResult::ok(serde_json::json!({
                    "url": raw_url,
                    "reachable": false,
                    "error": error.error.unwrap_or_else(|| "unknown error".to_string()),
                }))
            }
        };

        let mut last_error: Option<ToolExecutionError> = None;
        for attempt in 0..WEB_RETRY_ATTEMPTS {
            match client.head(safe_url.clone()).send().await {
                Ok(response) => {
                    let status = response.status().as_u16();
                    if status > 399 {
                        let mapped = ToolExecutionError::HttpStatus {
                            operation: "check_url_status",
                            url: response.url().to_string(),
                            status,
                        };
                        return ToolResult::ok(serde_json::json!({
                            "url": raw_url,
                            "status": status,
                            "reachable": false,
                            "error": mapped.to_string(),
                        }));
                    }

                    return ToolResult::ok(serde_json::json!({
                        "url": raw_url,
                        "status": status,
                        "reachable": response.status().is_success() || response.status().is_redirection(),
                    }));
                }
                Err(error) => {
                    last_error = Some(map_http_request_error("check_url_status", &safe_url, error));
                }
            }

            if attempt + 1 < WEB_RETRY_ATTEMPTS {
                tokio::time::sleep(backoff_delay(attempt)).await;
            }
        }

        ToolResult::ok(serde_json::json!({
            "url": raw_url,
            "reachable": false,
            "error": last_error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "status check failed after retries".to_string()),
        }))
    }
}

struct GetPublicIp;
#[async_trait]
impl ToolHandler for GetPublicIp {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let _input: EmptyInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        let operation = "get_public_ip";
        let url = match reqwest::Url::parse("https://api.ipify.org?format=json") {
            Ok(url) => url,
            Err(error) => return op_error(operation, format!("invalid endpoint: {error}")),
        };

        let client = match build_http_client(operation, 10, true) {
            Ok(client) => client,
            Err(error) => return error,
        };

        let response = match client.get(url.clone()).send().await {
            Ok(response) => response,
            Err(error) => return tool_error(map_http_request_error(operation, &url, error)),
        };

        if let Err(error) = ensure_status_below_400(operation, response.url(), response.status()) {
            return tool_error(error);
        }

        let body: serde_json::Value = match response.json().await {
            Ok(body) => body,
            Err(error) => {
                return op_error(operation, format!("failed to decode JSON response: {error}"));
            }
        };

        ToolResult::ok(body)
    }
}

struct PingHost;
#[async_trait]
impl ToolHandler for PingHost {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: PingInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        let host = input.host.trim().to_string();
        if let Err(error) = require_non_empty(&host, "host") {
            return error;
        }

        let count = input.count.clamp(1, 10).to_string();
        let args = vec!["-c".to_string(), count.clone(), host.clone()];
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();

        let output = match run_command("ping_host", "ping", &refs).await {
            Ok(output) => output,
            Err(error) => return error,
        };

        ToolResult::ok(serde_json::json!({
            "host": host,
            "count": count,
            "success": true,
            "output": preferred_output(&output),
            "exit_code": output.exit_code,
        }))
    }
}

struct DnsLookup;
#[async_trait]
impl ToolHandler for DnsLookup {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: DnsLookupInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        let domain = input.domain.trim().to_string();
        if let Err(error) = require_non_empty(&domain, "domain") {
            return error;
        }

        let args = vec!["+short".to_string(), domain.clone()];
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();

        let output = match run_command("dns_lookup", "dig", &refs).await {
            Ok(output) => output,
            Err(error) => return error,
        };

        let records: Vec<String> = output
            .stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToString::to_string)
            .collect();

        ToolResult::ok(serde_json::json!({
            "domain": domain,
            "records": records,
            "success": true,
        }))
    }
}

struct DownloadFile;
#[async_trait]
impl ToolHandler for DownloadFile {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: DownloadFileInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        let raw_url = input.url.trim().to_string();
        if let Err(error) = require_non_empty(&raw_url, "url") {
            return error;
        }
        if let Err(error) = require_non_empty(&input.destination, "destination") {
            return error;
        }

        let safe_url = match validate_safe_url("download_file", &raw_url) {
            Ok(url) => url,
            Err(error) => return tool_error(error),
        };

        if input.max_size_mb == 0 {
            return op_error("download_file", "max_size_mb must be greater than 0");
        }
        let max_bytes = input.max_size_mb.saturating_mul(1024 * 1024);

        match tokio::fs::metadata(&input.destination).await {
            Ok(metadata) => {
                if metadata.is_dir() {
                    return op_error(
                        "download_file",
                        format!("destination is a directory: {}", input.destination),
                    );
                }

                if !input.overwrite {
                    return ToolResult::ok(serde_json::json!({
                        "url": raw_url,
                        "destination": input.destination,
                        "size_bytes": metadata.len(),
                        "changed": false,
                        "already_in_desired_state": true,
                    }));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return io_error("download_file", input.destination.clone(), error),
        }

        let client = match build_http_client("download_file", 30, true) {
            Ok(client) => client,
            Err(error) => return error,
        };

        let mut last_error: Option<ToolExecutionError> = None;
        let mut response_opt = None;
        for attempt in 0..WEB_RETRY_ATTEMPTS {
            match client.get(safe_url.clone()).send().await {
                Ok(response) => {
                    if let Err(status_error) =
                        ensure_status_below_400("download_file", response.url(), response.status())
                    {
                        last_error = Some(status_error);
                    } else {
                        if let Some(content_length) = response.content_length() {
                            if content_length > max_bytes {
                                return op_error(
                                    "download_file",
                                    format!(
                                        "file too large: {} MB (max {} MB)",
                                        content_length / (1024 * 1024),
                                        input.max_size_mb
                                    ),
                                );
                            }
                        }
                        response_opt = Some(response);
                        break;
                    }
                }
                Err(error) => {
                    last_error = Some(map_http_request_error("download_file", &safe_url, error));
                }
            }

            if attempt + 1 < WEB_RETRY_ATTEMPTS {
                tokio::time::sleep(backoff_delay(attempt)).await;
            }
        }

        let response = match response_opt {
            Some(response) => response,
            None => {
                return tool_error(last_error.unwrap_or(ToolExecutionError::Operation {
                    operation: "download_file",
                    reason: "download failed after retries".to_string(),
                }));
            }
        };

        let response_url = response.url().clone();
        let bytes = match response.bytes().await {
            Ok(bytes) => bytes,
            Err(error) => {
                return tool_error(map_http_request_error("download_file", &response_url, error));
            }
        };

        if (bytes.len() as u64) > max_bytes {
            return op_error(
                "download_file",
                format!(
                    "file too large after download: {} MB (max {} MB)",
                    bytes.len() / (1024 * 1024),
                    input.max_size_mb
                ),
            );
        }

        if let Some(parent) = Path::new(&input.destination).parent() {
            if !parent.as_os_str().is_empty() {
                if let Err(error) = tokio::fs::create_dir_all(parent).await {
                    return io_error(
                        "download_file",
                        parent.to_string_lossy().to_string(),
                        error,
                    );
                }
            }
        }

        if let Err(error) = tokio::fs::write(&input.destination, &bytes).await {
            return io_error("download_file", input.destination.clone(), error);
        }

        ToolResult::ok(serde_json::json!({
            "url": raw_url,
            "destination": input.destination,
            "size_bytes": bytes.len(),
            "changed": true,
            "already_in_desired_state": false,
        }))
    }
}

struct SpeedTest;
#[async_trait]
impl ToolHandler for SpeedTest {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let _input: EmptyInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        let operation = "speed_test";
        let url = match reqwest::Url::parse("https://speed.cloudflare.com/__down?bytes=1048576") {
            Ok(url) => url,
            Err(error) => return op_error(operation, format!("invalid speed test url: {error}")),
        };

        let client = match build_http_client(operation, 20, true) {
            Ok(client) => client,
            Err(error) => return error,
        };

        let started = std::time::Instant::now();
        let response = match client.get(url.clone()).send().await {
            Ok(response) => response,
            Err(error) => return tool_error(map_http_request_error(operation, &url, error)),
        };

        if let Err(error) = ensure_status_below_400(operation, response.url(), response.status()) {
            return tool_error(error);
        }

        let bytes = match response.bytes().await {
            Ok(bytes) => bytes,
            Err(error) => return tool_error(map_http_request_error(operation, &url, error)),
        };

        let elapsed = started.elapsed().as_secs_f64().max(0.001);
        let mbps = (bytes.len() as f64 * 8.0) / (elapsed * 1_000_000.0);

        ToolResult::ok(serde_json::json!({
            "download_mbps": format!("{:.1}", mbps),
            "bytes_downloaded": bytes.len(),
            "elapsed_seconds": format!("{:.2}", elapsed),
        }))
    }
}

struct SearxngSearch;
#[async_trait]
impl ToolHandler for SearxngSearch {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: SearxngSearchInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        let query = input.query.trim().to_string();
        if let Err(error) = require_non_empty(&query, "query") {
            return error;
        }

        let max_results = input.max_results.clamp(1, 50);
        let search_url = format!("{}/search", input.instance_url.trim_end_matches('/'));

        let client = match build_http_client("searxng_search", 10, true) {
            Ok(client) => client,
            Err(error) => return error,
        };

        let response = client
            .get(&search_url)
            .query(&[("q", query.as_str()), ("format", "json"), ("language", "en")])
            .header("User-Agent", "KRIA/0.1")
            .send()
            .await;

        match response {
            Ok(response) => {
                if let Err(status_error) =
                    ensure_status_below_400("searxng_search", response.url(), response.status())
                {
                    let searx_error = status_error.to_string();
                    tracing::warn!(instance = %input.instance_url, %searx_error, "searxng_search failed, falling back to DuckDuckGo");

                    match search_duckduckgo_lite(&query, max_results).await {
                        Ok(fallback_rows) => {
                            let results: Vec<serde_json::Value> = fallback_rows
                                .into_iter()
                                .map(|row| {
                                    serde_json::json!({
                                        "title": row,
                                        "url": serde_json::Value::Null,
                                        "snippet": serde_json::Value::Null,
                                        "engine": "duckduckgo-lite",
                                    })
                                })
                                .collect();

                            ToolResult::ok(serde_json::json!({
                                "query": query,
                                "results": results,
                                "count": results.len(),
                                "backend": "duckduckgo-lite",
                                "fallback_from": "searxng",
                                "fallback_reason": searx_error,
                            }))
                        }
                        Err(fallback_error) => ToolResult::err(format!(
                            "searxng_search failed ({searx_error}) and fallback web_search failed: {fallback_error}"
                        )),
                    }
                } else {
                    let body: serde_json::Value = match response.json().await {
                        Ok(body) => body,
                        Err(error) => {
                            return op_error(
                                "searxng_search",
                                format!("failed to decode searxng response: {error}"),
                            );
                        }
                    };

                    let empty_results = Vec::new();
                    let results: Vec<serde_json::Value> = body["results"]
                        .as_array()
                        .unwrap_or(&empty_results)
                        .iter()
                        .take(max_results)
                        .map(|row| {
                            serde_json::json!({
                                "title": row["title"],
                                "url": row["url"],
                                "snippet": row["content"],
                                "engine": row["engine"],
                            })
                        })
                        .collect();

                    ToolResult::ok(serde_json::json!({
                        "query": query,
                        "results": results,
                        "count": results.len(),
                    }))
                }
            }
            Err(error) => {
                let searx_error = ToolExecutionError::HttpRequest {
                    operation: "searxng_search",
                    url: search_url,
                    reason: error.to_string(),
                }
                .to_string();
                tracing::warn!(instance = %input.instance_url, %searx_error, "searxng_search failed, falling back to DuckDuckGo");

                match search_duckduckgo_lite(&query, max_results).await {
                    Ok(fallback_rows) => {
                        let results: Vec<serde_json::Value> = fallback_rows
                            .into_iter()
                            .map(|row| {
                                serde_json::json!({
                                    "title": row,
                                    "url": serde_json::Value::Null,
                                    "snippet": serde_json::Value::Null,
                                    "engine": "duckduckgo-lite",
                                })
                            })
                            .collect();

                        ToolResult::ok(serde_json::json!({
                            "query": query,
                            "results": results,
                            "count": results.len(),
                            "backend": "duckduckgo-lite",
                            "fallback_from": "searxng",
                            "fallback_reason": searx_error,
                        }))
                    }
                    Err(fallback_error) => ToolResult::err(format!(
                        "{searx_error}; fallback web_search failed: {fallback_error}"
                    )),
                }
            }
        }
    }
}

struct GetCurrentTime;
#[async_trait]
impl ToolHandler for GetCurrentTime {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: GetCurrentTimeInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        let tz_name = input.timezone;
        let now = chrono::Utc::now();

        let (display_time, tz_label) = match tz_name.to_uppercase().as_str() {
            "UTC" | "GMT" => (now.format("%Y-%m-%d %H:%M:%S").to_string(), "UTC"),
            "EST" | "US/EASTERN" => {
                let offset = chrono::FixedOffset::west_opt(5 * 3600).unwrap();
                (
                    now.with_timezone(&offset)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    "EST (UTC-5)",
                )
            }
            "CST" | "US/CENTRAL" => {
                let offset = chrono::FixedOffset::west_opt(6 * 3600).unwrap();
                (
                    now.with_timezone(&offset)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    "CST (UTC-6)",
                )
            }
            "MST" | "US/MOUNTAIN" => {
                let offset = chrono::FixedOffset::west_opt(7 * 3600).unwrap();
                (
                    now.with_timezone(&offset)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    "MST (UTC-7)",
                )
            }
            "PST" | "US/PACIFIC" => {
                let offset = chrono::FixedOffset::west_opt(8 * 3600).unwrap();
                (
                    now.with_timezone(&offset)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    "PST (UTC-8)",
                )
            }
            "CET" | "EUROPE/BERLIN" | "EUROPE/PARIS" => {
                let offset = chrono::FixedOffset::east_opt(3600).unwrap();
                (
                    now.with_timezone(&offset)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    "CET (UTC+1)",
                )
            }
            "JST" | "ASIA/TOKYO" => {
                let offset = chrono::FixedOffset::east_opt(9 * 3600).unwrap();
                (
                    now.with_timezone(&offset)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    "JST (UTC+9)",
                )
            }
            "IST" | "ASIA/KOLKATA" => {
                let offset = chrono::FixedOffset::east_opt(5 * 3600 + 1800).unwrap();
                (
                    now.with_timezone(&offset)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    "IST (UTC+5:30)",
                )
            }
            "PKT" | "ASIA/KARACHI" => {
                let offset = chrono::FixedOffset::east_opt(5 * 3600).unwrap();
                (
                    now.with_timezone(&offset)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    "PKT (UTC+5)",
                )
            }
            "AEST" | "AUSTRALIA/SYDNEY" => {
                let offset = chrono::FixedOffset::east_opt(10 * 3600).unwrap();
                (
                    now.with_timezone(&offset)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string(),
                    "AEST (UTC+10)",
                )
            }
            _ => {
                if let Ok(hours) = tz_name.parse::<i32>() {
                    let offset = chrono::FixedOffset::east_opt(hours * 3600).unwrap();
                    (
                        now.with_timezone(&offset)
                            .format("%Y-%m-%d %H:%M:%S")
                            .to_string(),
                        tz_name.as_str(),
                    )
                } else {
                    (
                        now.format("%Y-%m-%d %H:%M:%S").to_string(),
                        "UTC (unknown timezone, defaulting)",
                    )
                }
            }
        };

        ToolResult::ok(serde_json::json!({
            "datetime": display_time,
            "timezone": tz_label,
            "utc": now.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
            "unix_timestamp": now.timestamp(),
            "day_of_week": now.format("%A").to_string(),
        }))
    }
}

struct GetWeather;
#[async_trait]
impl ToolHandler for GetWeather {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: GetWeatherInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        let location = input.location;
        let client = match build_http_client("get_weather", 10, true) {
            Ok(client) => client,
            Err(error) => return error,
        };

        let geocode_url = match reqwest::Url::parse("https://geocoding-api.open-meteo.com/v1/search") {
            Ok(url) => url,
            Err(error) => return op_error("get_weather", format!("invalid geocoding url: {error}")),
        };

        let geocode_response = client
            .get(geocode_url.clone())
            .query(&[("name", location.as_str()), ("count", "1"), ("language", "en")])
            .send()
            .await;

        let (lat, lon, resolved_name) = match geocode_response {
            Ok(response) => {
                if let Err(error) =
                    ensure_status_below_400("get_weather", response.url(), response.status())
                {
                    return tool_error(error);
                }

                let body: serde_json::Value = match response.json().await {
                    Ok(body) => body,
                    Err(error) => {
                        return op_error(
                            "get_weather",
                            format!("failed to decode geocoding response: {error}"),
                        );
                    }
                };

                if let Some(result) = body["results"].as_array().and_then(|rows| rows.first()) {
                    (
                        result["latitude"].as_f64().unwrap_or(52.52),
                        result["longitude"].as_f64().unwrap_or(13.41),
                        result["name"].as_str().unwrap_or(location.as_str()).to_string(),
                    )
                } else {
                    return op_error("get_weather", format!("location not found: {location}"));
                }
            }
            Err(error) => {
                return tool_error(map_http_request_error("get_weather", &geocode_url, error));
            }
        };

        let weather_url = match reqwest::Url::parse("https://api.open-meteo.com/v1/forecast") {
            Ok(url) => url,
            Err(error) => return op_error("get_weather", format!("invalid forecast url: {error}")),
        };

        let weather_response = client
            .get(weather_url.clone())
            .query(&[
                ("latitude", lat.to_string()),
                ("longitude", lon.to_string()),
                (
                    "current",
                    "temperature_2m,relative_humidity_2m,wind_speed_10m,weather_code,is_day"
                        .to_string(),
                ),
                (
                    "daily",
                    "temperature_2m_max,temperature_2m_min,precipitation_sum,weather_code"
                        .to_string(),
                ),
                ("timezone", "auto".to_string()),
                ("forecast_days", "3".to_string()),
            ])
            .send()
            .await;

        match weather_response {
            Ok(response) => {
                if let Err(error) =
                    ensure_status_below_400("get_weather", response.url(), response.status())
                {
                    return tool_error(error);
                }

                let body: serde_json::Value = match response.json().await {
                    Ok(body) => body,
                    Err(error) => {
                        return op_error(
                            "get_weather",
                            format!("failed to decode weather response: {error}"),
                        );
                    }
                };

                let current = &body["current"];
                let weather_desc = match current["weather_code"].as_u64().unwrap_or(0) {
                    0 => "Clear sky",
                    1 => "Mainly clear",
                    2 => "Partly cloudy",
                    3 => "Overcast",
                    45 | 48 => "Foggy",
                    51..=55 => "Drizzle",
                    61..=65 => "Rain",
                    71..=75 => "Snow",
                    80..=82 => "Rain showers",
                    85 | 86 => "Snow showers",
                    95 => "Thunderstorm",
                    96 | 99 => "Thunderstorm with hail",
                    _ => "Unknown",
                };

                ToolResult::ok(serde_json::json!({
                    "location": resolved_name,
                    "coordinates": { "lat": lat, "lon": lon },
                    "current": {
                        "temperature_c": current["temperature_2m"],
                        "humidity_percent": current["relative_humidity_2m"],
                        "wind_speed_kmh": current["wind_speed_10m"],
                        "condition": weather_desc,
                        "is_day": current["is_day"],
                    },
                    "forecast": body["daily"],
                }))
            }
            Err(error) => tool_error(map_http_request_error("get_weather", &weather_url, error)),
        }
    }
}

struct GetNews;
#[async_trait]
impl ToolHandler for GetNews {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: GetNewsInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        let safe_url = match validate_safe_url("get_news", &input.feed_url) {
            Ok(url) => url,
            Err(error) => return tool_error(error),
        };

        let max_items = input.max_items.clamp(1, 50);
        let client = match build_http_client("get_news", 10, true) {
            Ok(client) => client,
            Err(error) => return error,
        };

        match client
            .get(safe_url.clone())
            .header("User-Agent", "KRIA/0.1")
            .send()
            .await
        {
            Ok(response) => {
                if let Err(error) =
                    ensure_status_below_400("get_news", response.url(), response.status())
                {
                    return tool_error(error);
                }

                let xml = match response.text().await {
                    Ok(xml) => xml,
                    Err(error) => return tool_error(map_http_request_error("get_news", &safe_url, error)),
                };

                let mut items = Vec::new();
                let document = scraper::Html::parse_document(&xml);

                if let Ok(item_selector) = scraper::Selector::parse("item") {
                    let title_selector = scraper::Selector::parse("title").ok();
                    let link_selector = scraper::Selector::parse("link").ok();
                    let description_selector = scraper::Selector::parse("description").ok();

                    for item in document.select(&item_selector).take(max_items) {
                        let title = title_selector
                            .as_ref()
                            .and_then(|selector| item.select(selector).next())
                            .map(|element| element.text().collect::<String>())
                            .unwrap_or_default();
                        let link = link_selector
                            .as_ref()
                            .and_then(|selector| item.select(selector).next())
                            .map(|element| element.text().collect::<String>())
                            .unwrap_or_default();
                        let description = description_selector
                            .as_ref()
                            .and_then(|selector| item.select(selector).next())
                            .map(|element| element.text().collect::<String>())
                            .unwrap_or_default();

                        items.push(serde_json::json!({
                            "title": title.trim(),
                            "link": link.trim(),
                            "description": if description.len() > 200 { &description[..200] } else { &description },
                        }));
                    }
                }

                if items.is_empty() {
                    if let Ok(entry_selector) = scraper::Selector::parse("entry") {
                        let title_selector = scraper::Selector::parse("title").ok();
                        let link_selector = scraper::Selector::parse("link").ok();

                        for entry in document.select(&entry_selector).take(max_items) {
                            let title = title_selector
                                .as_ref()
                                .and_then(|selector| entry.select(selector).next())
                                .map(|element| element.text().collect::<String>())
                                .unwrap_or_default();
                            let link = link_selector
                                .as_ref()
                                .and_then(|selector| entry.select(selector).next())
                                .and_then(|element| element.value().attr("href").map(String::from))
                                .unwrap_or_default();

                            items.push(serde_json::json!({
                                "title": title.trim(),
                                "link": link.trim(),
                            }));
                        }
                    }
                }

                ToolResult::ok(serde_json::json!({
                    "feed_url": input.feed_url,
                    "items": items,
                    "count": items.len(),
                }))
            }
            Err(error) => tool_error(map_http_request_error("get_news", &safe_url, error)),
        }
    }
}

struct GetExchangeRate;
#[async_trait]
impl ToolHandler for GetExchangeRate {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: GetExchangeRateInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        let base = input.base_currency.trim().to_uppercase();
        let target = input.target_currency.trim().to_uppercase();

        if base.len() != 3 || !base.chars().all(|value| value.is_ascii_alphabetic()) {
            return op_error("get_exchange_rate", "base_currency must be a 3-letter code");
        }
        if target.len() != 3 || !target.chars().all(|value| value.is_ascii_alphabetic()) {
            return op_error("get_exchange_rate", "target_currency must be a 3-letter code");
        }

        let client = match build_http_client("get_exchange_rate", 10, true) {
            Ok(client) => client,
            Err(error) => return error,
        };

        let url = match reqwest::Url::parse(&format!("https://open.er-api.com/v6/latest/{base}")) {
            Ok(url) => url,
            Err(error) => {
                return op_error("get_exchange_rate", format!("invalid exchange-rate url: {error}"));
            }
        };

        match client.get(url.clone()).send().await {
            Ok(response) => {
                if let Err(error) =
                    ensure_status_below_400("get_exchange_rate", response.url(), response.status())
                {
                    return tool_error(error);
                }

                let body: serde_json::Value = match response.json().await {
                    Ok(body) => body,
                    Err(error) => {
                        return op_error(
                            "get_exchange_rate",
                            format!("failed to decode exchange-rate response: {error}"),
                        );
                    }
                };

                if let Some(rate) = body["rates"][&target].as_f64() {
                    let converted = input.amount * rate;
                    ToolResult::ok(serde_json::json!({
                        "base": base,
                        "target": target,
                        "rate": rate,
                        "amount": input.amount,
                        "converted": format!("{:.2}", converted),
                        "last_update": body["time_last_update_utc"],
                    }))
                } else {
                    op_error("get_exchange_rate", format!("currency not found: {target}"))
                }
            }
            Err(error) => tool_error(map_http_request_error("get_exchange_rate", &url, error)),
        }
    }
}

struct Calculate;
#[async_trait]
impl ToolHandler for Calculate {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let input: CalculateInput = match parse_input(params) {
            Ok(input) => input,
            Err(error) => return error,
        };

        match eval_math(input.expression.trim()) {
            Ok(result) => ToolResult::ok(serde_json::json!({
                "expression": input.expression,
                "result": result,
            })),
            Err(error) => ToolResult::err(format!("calculation error: {error}")),
        }
    }
}

fn eval_math(expr: &str) -> Result<f64, String> {
    let expr = expr.trim();
    if expr.is_empty() {
        return Err("empty expression".into());
    }

    let allowed = |c: char| c.is_alphanumeric() || "+-*/%.^() ,".contains(c);
    if !expr.chars().all(allowed) {
        return Err("invalid characters in expression".into());
    }

    eval_expr(&mut expr.chars().peekable())
}

fn eval_expr(chars: &mut std::iter::Peekable<std::str::Chars>) -> Result<f64, String> {
    let mut result = eval_term(chars)?;
    loop {
        skip_spaces(chars);
        match chars.peek() {
            Some(&'+') => {
                chars.next();
                result += eval_term(chars)?;
            }
            Some(&'-') => {
                chars.next();
                result -= eval_term(chars)?;
            }
            _ => break,
        }
    }
    Ok(result)
}

fn eval_term(chars: &mut std::iter::Peekable<std::str::Chars>) -> Result<f64, String> {
    let mut result = eval_power(chars)?;
    loop {
        skip_spaces(chars);
        match chars.peek() {
            Some(&'*') => {
                chars.next();
                result *= eval_power(chars)?;
            }
            Some(&'/') => {
                chars.next();
                let divisor = eval_power(chars)?;
                if divisor == 0.0 {
                    return Err("division by zero".into());
                }
                result /= divisor;
            }
            Some(&'%') => {
                chars.next();
                let divisor = eval_power(chars)?;
                if divisor == 0.0 {
                    return Err("modulo by zero".into());
                }
                result %= divisor;
            }
            _ => break,
        }
    }
    Ok(result)
}

fn eval_power(chars: &mut std::iter::Peekable<std::str::Chars>) -> Result<f64, String> {
    let base = eval_unary(chars)?;
    skip_spaces(chars);
    if chars.peek() == Some(&'^') {
        chars.next();
        let exp = eval_unary(chars)?;
        Ok(base.powf(exp))
    } else {
        Ok(base)
    }
}

fn eval_unary(chars: &mut std::iter::Peekable<std::str::Chars>) -> Result<f64, String> {
    skip_spaces(chars);
    if chars.peek() == Some(&'-') {
        chars.next();
        Ok(-eval_atom(chars)?)
    } else if chars.peek() == Some(&'+') {
        chars.next();
        eval_atom(chars)
    } else {
        eval_atom(chars)
    }
}

fn eval_atom(chars: &mut std::iter::Peekable<std::str::Chars>) -> Result<f64, String> {
    skip_spaces(chars);
    if chars.peek() == Some(&'(') {
        chars.next();
        let result = eval_expr(chars)?;
        skip_spaces(chars);
        if chars.peek() == Some(&')') {
            chars.next();
        }
        return Ok(result);
    }

    let mut name = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_alphabetic() || c == '_' {
            name.push(c);
            chars.next();
        } else {
            break;
        }
    }

    if !name.is_empty() {
        let name_lower = name.to_lowercase();
        match name_lower.as_str() {
            "pi" => return Ok(std::f64::consts::PI),
            "e" => return Ok(std::f64::consts::E),
            "sqrt" | "abs" | "sin" | "cos" | "tan" | "log" | "ln" | "ceil" | "floor"
            | "round" => {
                skip_spaces(chars);
                if chars.peek() == Some(&'(') {
                    chars.next();
                    let arg = eval_expr(chars)?;
                    skip_spaces(chars);
                    if chars.peek() == Some(&')') {
                        chars.next();
                    }
                    return match name_lower.as_str() {
                        "sqrt" => Ok(arg.sqrt()),
                        "abs" => Ok(arg.abs()),
                        "sin" => Ok(arg.sin()),
                        "cos" => Ok(arg.cos()),
                        "tan" => Ok(arg.tan()),
                        "log" => Ok(arg.log10()),
                        "ln" => Ok(arg.ln()),
                        "ceil" => Ok(arg.ceil()),
                        "floor" => Ok(arg.floor()),
                        "round" => Ok(arg.round()),
                        _ => unreachable!(),
                    };
                }
                return Err(format!("expected '(' after function '{name}'"));
            }
            _ => return Err(format!("unknown function: {name}")),
        }
    }

    let mut num_str = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() || c == '.' {
            num_str.push(c);
            chars.next();
        } else {
            break;
        }
    }
    skip_spaces(chars);

    if num_str.is_empty() {
        return Err("expected number".into());
    }

    num_str
        .parse::<f64>()
        .map_err(|_| format!("invalid number: {num_str}"))
}

fn skip_spaces(chars: &mut std::iter::Peekable<std::str::Chars>) {
    while chars.peek() == Some(&' ') {
        chars.next();
    }
}

pub fn register(reg: &ToolRegistry) {
    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        (ToolDef {
            name: "web_search".into(), description: "Search the web using DuckDuckGo".into(),
            category: "internet".into(), default_tier: RiskLevel::Green, min_tier: "lite",
            parameters: vec![
                param("query", "string", "Search query", true),
                param("max_results", "integer", "Max results (default 5)", false),
            ],
        }, Arc::new(WebSearch)),
        (ToolDef {
            name: "fetch_webpage".into(), description: "Fetch and extract text from a webpage".into(),
            category: "internet".into(), default_tier: RiskLevel::Green, min_tier: "lite",
            parameters: vec![
                param("url", "string", "URL to fetch", true),
                param("max_chars", "integer", "Max chars (default 20000)", false),
            ],
        }, Arc::new(FetchWebpage)),
        (ToolDef {
            name: "check_url_status".into(), description: "Check if a URL is reachable".into(),
            category: "internet".into(), default_tier: RiskLevel::Green, min_tier: "lite",
            parameters: vec![param("url", "string", "URL to check", true)],
        }, Arc::new(CheckUrlStatus)),
        (ToolDef {
            name: "get_public_ip".into(), description: "Get public IP address".into(),
            category: "internet".into(), default_tier: RiskLevel::Green, min_tier: "lite",
            parameters: vec![],
        }, Arc::new(GetPublicIp)),
        (ToolDef {
            name: "ping_host".into(), description: "Ping a host and get response".into(),
            category: "internet".into(), default_tier: RiskLevel::Green, min_tier: "lite",
            parameters: vec![
                param("host", "string", "Hostname or IP", true),
                param("count", "integer", "Number of pings (default 4)", false),
            ],
        }, Arc::new(PingHost)),
        (ToolDef {
            name: "dns_lookup".into(), description: "DNS lookup for a domain".into(),
            category: "internet".into(), default_tier: RiskLevel::Green, min_tier: "lite",
            parameters: vec![param("domain", "string", "Domain name", true)],
        }, Arc::new(DnsLookup)),
        (ToolDef {
            name: "speed_test".into(), description: "Simple network speed test (download)".into(),
            category: "internet".into(), default_tier: RiskLevel::Green, min_tier: "standard",
            parameters: vec![],
        }, Arc::new(SpeedTest)),
        (ToolDef {
            name: "download_file".into(), description: "Download a file from URL to disk".into(),
            category: "internet".into(), default_tier: RiskLevel::Yellow, min_tier: "lite",
            parameters: vec![
                param("url", "string", "URL to download", true),
                param("destination", "string", "Local file path", true),
                param("max_size_mb", "integer", "Max file size in MB (default 500)", false),
                param("overwrite", "boolean", "Overwrite destination when file exists (default false)", false),
            ],
        }, Arc::new(DownloadFile)),
        (ToolDef {
            name: "searxng_search".into(),
            description: "Search the web via a SearXNG instance (structured results)".into(),
            category: "internet".into(), default_tier: RiskLevel::Green, min_tier: "lite",
            parameters: vec![
                param("query", "string", "Search query", true),
                param("max_results", "integer", "Max results (default 5)", false),
                param("instance_url", "string", "SearXNG URL (default http://localhost:8888)", false),
            ],
        }, Arc::new(SearxngSearch)),
        (ToolDef {
            name: "get_current_time".into(),
            description: "Get the current date, time, and day of week in any timezone".into(),
            category: "internet".into(), default_tier: RiskLevel::Green, min_tier: "lite",
            parameters: vec![
                param("timezone", "string", "Timezone name (UTC, EST, PST, JST, IST, PKT, CET, AEST) or offset like +5, -8. Default: UTC", false),
            ],
        }, Arc::new(GetCurrentTime)),
        (ToolDef {
            name: "get_weather".into(),
            description: "Get current weather and 3-day forecast for a location (Open-Meteo, free, no API key)".into(),
            category: "internet".into(), default_tier: RiskLevel::Green, min_tier: "lite",
            parameters: vec![
                param("location", "string", "City name or location", true),
            ],
        }, Arc::new(GetWeather)),
        (ToolDef {
            name: "get_news".into(),
            description: "Fetch latest news headlines from an RSS/Atom feed".into(),
            category: "internet".into(), default_tier: RiskLevel::Green, min_tier: "lite",
            parameters: vec![
                param("feed_url", "string", "RSS/Atom feed URL (default: Hacker News)", false),
                param("max_items", "integer", "Max items (default 10)", false),
            ],
        }, Arc::new(GetNews)),
        (ToolDef {
            name: "get_exchange_rate".into(),
            description: "Get currency exchange rates and convert amounts".into(),
            category: "internet".into(), default_tier: RiskLevel::Green, min_tier: "lite",
            parameters: vec![
                param("base_currency", "string", "Base currency code, e.g. USD", true),
                param("target_currency", "string", "Target currency code, e.g. EUR", true),
                param("amount", "number", "Amount to convert (default 1.0)", false),
            ],
        }, Arc::new(GetExchangeRate)),
        (ToolDef {
            name: "calculate".into(),
            description: "Evaluate a mathematical expression safely. Supports: +, -, *, /, %, ^, sqrt, abs, sin, cos, tan, log, ln, pi, e".into(),
            category: "internet".into(), default_tier: RiskLevel::Green, min_tier: "lite",
            parameters: vec![
                param("expression", "string", "Mathematical expression, e.g. '2^10 + sqrt(144)'", true),
            ],
        }, Arc::new(Calculate)),
    ];

    for (def, handler) in tools {
        reg.register(def, handler);
    }
}

#[cfg(test)]
mod tests {
    use super::validate_safe_url;

    #[test]
    fn allows_public_https_url() {
        assert!(validate_safe_url("test", "https://example.com/path?q=1").is_ok());
    }

    #[test]
    fn blocks_localhost_and_private_ips() {
        std::env::remove_var("KRIA_ALLOW_LOCAL_TEST_URLS");
        assert!(validate_safe_url("test", "http://localhost:8080").is_err());
        assert!(validate_safe_url("test", "http://127.0.0.1:8080").is_err());
        assert!(validate_safe_url("test", "http://10.0.0.5").is_err());
        assert!(validate_safe_url("test", "http://192.168.1.3").is_err());
    }

    #[test]
    fn blocks_non_http_schemes_and_embedded_credentials() {
        assert!(validate_safe_url("test", "file:///etc/passwd").is_err());
        assert!(validate_safe_url("test", "ftp://example.com/file.txt").is_err());
        assert!(validate_safe_url("test", "https://user:pass@example.com").is_err());
    }
}
