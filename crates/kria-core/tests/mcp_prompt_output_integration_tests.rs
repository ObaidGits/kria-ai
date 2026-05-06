mod common;

use common::{tool_call_response, text_response, MockLlmServer};
use kria_core::agent::loop_engine::StreamEvent;
use kria_core::agent::AgentLoop;
use kria_core::config::McpServerConfig;
use kria_core::llm::{ChatMessage, ModelRouter};
use kria_core::mcp::{McpClient, McpServerManager};
use kria_core::safety::hitl::HitlGateway;
use kria_core::safety::{AuditLogger, PolicyEngine, RollbackManager};
use kria_core::tools::mount_manager::ToolMountManager;
use kria_core::tools::registry::ToolRegistry;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::sync::{mpsc, RwLock};

const FAKE_MCP_SERVER_SCRIPT: &str = r#"
import json
import os
import sys
import time

TOOLS_PAGE_1 = [
    {
        "name": "echo_tool",
        "description": "Echo a text payload",
        "inputSchema": {
            "type": "object",
            "properties": {
                "text": {"type": "string", "description": "Text to echo"}
            },
            "required": ["text"]
        }
    },
    {
        "name": "unstable_tool",
        "description": "Fails when account is injected",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Search query"}
            },
            "required": ["query"]
        }
    }
]

TOOLS_PAGE_2 = [
    {
        "name": "error_tool",
        "description": "Returns an MCP-level tool error",
        "inputSchema": {
            "type": "object",
            "properties": {
                "input": {"type": "string", "description": "Freeform input"}
            }
        }
    },
    {
        "name": "slow_tool",
        "description": "Sleeps before replying",
        "inputSchema": {
            "type": "object",
            "properties": {
                "sleep_secs": {"type": "number", "description": "Sleep duration"}
            }
        }
    },
    {
        "name": "exit_tool",
        "description": "Exits process before replying",
        "inputSchema": {
            "type": "object",
            "properties": {}
        }
    }
]


def send_response(req_id, result=None, error=None):
    payload = {"jsonrpc": "2.0", "id": req_id}
    if error is not None:
        payload["error"] = error
    else:
        payload["result"] = result
    sys.stdout.write(json.dumps(payload) + "\n")
    sys.stdout.flush()


for raw in sys.stdin:
    raw = raw.strip()
    if not raw:
        continue

    try:
        req = json.loads(raw)
    except Exception:
        continue

    method = req.get("method")
    req_id = req.get("id")
    params = req.get("params") or {}

    if method == "initialize":
        send_response(req_id, {
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}, "resources": None, "prompts": None},
            "serverInfo": {"name": "fake-mcp", "version": "1.0"}
        })
        continue

    if method == "notifications/initialized":
        continue

    if method == "tools/list":
        cursor = params.get("cursor")
        if cursor is None:
            send_response(req_id, {"tools": TOOLS_PAGE_1, "nextCursor": "page-2"})
        elif cursor == "page-2":
            send_response(req_id, {"tools": TOOLS_PAGE_2})
        else:
            send_response(req_id, {"tools": []})
        continue

    if method == "ping":
        if os.getenv("KRIA_FAKE_MCP_PING_MODE") == "hang":
            time.sleep(10)
            continue
        send_response(req_id, {"ok": True})
        continue

    if method == "tools/call":
        tool_name = params.get("name")
        args = params.get("arguments") or {}

        if tool_name == "echo_tool":
            text = str(args.get("text", ""))
            send_response(req_id, {
                "content": [{"type": "text", "text": f"echo:{text}"}],
                "isError": False
            })
            continue

        if tool_name == "unstable_tool":
            if isinstance(args, dict) and "account" in args:
                send_response(req_id, {
                    "content": [{
                        "type": "text",
                        "text": "Unexpected parameter 'account': additional properties not allowed"
                    }],
                    "isError": True
                })
            else:
                send_response(req_id, {
                    "content": [{"type": "text", "text": "retry-path-ok"}],
                    "isError": False
                })
            continue

        if tool_name == "error_tool":
            send_response(req_id, {
                "content": [{"type": "text", "text": "server-side failure"}],
                "isError": True
            })
            continue

        if tool_name == "slow_tool":
            sleep_secs = float(args.get("sleep_secs", 2.5))
            time.sleep(sleep_secs)
            send_response(req_id, {
                "content": [{"type": "text", "text": "slow-tool-complete"}],
                "isError": False
            })
            continue

        if tool_name == "exit_tool":
            sys.exit(0)

        send_response(req_id, {
            "content": [{"type": "text", "text": "unknown tool"}],
            "isError": True
        })
        continue

    send_response(req_id, error={"code": -32601, "message": "Method not found"})
"#;

struct FakeMcpHarness {
    _temp_dir: TempDir,
    python_cmd: String,
    script_path: PathBuf,
}

impl FakeMcpHarness {
    fn new() -> Option<Self> {
        let python_cmd = detect_python_command()?;
        let temp_dir = tempfile::tempdir().ok()?;
        let script_path = temp_dir.path().join("fake_mcp_server.py");

        std::fs::write(&script_path, FAKE_MCP_SERVER_SCRIPT).ok()?;

        Some(Self {
            _temp_dir: temp_dir,
            python_cmd,
            script_path,
        })
    }

    fn server_config(&self, server_name: &str, env: HashMap<String, String>) -> McpServerConfig {
        McpServerConfig {
            name: server_name.to_string(),
            command: self.python_cmd.clone(),
            args: vec![
                "-u".to_string(),
                self.script_path.to_string_lossy().to_string(),
            ],
            env,
            enabled: true,
            trust_level: "GREEN".to_string(),
            tool_overrides: HashMap::new(),
        }
    }

    fn start_args(&self) -> Vec<String> {
        vec![
            "-u".to_string(),
            self.script_path.to_string_lossy().to_string(),
        ]
    }
}

fn detect_python_command() -> Option<String> {
    for candidate in ["python3", "python"] {
        let ok = Command::new(candidate)
            .arg("--version")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false);
        if ok {
            return Some(candidate.to_string());
        }
    }
    None
}

fn build_test_agent_loop(api_url: String, tool_registry: Arc<ToolRegistry>) -> AgentLoop {
    let mut config = kria_core::config::KriaConfig::default();
    config.llm.local_api_url = api_url;
    config.llm.active_model = "mock-model".to_string();
    config.llm.routing_mode = "local".to_string();

    let model_router = Arc::new(ModelRouter::from_config(&config));
    let policy_engine = Arc::new(PolicyEngine::new());
    let hitl = Arc::new(HitlGateway::new(0));
    let audit_conn = rusqlite::Connection::open_in_memory().expect("open in-memory audit db");
    let audit_logger = Arc::new(AuditLogger::new(audit_conn));
    let rollback_dir = std::env::temp_dir().join(format!(
        "kria-mcp-prompt-output-rollback-{}",
        uuid::Uuid::new_v4()
    ));
    let rollback_mgr = Arc::new(RollbackManager::new(rollback_dir, 1, 10));
    let mount_mgr = Arc::new(RwLock::new(ToolMountManager::new()));

    AgentLoop::new(
        model_router,
        tool_registry,
        mount_mgr,
        policy_engine,
        hitl,
        audit_logger,
        rollback_mgr,
    )
    .with_hardware_tier("high")
    .with_max_tool_rounds(3)
}

async fn collect_events(mut rx: mpsc::UnboundedReceiver<StreamEvent>) -> Vec<StreamEvent> {
    let mut events = Vec::new();
    while let Some(ev) = rx.recv().await {
        events.push(ev);
    }
    events
}

fn extract_tool_end(events: &[StreamEvent], tool_name: &str) -> Option<(serde_json::Value, bool)> {
    events.iter().find_map(|ev| match ev {
        StreamEvent::ToolEnd {
            name,
            result,
            success,
        } if name == tool_name => Some((result.clone(), *success)),
        _ => None,
    })
}

async fn run_prompt_flow(
    tool_registry: Arc<ToolRegistry>,
    responses: Vec<serde_json::Value>,
    user_prompt: &str,
    session_id: &str,
) -> Vec<StreamEvent> {
    let mock_server = MockLlmServer::new(responses);
    let agent_loop = build_test_agent_loop(mock_server.base_url.clone(), tool_registry);

    let (tx, rx) = mpsc::unbounded_channel();
    let mut messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: "MCP integration prompt-output test".to_string(),
            name: None,
            images: None,
        },
        ChatMessage {
            role: "user".to_string(),
            content: user_prompt.to_string(),
            name: None,
            images: None,
        },
    ];

    agent_loop.run(session_id, &mut messages, tx).await;
    collect_events(rx).await
}

#[tokio::test]
async fn mcp_prompt_output_discovers_tools_and_invokes_from_prompt() {
    let Some(fake_mcp) = FakeMcpHarness::new() else {
        eprintln!("SKIP: python runtime not available for fake MCP server");
        return;
    };

    let registry = Arc::new(ToolRegistry::new());
    let mut manager = McpServerManager::new(vec![fake_mcp.server_config("gworkspace", HashMap::new())]);
    manager.start_all(&registry).await;

    assert!(
        registry.get_def("mcp_gworkspace_echo_tool").is_some(),
        "expected discovered MCP tool mcp_gworkspace_echo_tool"
    );
    assert!(
        registry.get_def("mcp_gworkspace_unstable_tool").is_some(),
        "expected paginated discovery of mcp_gworkspace_unstable_tool"
    );
    assert!(
        registry.get_def("mcp_gworkspace_error_tool").is_some(),
        "expected paginated discovery of mcp_gworkspace_error_tool"
    );

    let events = run_prompt_flow(
        Arc::clone(&registry),
        vec![
            tool_call_response(
                "mcp_gworkspace_echo_tool",
                serde_json::json!({ "text": "hello from prompt" }),
            ),
            text_response("done"),
        ],
        "#tool:mcp_gworkspace_echo_tool for gmail workflow, run the MCP echo tool",
        "mcp-prompt-output-discovery",
    )
    .await;

    let Some((result, success)) = extract_tool_end(&events, "mcp_gworkspace_echo_tool") else {
        panic!("missing ToolEnd for mcp_gworkspace_echo_tool");
    };

    assert!(success, "echo MCP call should succeed");
    let echoed = result.as_str().unwrap_or_default();
    assert!(
        echoed.contains("echo:hello from prompt"),
        "unexpected echoed payload: {echoed}"
    );
    assert!(
        events
            .iter()
            .any(|ev| matches!(ev, StreamEvent::Done(_) | StreamEvent::Error(_))),
        "expected terminal Done or Error event"
    );

    manager.stop_all().await;
}

#[tokio::test]
async fn mcp_prompt_output_retries_without_injected_account() {
    let Some(fake_mcp) = FakeMcpHarness::new() else {
        eprintln!("SKIP: python runtime not available for fake MCP server");
        return;
    };

    let registry = Arc::new(ToolRegistry::new());
    let mut manager = McpServerManager::new(vec![fake_mcp.server_config("gworkspace", HashMap::new())]);
    manager.start_all(&registry).await;

    assert!(
        registry.get_def("mcp_gworkspace_unstable_tool").is_some(),
        "expected discovered MCP tool mcp_gworkspace_unstable_tool"
    );

    let events = run_prompt_flow(
        Arc::clone(&registry),
        vec![
            tool_call_response(
                "mcp_gworkspace_unstable_tool",
                serde_json::json!({ "query": "unread mail" }),
            ),
            text_response("done"),
        ],
        "#tool:mcp_gworkspace_unstable_tool for gmail workflow, run the unstable MCP tool",
        "mcp-prompt-output-retry",
    )
    .await;

    let Some((result, success)) = extract_tool_end(&events, "mcp_gworkspace_unstable_tool") else {
        panic!("missing ToolEnd for mcp_gworkspace_unstable_tool");
    };

    assert!(
        success,
        "unstable tool should succeed after retry without injected account"
    );
    let payload = result.as_str().unwrap_or_default();
    assert!(
        payload.contains("retry-path-ok"),
        "retry result did not surface expected payload: {payload}"
    );

    manager.stop_all().await;
}

#[tokio::test]
async fn mcp_timeout_contract_ping_returns_false_when_server_hangs() {
    let Some(fake_mcp) = FakeMcpHarness::new() else {
        eprintln!("SKIP: python runtime not available for fake MCP server");
        return;
    };

    let mut env = HashMap::new();
    env.insert("KRIA_FAKE_MCP_PING_MODE".to_string(), "hang".to_string());

    let client = McpClient::new("gworkspace");
    let start_args = fake_mcp.start_args();

    client
        .start(&fake_mcp.python_cmd, &start_args, &env)
        .await
        .expect("fake MCP client should start");

    let started = Instant::now();
    let alive = client.ping().await;
    let elapsed = started.elapsed();

    assert!(!alive, "ping should report false when server does not reply");
    assert!(
        elapsed >= Duration::from_secs(5),
        "ping timeout should wait at least 5s, elapsed={elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(9),
        "ping timeout should not exceed reasonable bound, elapsed={elapsed:?}"
    );

    client.stop().await.expect("client stop should succeed");
}

#[tokio::test]
async fn mcp_prompt_output_surfaces_tool_errors_cleanly() {
    let Some(fake_mcp) = FakeMcpHarness::new() else {
        eprintln!("SKIP: python runtime not available for fake MCP server");
        return;
    };

    let registry = Arc::new(ToolRegistry::new());
    let mut manager = McpServerManager::new(vec![fake_mcp.server_config("gworkspace", HashMap::new())]);
    manager.start_all(&registry).await;

    assert!(
        registry.get_def("mcp_gworkspace_error_tool").is_some(),
        "expected discovered MCP tool mcp_gworkspace_error_tool"
    );

    let events = run_prompt_flow(
        Arc::clone(&registry),
        vec![
            tool_call_response(
                "mcp_gworkspace_error_tool",
                serde_json::json!({ "input": "trigger" }),
            ),
            text_response("done"),
        ],
        "#tool:mcp_gworkspace_error_tool for gmail workflow, run the MCP error tool",
        "mcp-prompt-output-error-surface",
    )
    .await;

    let Some((result, success)) = extract_tool_end(&events, "mcp_gworkspace_error_tool") else {
        panic!("missing ToolEnd for mcp_gworkspace_error_tool");
    };

    assert!(!success, "error_tool should fail and surface MCP error");
    let err = if let Some(s) = result.as_str() {
        s.to_string()
    } else {
        result["error"].as_str().unwrap_or_default().to_string()
    };
    assert!(
        err.contains("server-side failure"),
        "ToolEnd error should include MCP message, got: {err}"
    );
    assert!(
        events
            .iter()
            .any(|ev| matches!(ev, StreamEvent::Done(_) | StreamEvent::Error(_))),
        "expected terminal Done or Error event after error surface"
    );

    manager.stop_all().await;
}
