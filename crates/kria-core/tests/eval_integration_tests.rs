// ─────────────────────────────────────────────────────────────────────────────
//  eval_integration_tests.rs
//
//  Eval tests that properly follow the real usage flow:
//  1. Check if kria-server is running
//  2. Start it if not running
//  3. Wait for health
//  4. Send prompt via HTTP API
//  5. Detect hallucination, unavailable, wrong data
//  6. Verify correctness
//
//  Run with:
//    KRIA_EVAL_INTEGRATION=1 cargo test -p kria-core --test eval_integration_tests
// ─────────────────────────────────────────────────────────────────────────────

use serde_json::Value;

mod common;

// ═══════════════════════════════════════════════════════════════════════════
//  Server lifecycle (shared with quality_hallucination_tests)
// ═══════════════════════════════════════════════════════════════════════════

static SERVER_GUARD: std::sync::Mutex<Option<std::process::Child>> = std::sync::Mutex::new(None);

fn ensure_server_running() {
    // Check if already initialized
    {
        let guard = SERVER_GUARD.lock().unwrap();
        if guard.is_some() {
            return;
        }
    }

    let base_url = std::env::var("KRIA_EVAL_SERVER_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8088".to_string());

    if is_server_up(&base_url) {
        eprintln!("kria-server already running at {base_url}");
        return;
    }

    let server_bin = find_server_binary();
    let server_bin = match server_bin {
        Some(p) => p,
        None => {
            eprintln!("WARN: kria-server binary not found; eval integration tests will skip");
            return;
        }
    };

    eprintln!("Starting kria-server from {server_bin}...");
    let mut child = std::process::Command::new(&server_bin)
        .env("KRIA_LOG_LEVEL", "warn")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to spawn kria-server");

    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(60);
    while start.elapsed() < timeout {
        if is_server_up(&base_url) {
            eprintln!("kria-server ready in {:.1}s", start.elapsed().as_secs_f64());
            let mut guard = SERVER_GUARD.lock().unwrap();
            *guard = Some(child);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    eprintln!("WARN: kria-server did not become ready within 60s");
    let _ = child.kill();
}

fn is_server_up(base_url: &str) -> bool {
    std::process::Command::new("curl")
        .args(["-s", "--max-time", "2", &format!("{base_url}/health")])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn find_server_binary() -> Option<String> {
    let candidates = [
        "target/debug/kria-server",
        "target/release/kria-server",
        "/media/obaid/SSD/KRIA/target/debug/kria-server",
        "/media/obaid/SSD/KRIA/target/release/kria-server",
    ];
    for c in &candidates {
        if std::path::Path::new(c).exists() {
            return Some(c.to_string());
        }
    }
    None
}

// Drop guard to kill the server when tests complete
struct ServerGuard;
impl Drop for ServerGuard {
    fn drop(&mut self) {
        if let Some(mut child) = SERVER_GUARD.lock().unwrap().take() {
            eprintln!(
                "Shutting down eval integration test kria-server (pid {})",
                child.id()
            );
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Response issue detection
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
struct ResponseIssue {
    kind: IssueKind,
    description: String,
}

#[derive(Debug, Clone, PartialEq)]
enum IssueKind {
    Unavailable,
    Hallucination,
    WrongData,
    BashFallback,
}

fn detect_response_issues(response: &str, prompt: &str) -> Vec<ResponseIssue> {
    let mut issues = Vec::new();
    let response_lower = response.to_lowercase();
    let prompt_lower = prompt.to_lowercase();

    // Unavailable patterns
    let unavailable_patterns = [
        "i cannot access real-time",
        "i can't access real-time",
        "i do not have access to",
        "i'm sorry, but i cannot",
        "unable to access real-time",
        "don't have the ability",
        "please provide your",
        "you can install",
        "follow these steps",
    ];

    for pattern in &unavailable_patterns {
        if response_lower.contains(pattern) {
            let should_flag = prompt_lower.contains("weather")
                || prompt_lower.contains("news")
                || prompt_lower.contains("install")
                || prompt_lower.contains("check")
                || prompt_lower.contains("search");
            if should_flag {
                issues.push(ResponseIssue {
                    kind: IssueKind::Unavailable,
                    description: format!("Claimed unavailable: '{}'", pattern),
                });
            }
        }
    }

    // Bash fallback patterns
    if response.contains("```bash") || response.contains("```sh") {
        issues.push(ResponseIssue {
            kind: IssueKind::BashFallback,
            description: "Response contains bash code".to_string(),
        });
    }

    // Hallucination patterns
    if response_lower.contains("here are the steps to check")
        || response_lower.contains("based on my training")
        || response_lower.contains("as of my last training")
    {
        issues.push(ResponseIssue {
            kind: IssueKind::Hallucination,
            description: "Hallucination markers detected".to_string(),
        });
    }

    // Empty response
    if response.trim().len() < 5 {
        issues.push(ResponseIssue {
            kind: IssueKind::Hallucination,
            description: "Response too short or empty".to_string(),
        });
    }

    issues
}

// ═══════════════════════════════════════════════════════════════════════════
//  HTTP API client
// ═══════════════════════════════════════════════════════════════════════════

async fn send_prompt_via_api(prompt: &str) -> Result<Value, String> {
    let base_url = std::env::var("KRIA_EVAL_SERVER_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8088".to_string());

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| format!("HTTP client build failed: {e}"))?;

    let body = serde_json::json!({
        "session_id": "eval-integration",
        "message": prompt
    });

    let resp = client
        .post(format!("{base_url}/api/chat"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Server returned error: {}", resp.status()));
    }

    resp.json()
        .await
        .map_err(|e| format!("Failed to parse response: {e}"))
}

// ═══════════════════════════════════════════════════════════════════════════
//  Test cases
// ═══════════════════════════════════════════════════════════════════════════

fn eval_integration_enabled() -> bool {
    matches!(std::env::var("KRIA_EVAL_INTEGRATION").as_deref(), Ok("1"))
}

fn require_enabled() {
    if !eval_integration_enabled() {
        eprintln!("SKIP: set KRIA_EVAL_INTEGRATION=1 to run eval integration tests");
    }
}

macro_rules! eval_integration_guard {
    () => {
        if !eval_integration_enabled() {
            return;
        }
        require_enabled();
        ensure_server_running();
    };
}

#[tokio::test]
async fn eval_weather_must_use_tool() {
    eval_integration_guard!();
    let _guard = ServerGuard;

    let json = send_prompt_via_api("What is the weather in Mumbai?")
        .await
        .expect("eval integration: failed to get response from API");

    let tool = json["tool_calls"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|t| t["name"].as_str());

    let response = json["response"]
        .as_str()
        .or_else(|| json["message"].as_str())
        .unwrap_or("");

    // Check for response issues
    let issues = detect_response_issues(response, "What is the weather in Mumbai?");
    assert!(
        issues.is_empty(),
        "Weather response has issues: {:?}",
        issues
    );

    // Must use a tool
    let t = tool.unwrap_or("").to_lowercase();
    assert!(
        t.contains("weather") || t.contains("web_search") || t.contains("search"),
        "Weather prompt must use a weather/web_search tool, got: {}",
        t
    );
}

#[tokio::test]
async fn eval_news_must_use_tool() {
    eval_integration_guard!();
    let _guard = ServerGuard;

    let json = send_prompt_via_api("Give me latest AI news")
        .await
        .expect("eval integration: failed to get response from API");

    let tool = json["tool_calls"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|t| t["name"].as_str());

    let response = json["response"]
        .as_str()
        .or_else(|| json["message"].as_str())
        .unwrap_or("");

    let issues = detect_response_issues(response, "Give me latest AI news");
    assert!(issues.is_empty(), "News response has issues: {:?}", issues);

    let t = tool.unwrap_or("").to_lowercase();
    assert!(
        t.contains("news") || t.contains("search") || t.contains("web_search"),
        "News prompt must use a news/search tool, got: {}",
        t
    );
}

#[tokio::test]
async fn eval_install_must_execute() {
    eval_integration_guard!();
    let _guard = ServerGuard;

    let json = send_prompt_via_api("Install neofetch on my system")
        .await
        .expect("eval integration: failed to get response from API");

    let tool = json["tool_calls"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|t| t["name"].as_str());

    let response = json["response"]
        .as_str()
        .or_else(|| json["message"].as_str())
        .unwrap_or("");

    // Must NOT just give instructions
    assert!(
        !response.to_lowercase().contains("you can install")
            && !response.to_lowercase().contains("follow these steps"),
        "Install prompt must execute, not give instructions"
    );

    let t = tool.unwrap_or("").to_lowercase();
    assert!(
        t.contains("install") || t.contains("execute") || t.contains("bash"),
        "Install prompt must use install/execute tool, got: {}",
        t
    );
}

#[tokio::test]
async fn eval_file_ops_must_use_tool() {
    eval_integration_guard!();
    let _guard = ServerGuard;

    let json = send_prompt_via_api("List files in /tmp")
        .await
        .expect("eval integration: failed to get response from API");

    let tool = json["tool_calls"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|t| t["name"].as_str());

    let t = tool.unwrap_or("").to_lowercase();
    assert!(
        t.contains("list") || t.contains("file") || t.contains("files"),
        "File list prompt must use file tool, got: {}",
        t
    );
}

#[tokio::test]
async fn eval_no_hallucination_on_system_stats() {
    eval_integration_guard!();
    let _guard = ServerGuard;

    let json = send_prompt_via_api("What are my system stats?")
        .await
        .expect("eval integration: failed to get response from API");

    let response = json["response"]
        .as_str()
        .or_else(|| json["message"].as_str())
        .unwrap_or("");

    let issues = detect_response_issues(response, "What are my system stats?");

    // Filter out non-critical issues
    let critical: Vec<_> = issues
        .into_iter()
        .filter(|i| i.kind == IssueKind::Unavailable || i.kind == IssueKind::Hallucination)
        .collect();

    assert!(
        critical.is_empty(),
        "System stats response has critical issues: {:?}",
        critical
    );
}

#[tokio::test]
async fn eval_internet_check_uses_tool() {
    eval_integration_guard!();
    let _guard = ServerGuard;

    let json = send_prompt_via_api("Are you connected to the internet?")
        .await
        .expect("eval integration: failed to get response from API");

    let tool = json["tool_calls"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|t| t["name"].as_str());

    let t = tool.unwrap_or("").to_lowercase();
    assert!(
        t.contains("internet") || t.contains("ping") || t.contains("connect"),
        "Internet check must use connectivity tool, got: {}",
        t
    );
}
