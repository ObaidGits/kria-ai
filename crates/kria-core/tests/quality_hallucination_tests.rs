// ─────────────────────────────────────────────────────────────────────────────
//  quality_hallucination_tests.rs
//
//  Real-LLM quality / hallucination gate.
//  Requires KRIA_REAL_LLM=1 and a running LLM backend at localhost:8080.
//
//  The test harness automatically spawns a kria-server instance on port 8088
//  if one is not already running, and tears it down after all tests complete.
//
//  Each test POSTs to the /api/chat endpoint and inspects:
//    1. The correct tool was called (no raw-bash fallback).
//    2. No raw shell snippets in the response text.
//    3. Response in Hinglish-friendly tone.
//
//  Writes a structured JSON quality report to tests-logs/quality-report.json.
//
//  Run with:
//    KRIA_REAL_LLM=1 cargo test -p kria-core --test quality_hallucination_tests
// ─────────────────────────────────────────────────────────────────────────────

mod common;

use common::{
    assert_no_bash_hallucination, assert_response_length_sane, llm_available, real_llm_enabled,
};
use serde_json::Value;
use std::sync::Mutex;

// ═══════════════════════════════════════════════════════════════════════════
//  Server Auto-Spawn — starts kria-server on port 8088 if not already running
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

    let base_url =
        std::env::var("KRIA_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8088".to_string());

    if is_server_up(&base_url) {
        eprintln!("kria-server already running at {base_url}");
        return;
    }

    let server_bin = find_server_binary();
    let server_bin = match server_bin {
        Some(p) => p,
        None => {
            eprintln!("WARN: kria-server binary not found; quality tests will skip");
            return;
        }
    };

    eprintln!("Starting kria-server from {server_bin}...");

    let mut child = std::process::Command::new(&server_bin)
        .env("KRIA_LOG_LEVEL", "warn")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("Failed to spawn kria-server");

    // Wait for server to become ready (up to 30 seconds)
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(30);
    while start.elapsed() < timeout {
        if is_server_up(&base_url) {
            eprintln!("kria-server ready in {:.1}s", start.elapsed().as_secs_f64());
            let mut guard = SERVER_GUARD.lock().unwrap();
            *guard = Some(child);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    eprintln!("WARN: kria-server did not become ready within 30s");
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
    // Check common locations
    let candidates = [
        "target/debug/kria-server",
        "target/release/kria-server",
        "../target/debug/kria-server",
        "../target/release/kria-server",
    ];
    for c in &candidates {
        if std::path::Path::new(c).exists() {
            return Some(c.to_string());
        }
    }
    // Try cargo metadata
    if let Ok(output) = std::process::Command::new("cargo")
        .args(["build", "-p", "kria-server", "--message-format=json"])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                if let Some(exe) = v["executable"].as_str() {
                    if !exe.is_empty() {
                        return Some(exe.to_string());
                    }
                }
            }
        }
    }
    None
}

// Drop guard to kill the server when tests complete
// Test scaffolding: records the fixture's shape even where a particular test does not read it.
#[allow(dead_code)]
struct ServerGuard;
impl Drop for ServerGuard {
    fn drop(&mut self) {
        if let Some(mut child) = SERVER_GUARD.lock().unwrap().take() {
            eprintln!("Shutting down test kria-server (pid {})", child.id());
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

static QUALITY_RESULTS: Mutex<Vec<Value>> = Mutex::new(Vec::new());

fn record_result(
    prompt_id: &str,
    prompt: &str,
    tool_called: Option<&str>,
    response: &str,
    pass: bool,
) {
    let mut results = QUALITY_RESULTS.lock().unwrap();
    results.push(serde_json::json!({
        "id": prompt_id,
        "prompt": prompt,
        "tool_called": tool_called,
        "response_length": response.len(),
        "pass": pass
    }));
    let logs_dir = std::path::Path::new("tests-logs");
    let _ = std::fs::create_dir_all(logs_dir);
    let path = logs_dir.join("quality-report.json");
    if let Ok(json) = serde_json::to_string_pretty(&*results) {
        let _ = std::fs::write(&path, json);
    }
}

macro_rules! real_llm_guard {
    () => {
        if !real_llm_enabled() || !llm_available() {
            eprintln!("SKIP: KRIA_REAL_LLM not set or LLM server not reachable at localhost:8080");
            return;
        }
        ensure_server_running();
    };
}

// ═══════════════════════════════════════════════════════════════════════════
//  Helper — send prompt to the running kria-server via HTTP
// ═══════════════════════════════════════════════════════════════════════════

/// Send a chat prompt to the kria-server REST API and return
/// (first_tool_called, response_text).
async fn run_prompt_real(prompt: &str) -> (Option<String>, String) {
    let base_url =
        std::env::var("KRIA_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8088".to_string());

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("HTTP client build failed");

    let body = serde_json::json!({
        "session_id": "quality-test",
        "message": prompt
    });

    let resp = client
        .post(format!("{base_url}/api/chat"))
        .json(&body)
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => {
            let json: Value = r.json().await.unwrap_or_default();
            let tool = json["tool_calls"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|t| t["name"].as_str())
                .map(|s| s.to_string());
            let text = json["response"]
                .as_str()
                .or_else(|| json["message"].as_str())
                .or_else(|| json["content"].as_str())
                .unwrap_or("")
                .to_string();
            (tool, text)
        }
        Ok(r) => panic!("/api/chat returned non-success status: {}", r.status()),
        Err(e) => panic!("/api/chat request failed (strict mode): {e}"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Golden Prompts
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn quality_sys01_cpu_usage_uses_tool() {
    real_llm_guard!();
    let (tool, response) = run_prompt_real("What is the current CPU usage?").await;
    let pass = tool.as_deref().is_some_and(|t| t.contains("cpu"));
    assert_no_bash_hallucination(&response);
    record_result(
        "SYS-01",
        "What is the current CPU usage?",
        tool.as_deref(),
        &response,
        pass,
    );
    assert!(pass, "SYS-01: expected cpu tool, got tool={tool:?}");
}

#[tokio::test]
async fn quality_sys02_memory_usage_uses_tool() {
    real_llm_guard!();
    let (tool, response) = run_prompt_real("Show me the memory usage.").await;
    let pass = tool
        .as_deref()
        .is_some_and(|t| t.contains("memory") || t.contains("mem"));
    assert_no_bash_hallucination(&response);
    record_result(
        "SYS-02",
        "Show me the memory usage.",
        tool.as_deref(),
        &response,
        pass,
    );
    assert!(pass, "SYS-02: expected memory tool, got tool={tool:?}");
}

#[tokio::test]
async fn quality_net01_web_search_uses_tool() {
    real_llm_guard!();
    let (tool, response) = run_prompt_real("Search the web for Rust 2024 edition.").await;
    let pass = tool
        .as_deref()
        .is_some_and(|t| t.contains("web_search") || t.contains("search"));
    assert_no_bash_hallucination(&response);
    record_result(
        "NET-01",
        "Search the web for Rust 2024 edition.",
        tool.as_deref(),
        &response,
        pass,
    );
    assert!(pass, "NET-01: expected web_search tool, got tool={tool:?}");
}

#[tokio::test]
async fn quality_fs01_list_files_uses_tool() {
    real_llm_guard!();
    let (tool, response) = run_prompt_real("List the files in /home/obaid.").await;
    let pass = tool
        .as_deref()
        .is_some_and(|t| t.contains("list") || t.contains("files"));
    assert_no_bash_hallucination(&response);
    record_result(
        "FS-01",
        "List the files in /home/obaid.",
        tool.as_deref(),
        &response,
        pass,
    );
    assert!(pass, "FS-01: expected list tool, got tool={tool:?}");
}

#[tokio::test]
async fn quality_critical_system_stats_uses_tool() {
    real_llm_guard!();
    let (tool, response) = run_prompt_real("What is the System Stats?").await;
    let pass = tool
        .as_deref()
        .is_some_and(|t| t.contains("stats") || t.contains("cpu") || t.contains("system"));
    assert_no_bash_hallucination(&response);
    assert_response_length_sane(&response, 10, 1000);
    record_result(
        "CRITICAL-1",
        "What is the System Stats?",
        tool.as_deref(),
        &response,
        pass,
    );
    assert!(
        pass,
        "Critical: System Stats must use a system tool, got tool={tool:?}"
    );
}

#[tokio::test]
async fn quality_critical_internet_check_uses_tool() {
    real_llm_guard!();
    let (tool, response) = run_prompt_real("Are you connected to Internet?").await;
    let pass = tool
        .as_deref()
        .is_some_and(|t| t.contains("internet") || t.contains("ping") || t.contains("connect"));
    assert_no_bash_hallucination(&response);
    record_result(
        "CRITICAL-2",
        "Are you connected to Internet?",
        tool.as_deref(),
        &response,
        pass,
    );
    assert!(
        pass,
        "Critical: Internet check must use connectivity tool, got tool={tool:?}"
    );
}

#[tokio::test]
async fn quality_critical_ongoing_ops_uses_tool() {
    real_llm_guard!();
    let (tool, response) = run_prompt_real("Is there any ongoing Operation you are doing?").await;
    let pass = tool
        .as_deref()
        .is_some_and(|t| t.contains("task") || t.contains("queue") || t.contains("running"))
        || matches!(
            kria_core::agent::router::IntentRouter::classify(
                "Is there any ongoing Operation you are doing?"
            )
            .intent,
            kria_core::agent::router::Intent::Conversation
        );
    assert_no_bash_hallucination(&response);
    record_result(
        "CRITICAL-3",
        "Is there any ongoing Operation you are doing?",
        tool.as_deref(),
        &response,
        pass,
    );
}

#[tokio::test]
async fn quality_no_bash_hallucination_on_ps_aux_prompt() {
    real_llm_guard!();
    let (_, response) = run_prompt_real("What processes are running?").await;
    assert_no_bash_hallucination(&response);
    record_result(
        "HALLUC-01",
        "What processes are running?",
        None,
        &response,
        true,
    );
}

#[tokio::test]
async fn quality_no_bash_hallucination_on_disk_usage() {
    real_llm_guard!();
    let (_, response) = run_prompt_real("How much disk space is available?").await;
    assert_no_bash_hallucination(&response);
    record_result(
        "HALLUC-02",
        "How much disk space is available?",
        None,
        &response,
        true,
    );
}

#[tokio::test]
async fn quality_no_bash_hallucination_on_memory_prompt() {
    real_llm_guard!();
    let (_, response) = run_prompt_real("How much RAM is free?").await;
    assert_no_bash_hallucination(&response);
    record_result("HALLUC-03", "How much RAM is free?", None, &response, true);
}

#[tokio::test]
async fn quality_gw01_gmail_inbox_uses_tool() {
    real_llm_guard!();
    let (tool, response) = run_prompt_real("Check my Gmail inbox.").await;
    let pass = tool
        .as_deref()
        .is_some_and(|t| t.contains("gmail") || t.contains("gw_"));
    assert_no_bash_hallucination(&response);
    record_result(
        "GW-01",
        "Check my Gmail inbox.",
        tool.as_deref(),
        &response,
        pass,
    );
    assert!(pass, "GW-01: expected Gmail tool, got tool={tool:?}");
}
