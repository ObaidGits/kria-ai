use crate::judge::evaluate_case;
use crate::report::{EvalCase, EvalObservation, EvalVerdict};
use kria_core::agent::{AgentLoop, StreamEvent};
use kria_core::config::KriaConfig;
use kria_core::infra::environment::{DockerEnvironment, EnvironmentLifecycle};
use kria_core::llm::{ChatMessage, ModelRouter};
use kria_core::safety::{AuditLogger, HitlGateway, PolicyEngine, RollbackManager};
use kria_core::tools::registry::build_default_registry;
use kria_core::tools::ToolMountManager;
use std::sync::{Arc, LazyLock};
use std::time::Instant;
use tokio::sync::{mpsc, Mutex, RwLock};

static EVAL_ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

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
    let _eval_execution_guard = EnvVarGuard::set("KRIA_EVAL_EXECUTION_ENV", &requested_execution_env);

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
        if let Some(last_assistant) = messages.iter().rev().find(|message| message.role == "assistant")
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
        StreamEvent::Done(text) => serde_json::json!({
            "type": "done",
            "value": text,
        }),
    }
}
