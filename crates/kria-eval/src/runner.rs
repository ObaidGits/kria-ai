use crate::judge::evaluate_case;
use crate::report::{EvalCase, EvalObservation, EvalVerdict};
use kria_core::agent::{AgentLoop, StreamEvent};
use kria_core::config::KriaConfig;
use kria_core::infra::environment::{DockerEnvironment, EnvironmentLifecycle};
use kria_core::llm::{ChatMessage, ModelRouter};
use kria_core::safety::{AuditLogger, HitlGateway, PolicyEngine, RollbackManager};
use kria_core::tools::registry::build_default_registry;
use kria_core::tools::ToolMountManager;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::Instant;
use tokio::process::Child;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::time::{sleep, Duration};

static EVAL_ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Server lifecycle management for eval - ensures kria-server is running
/// before executing prompts, just like real usage would do.
static SERVER_GUARD: LazyLock<Mutex<Option<ServerHandle>>> = LazyLock::new(|| Mutex::new(None));

pub struct ServerHandle {
    pub port: u16,
    pub base_url: String,
    child: Option<Child>,
    we_spawned: bool,
}

impl ServerHandle {
    /// Check if server is healthy by hitting the /health endpoint
    async fn check_health(&self) -> bool {
        let url = format!("{}/health", self.base_url);
        if let Ok(output) = tokio::process::Command::new("curl")
            .args(["-s", "--max-time", "2", &url])
            .output()
            .await
        {
            output.status.success()
        } else {
            false
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        if self.we_spawned {
            if let Some(mut child) = self.child.take() {
                eprintln!("Shutting down eval kria-server (pid {:?})", child.id());
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

/// Find the kria-server binary in common locations or build it
fn find_server_binary() -> Option<PathBuf> {
    let candidates = [
        "target/debug/kria-server",
        "target/release/kria-server",
        "../target/debug/kria-server",
        "../target/release/kria-server",
        "/media/obaid/SSD/KRIA/target/debug/kria-server",
        "/media/obaid/SSD/KRIA/target/release/kria-server",
    ];

    for path in &candidates {
        if std::path::Path::new(path).exists() {
            return Some(PathBuf::from(path));
        }
    }

    None
}

/// Start kria-server on the configured port and wait for it to be healthy
async fn start_server(port: u16) -> Result<ServerHandle, String> {
    let server_bin =
        find_server_binary().ok_or_else(|| "kria-server binary not found".to_string())?;

    eprintln!("Starting kria-server on port {port}...");

    let child = tokio::process::Command::new(&server_bin)
        .env("KRIA_LOG_LEVEL", "warn")
        .env("RUST_BACKTRACE", "1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn kria-server: {e}"))?;

    let base_url = format!("http://127.0.0.1:{port}");
    let mut handle = ServerHandle {
        port,
        base_url: base_url.clone(),
        child: Some(child),
        we_spawned: true,
    };

    // Wait for server to become healthy (up to 60 seconds)
    let start = Instant::now();
    let timeout = Duration::from_secs(60);

    while start.elapsed() < timeout {
        if handle.check_health().await {
            eprintln!(
                "kria-server ready at {base_url} in {:.1}s",
                start.elapsed().as_secs_f64()
            );
            return Ok(handle);
        }
        sleep(Duration::from_millis(500)).await;
    }

    // Kill the failed spawn
    if let Some(mut child) = handle.child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }

    Err("kria-server did not become healthy within 60s".to_string())
}

/// Ensure a kria-server is running - either use existing or spawn new
pub async fn ensure_server_running() -> Result<ServerHandle, String> {
    let mut guard = SERVER_GUARD.lock().await;

    if let Some(ref handle) = *guard {
        if handle.check_health().await {
            eprintln!("Using existing kria-server at {}", handle.base_url);
            return Ok(handle.clone());
        }
    }

    // Parse port from env or default
    let port = std::env::var("KRIA_EVAL_SERVER_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(8088);

    let base_url = format!("http://127.0.0.1:{port}");

    // Check if something is already running on this port
    if let Ok(output) = tokio::process::Command::new("curl")
        .args(["-s", "--max-time", "2", &format!("{}/health", base_url)])
        .output()
        .await
    {
        if output.status.success() {
            eprintln!("kria-server already running at {base_url}");
            let handle = ServerHandle {
                port,
                base_url,
                child: None,
                we_spawned: false,
            };
            *guard = Some(handle.clone());
            return Ok(handle);
        }
    }

    // Spawn new server
    let handle = start_server(port).await?;
    *guard = Some(handle.clone());
    Ok(handle)
}

/// Send a prompt to the running kria-server via HTTP and return the response
pub async fn send_prompt_via_http(prompt: &str, session_id: &str) -> Result<String, String> {
    let handle = ensure_server_running().await?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| format!("HTTP client build failed: {e}"))?;

    let body = serde_json::json!({
        "session_id": session_id,
        "message": prompt
    });

    let resp = client
        .post(format!("{}/api/chat", handle.base_url))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Server returned error: {}", resp.status()));
    }

    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse JSON response: {e}"))?;

    // Extract response text from various possible JSON shapes
    let response_text = json["response"]
        .as_str()
        .or_else(|| json["message"].as_str())
        .or_else(|| json["content"].as_str())
        .unwrap_or("")
        .to_string();

    Ok(response_text)
}

/// Stop the managed kria-server if we spawned it
pub async fn shutdown_server() {
    let mut guard = SERVER_GUARD.lock().await;
    *guard = None;
    eprintln!("Eval server guard released");
}

impl Clone for ServerHandle {
    fn clone(&self) -> Self {
        Self {
            port: self.port,
            base_url: self.base_url.clone(),
            child: None,
            we_spawned: false,
        }
    }
}

/// Run an eval case via HTTP API - mimics real usage by calling running kria-server
pub async fn run_eval_case_via_api(case: EvalCase) -> (EvalObservation, EvalVerdict) {
    let _ = dotenvy::dotenv();
    let started_at = Instant::now();

    // Step 1: Ensure server is running
    let server_handle = match ensure_server_running().await {
        Ok(handle) => handle,
        Err(error) => {
            let obs = EvalObservation {
                case_id: case.id.clone(),
                events: vec![],
                tool_calls: vec![],
                policy_trace: vec![],
                final_response: format!("Failed to start kria-server: {}", error),
                timings: serde_json::json!({
                    "duration_ms": started_at.elapsed().as_millis(),
                    "execution_provider": "http_api",
                    "server_error": error,
                }),
            };
            let verdict = evaluate_case(&case, &obs).await;
            return (obs, verdict);
        }
    };

    // Step 2: Send prompt via HTTP
    let session_id = format!("eval-api-{}", case.id);
    let response_text = match send_prompt_via_http(&case.prompt, &session_id).await {
        Ok(text) => text,
        Err(error) => {
            let obs = EvalObservation {
                case_id: case.id.clone(),
                events: vec![],
                tool_calls: vec![],
                policy_trace: vec![],
                final_response: format!("HTTP API call failed: {}", error),
                timings: serde_json::json!({
                    "duration_ms": started_at.elapsed().as_millis(),
                    "execution_provider": "http_api",
                    "api_error": error,
                    "server_url": server_handle.base_url,
                }),
            };
            let verdict = evaluate_case(&case, &obs).await;
            return (obs, verdict);
        }
    };

    // Step 3: Detect response issues (hallucination, unavailable, wrong data)
    let issues = detect_response_issues(&response_text, &case.prompt);
    let has_critical_issue = !issues.is_empty();

    let observation = EvalObservation {
        case_id: case.id.clone(),
        events: vec![serde_json::json!({
            "type": "api_response_received",
            "server_url": server_handle.base_url,
            "response_length": response_text.len(),
            "issues_detected": issues,
        })],
        tool_calls: vec![],
        policy_trace: vec![],
        final_response: response_text.clone(),
        timings: serde_json::json!({
            "duration_ms": started_at.elapsed().as_millis(),
            "execution_provider": "http_api",
            "server_url": server_handle.base_url,
        }),
    };

    // Step 4: If critical issues detected, fail fast
    if has_critical_issue {
        let issue_reasons: Vec<String> = issues.iter().map(|i| i.description.clone()).collect();
        let verdict = EvalVerdict {
            case_id: case.id.clone(),
            stage_a_pass: false,
            judge_grade: "FAIL".to_string(),
            confidence: 1.0,
            reasons: issue_reasons,
            artifacts: serde_json::json!({
                "stage": "A",
                "failure_kind": "response_issue_detected",
                "issues": issues,
            }),
        };
        return (observation, verdict);
    }

    let verdict = evaluate_case(&case, &observation).await;
    (observation, verdict)
}

/// Response issue types for detection
#[derive(Debug, Clone, serde::Serialize)]
pub struct ResponseIssue {
    pub kind: ResponseIssueKind,
    pub description: String,
    pub matched_pattern: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum ResponseIssueKind {
    /// KRIA said it cannot do something or doesn't have access
    Unavailable,
    /// KRIA hallucinated or made up information
    Hallucination,
    /// KRIA returned wrong or fabricated data
    WrongData,
    /// KRIA returned a bash/terminal command instead of using tools
    BashFallback,
}

/// Detect issues in KRIA's response: unavailable claims, hallucination, wrong data
pub fn detect_response_issues(response: &str, prompt: &str) -> Vec<ResponseIssue> {
    let mut issues = Vec::new();
    let response_lower = response.to_lowercase();
    let prompt_lower = prompt.to_lowercase();

    // ─── Unavailable / Cannot Do patterns ───
    let unavailable_patterns = [
        (
            "i cannot access real-time",
            "Real-time data access claimed unavailable",
        ),
        (
            "i can't access real-time",
            "Real-time data access claimed unavailable",
        ),
        ("i do not have access to", "Tool access claimed unavailable"),
        ("i don't have access to", "Tool access claimed unavailable"),
        ("i cannot check", "Action claimed unavailable"),
        ("i can't check", "Action claimed unavailable"),
        ("i cannot access", "Access claimed unavailable"),
        ("i can't access", "Access claimed unavailable"),
        ("i am not able to access", "Access claimed unavailable"),
        (
            "unable to access real-time",
            "Real-time data access claimed unavailable",
        ),
        ("do not have permission", "Permission claimed denied"),
        ("don't have the ability", "Capability claimed missing"),
        ("don't have the capability", "Capability claimed missing"),
        ("i cannot help with that", "Request claimed unsupported"),
        ("i'm sorry, but i cannot", "Request claimed unsupported"),
        ("as an ai", "Disclaiming knowledge/capability"),
        ("please provide your", "Requesting user to do KRIA's job"),
        ("you can install", "Redirecting to manual steps"),
        ("to install", "Redirecting to manual steps"),
        ("follow these steps", "Redirecting to manual steps"),
        ("open terminal", "Redirecting to manual terminal use"),
    ];

    for (pattern, description) in &unavailable_patterns {
        if response_lower.contains(pattern) {
            // Only flag if the prompt is asking for something KRIA should be able to do
            let should_be_able = prompt_lower.contains("weather")
                || prompt_lower.contains("news")
                || prompt_lower.contains("install")
                || prompt_lower.contains("check")
                || prompt_lower.contains("search")
                || prompt_lower.contains("file")
                || prompt_lower.contains("memory")
                || prompt_lower.contains("remember");
            if should_be_able {
                issues.push(ResponseIssue {
                    kind: ResponseIssueKind::Unavailable,
                    description: description.to_string(),
                    matched_pattern: pattern.to_string(),
                });
            }
        }
    }

    // ─── Bash/Terminal fallback patterns ───
    let bash_patterns = [
        ("```bash", "Bash code block in response"),
        ("```sh", "Shell code block in response"),
        ("run: `", "Inline bash command"),
        ("command: `", "Inline bash command"),
        ("sudo apt", "Apt command in text"),
        ("pip install", "Pip install in text"),
        ("brew install", "Brew install in text"),
        ("npx ", "NPX command in text"),
    ];

    for (pattern, description) in &bash_patterns {
        if response.contains(pattern) {
            issues.push(ResponseIssue {
                kind: ResponseIssueKind::BashFallback,
                description: description.to_string(),
                matched_pattern: pattern.to_string(),
            });
        }
    }

    // ─── Hallucination markers ───
    let hallucination_patterns = [
        (
            "here are the steps to check",
            "Instructions instead of actual data",
        ),
        ("based on my training", "Dated knowledge disclaimer"),
        ("as of my last training", "Training data date claim"),
        ("according to my knowledge", "Knowledge disclaimer"),
        (
            "i don't have real-time",
            "Real-time data hallucination disclaimer",
        ),
        ("i may not have the most", "Data freshness disclaimer"),
    ];

    for (pattern, description) in &hallucination_patterns {
        if response_lower.contains(pattern) {
            issues.push(ResponseIssue {
                kind: ResponseIssueKind::Hallucination,
                description: description.to_string(),
                matched_pattern: pattern.to_string(),
            });
        }
    }

    // ─── Check for empty or too-short responses ───
    if response.trim().len() < 5 {
        issues.push(ResponseIssue {
            kind: ResponseIssueKind::Hallucination,
            description: "Response too short or empty".to_string(),
            matched_pattern: "(empty)".to_string(),
        });
    }

    // ─── Weather-specific hallucination check ───
    if prompt_lower.contains("weather") && response_lower.contains("here are") {
        issues.push(ResponseIssue {
            kind: ResponseIssueKind::Hallucination,
            description: "Weather response with instructions instead of actual data".to_string(),
            matched_pattern: "here are".to_string(),
        });
    }

    // ─── News-specific hallucination check ───
    if (prompt_lower.contains("news") || prompt_lower.contains("headline"))
        && response_lower.contains("here are")
        && !response_lower.contains("breaking:")
        && !response_lower.contains("reported:")
    {
        issues.push(ResponseIssue {
            kind: ResponseIssueKind::Hallucination,
            description: "News response with instructions instead of actual news".to_string(),
            matched_pattern: "here are".to_string(),
        });
    }

    issues
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

pub async fn run_eval_case(case: EvalCase) -> (EvalObservation, EvalVerdict) {
    let _ = dotenvy::dotenv();
    let _env_lock = EVAL_ENV_LOCK.lock().await;
    let started_at = Instant::now();

    let sandbox_root = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(error) => {
            let obs = EvalObservation {
                case_id: case.id.clone(),
                events: vec![],
                tool_calls: vec![],
                policy_trace: vec![],
                final_response: format!("failed to create temp eval sandbox: {error}"),
                timings: serde_json::json!({
                    "duration_ms": started_at.elapsed().as_millis(),
                }),
            };
            let verdict = evaluate_case(&case, &obs).await;
            return (obs, verdict);
        }
    };

    let sandbox_root_str = sandbox_root.path().to_string_lossy().to_string();
    let _eval_mode_guard = EnvVarGuard::set("KRIA_EVAL_MODE", "1");
    let _eval_fs_guard = EnvVarGuard::set("KRIA_EVAL_FS_ROOT", &sandbox_root_str);

    let requested_execution_env = std::env::var("KRIA_EVAL_EXECUTION_ENV")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "docker".to_string());
    let _eval_execution_guard =
        EnvVarGuard::set("KRIA_EVAL_EXECUTION_ENV", &requested_execution_env);

    if !requested_execution_env.eq_ignore_ascii_case("docker") {
        let obs = EvalObservation {
            case_id: case.id.clone(),
            events: vec![],
            tool_calls: vec![],
            policy_trace: vec![],
            final_response: format!(
                "evaluator fail-closed: unsupported KRIA_EVAL_EXECUTION_ENV='{}'; docker is required",
                requested_execution_env
            ),
            timings: serde_json::json!({
                "duration_ms": started_at.elapsed().as_millis(),
                "execution_provider": "docker",
            }),
        };
        let verdict = evaluate_case(&case, &obs).await;
        return (obs, verdict);
    }

    let mut config = match KriaConfig::load(None) {
        Ok(config) => config,
        Err(error) => {
            eprintln!(
                "Warning: failed to load runtime config for eval; falling back to defaults: {}",
                error
            );
            KriaConfig::default()
        }
    };

    if let Ok(value) = std::env::var("KRIA_EVAL_LLM_BASE_URL") {
        if !value.trim().is_empty() {
            config.llm.local_api_url = value;
        }
    }

    if let Ok(value) = std::env::var("KRIA_EVAL_ACTIVE_MODEL") {
        if !value.trim().is_empty() {
            config.llm.active_model = value;
        }
    }

    if config.llm.local_api_url.trim().is_empty() {
        config.llm.local_api_url = "http://127.0.0.1:8080/v1".to_string();
    }
    if config.llm.active_model.trim().is_empty() {
        config.llm.active_model = "phi-4-mini".to_string();
    }
    if config.llm.routing_mode.trim().is_empty() {
        config.llm.routing_mode = "local".to_string();
    }

    let model_router = Arc::new(ModelRouter::from_config(&config));
    let tool_registry = Arc::new(build_default_registry());

    let docker_environment = match DockerEnvironment::from_env() {
        Ok(env) => Arc::new(env),
        Err(error) => {
            let obs = EvalObservation {
                case_id: case.id.clone(),
                events: vec![],
                tool_calls: vec![],
                policy_trace: vec![],
                final_response: format!(
                    "evaluator fail-closed: unable to construct docker execution provider: {error}"
                ),
                timings: serde_json::json!({
                    "duration_ms": started_at.elapsed().as_millis(),
                    "execution_provider": "docker",
                }),
            };
            let verdict = evaluate_case(&case, &obs).await;
            return (obs, verdict);
        }
    };

    if let Err(error) = docker_environment.ensure_ready().await {
        let obs = EvalObservation {
            case_id: case.id.clone(),
            events: vec![],
            tool_calls: vec![],
            policy_trace: vec![],
            final_response: format!(
                "evaluator fail-closed: docker execution provider is not ready (daemon/network policy): {error}"
            ),
            timings: serde_json::json!({
                "duration_ms": started_at.elapsed().as_millis(),
                "execution_provider": "docker",
            }),
        };
        let verdict = evaluate_case(&case, &obs).await;
        return (obs, verdict);
    }

    let mut reset_events = docker_environment.subscribe_environment_resets();
    tool_registry.set_environment_provider(docker_environment);

    let mount_manager = Arc::new(RwLock::new(ToolMountManager::new()));
    let policy_engine = Arc::new(PolicyEngine::new());
    let hitl_gateway = Arc::new(HitlGateway::new(0));

    let audit_logger = match rusqlite::Connection::open_in_memory() {
        Ok(conn) => Arc::new(AuditLogger::new(conn)),
        Err(error) => {
            let obs = EvalObservation {
                case_id: case.id.clone(),
                events: vec![],
                tool_calls: vec![],
                policy_trace: vec![],
                final_response: format!("failed to initialize in-memory audit DB: {error}"),
                timings: serde_json::json!({
                    "duration_ms": started_at.elapsed().as_millis(),
                }),
            };
            let verdict = evaluate_case(&case, &obs).await;
            return (obs, verdict);
        }
    };

    let rollback_mgr = Arc::new(RollbackManager::new(
        sandbox_root.path().join("rollback"),
        1,
        64,
    ));

    let agent_loop = AgentLoop::new(
        model_router,
        tool_registry,
        mount_manager,
        policy_engine,
        hitl_gateway,
        audit_logger,
        rollback_mgr,
    )
    .with_max_tool_rounds(3);

    let session_id = format!("eval-{}", case.id);
    let mut messages = vec![ChatMessage {
        role: "user".to_string(),
        content: case.prompt.clone(),
        name: None,
        images: None,
    }];

    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    agent_loop.run(&session_id, &mut messages, event_tx).await;

    let mut events = Vec::new();
    let mut tool_calls = Vec::new();
    let mut policy_trace = Vec::new();
    let mut final_response = String::new();

    while let Some(event) = event_rx.recv().await {
        let event_value = stream_event_to_json(&event);

        match &event {
            StreamEvent::ToolStart { name, params } => {
                tool_calls.push(serde_json::json!({
                    "phase": "start",
                    "name": name,
                    "params": params,
                }));
            }
            StreamEvent::ToolEnd {
                name,
                result,
                success,
                ..
            } => {
                tool_calls.push(serde_json::json!({
                    "phase": "end",
                    "name": name,
                    "result": result,
                    "success": success,
                }));
            }
            StreamEvent::ApprovalRequired { .. } | StreamEvent::ApprovalResult { .. } => {
                policy_trace.push(event_value.clone());
            }
            StreamEvent::Done(text) => {
                final_response = text.clone();
            }
            _ => {}
        }

        events.push(event_value);
    }

    loop {
        match reset_events.try_recv() {
            Ok(reset_event) => {
                events.push(serde_json::json!({
                    "type": "environment_reset",
                    "kind": reset_event.kind.as_str(),
                    "reason": reset_event.reason,
                    "generation": reset_event.generation,
                }));
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
            | Err(tokio::sync::broadcast::error::TryRecvError::Closed) => break,
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {
                events.push(serde_json::json!({
                    "type": "environment_reset",
                    "kind": "lagged",
                    "reason": "environment reset notification stream lagged",
                }));
                break;
            }
        }
    }

    if final_response.is_empty() {
        if let Some(last_assistant) = messages
            .iter()
            .rev()
            .find(|message| message.role == "assistant")
        {
            final_response = last_assistant.content.clone();
        }
    }

    let event_count = events.len();

    let observation = EvalObservation {
        case_id: case.id.clone(),
        events,
        tool_calls,
        policy_trace,
        final_response,
        timings: serde_json::json!({
            "duration_ms": started_at.elapsed().as_millis(),
            "event_count": event_count,
            "sandbox_root": sandbox_root.path().to_string_lossy().to_string(),
            "execution_provider": "docker",
        }),
    };

    let verdict = evaluate_case(&case, &observation).await;
    (observation, verdict)
}

fn stream_event_to_json(event: &StreamEvent) -> serde_json::Value {
    match event {
        StreamEvent::TurnAccepted {
            session_id,
            turn_id,
        } => {
            serde_json::json!({
                "type": "turn_accepted",
                "session_id": session_id,
                "turn_id": turn_id,
            })
        }
        StreamEvent::Token(token) => serde_json::json!({
            "type": "token",
            "value": token,
        }),
        StreamEvent::ToolStart { name, params } => serde_json::json!({
            "type": "tool_start",
            "name": name,
            "params": params,
        }),
        StreamEvent::ToolEnd {
            name,
            result,
            success,
            ..
        } => serde_json::json!({
            "type": "tool_end",
            "name": name,
            "result": result,
            "success": success,
        }),
        StreamEvent::ToolProgress {
            call_id,
            message,
            percent,
        } => serde_json::json!({
            "type": "tool_progress",
            "call_id": call_id,
            "message": message,
            "percent": percent,
        }),
        StreamEvent::ToolPayloadChunk {
            call_id,
            seq,
            is_final,
            data,
        } => serde_json::json!({
            "type": "tool_payload_chunk",
            "call_id": call_id,
            "seq": seq,
            "is_final": is_final,
            "data": data,
        }),
        StreamEvent::ApprovalRequired {
            request_id,
            action,
            risk_level,
            parameters,
        } => serde_json::json!({
            "type": "approval_required",
            "request_id": request_id,
            "action": action,
            "risk_level": risk_level,
            "parameters": parameters,
        }),
        StreamEvent::ApprovalResult { action, approved } => serde_json::json!({
            "type": "approval_result",
            "action": action,
            "approved": approved,
        }),
        StreamEvent::ToolChoiceRequired {
            query,
            confidence,
            min_confidence,
            candidates,
        } => serde_json::json!({
            "type": "tool_choice_required",
            "query": query,
            "confidence": confidence,
            "min_confidence": min_confidence,
            "candidates": candidates.iter().map(|candidate| serde_json::json!({
                "name": candidate.name,
                "label": candidate.label,
                "reason": candidate.reason,
                "confidence": candidate.confidence,
            })).collect::<Vec<_>>(),
        }),
        StreamEvent::Plan(plan) => serde_json::json!({
            "type": "plan",
            "value": plan,
        }),
        StreamEvent::Error(error) => serde_json::json!({
            "type": "error",
            "value": error,
        }),
        StreamEvent::RecoveryOptions {
            context,
            detail,
            options,
        } => serde_json::json!({
            "type": "recovery_options",
            "context": context,
            "detail": detail,
            "options": options.iter().map(|o| serde_json::json!({
                "label": o.label,
                "action_prompt": o.action_prompt,
                "style": o.style,
            })).collect::<Vec<_>>(),
        }),
        StreamEvent::TaskStep(step) => serde_json::json!({
            "type": "task_step",
            "index": step.index,
            "total": step.total,
            "description": step.description,
            "status": step.status,
        }),
        StreamEvent::Done(text) => serde_json::json!({
            "type": "done",
            "value": text,
        }),
    }
}
