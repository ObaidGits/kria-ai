mod common;

use kria_core::agent::loop_engine::StreamEvent;
use kria_core::agent::AgentLoop;
use kria_core::infra::ToolResult;
use kria_core::llm::{ChatMessage, ModelRouter};
use kria_core::safety::hitl::HitlGateway;
use kria_core::safety::{AuditLogger, PolicyEngine, RiskLevel, RollbackManager};
use kria_core::tools::mount_manager::ToolMountManager;
use kria_core::tools::registry::{self, ToolDef, ToolRegistry};
use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::sync::RwLock;
use tokio::time::timeout;

fn build_test_agent_loop(
    api_url: String,
    tool_registry: Arc<ToolRegistry>,
    mount_mgr: Arc<RwLock<ToolMountManager>>,
) -> AgentLoop {
    let mut config = kria_core::config::KriaConfig::default();
    config.llm.local_api_url = api_url;
    config.llm.active_model = "mock-model".into();
    config.llm.routing_mode = "local".into();

    let model_router = Arc::new(ModelRouter::from_config(&config));
    let policy_engine = Arc::new(PolicyEngine::new());
    let hitl = Arc::new(HitlGateway::new(0));
    let audit_conn = rusqlite::Connection::open_in_memory().expect("open in-memory audit db");
    let audit_logger = Arc::new(AuditLogger::new(audit_conn));
    let rollback_dir =
        std::env::temp_dir().join(format!("kria-test-rollback-{}", uuid::Uuid::new_v4()));
    let rollback_mgr = Arc::new(RollbackManager::new(rollback_dir, 1, 10));

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
    .with_max_tool_rounds(2)
}

async fn collect_events(mut rx: mpsc::UnboundedReceiver<StreamEvent>) -> Vec<StreamEvent> {
    let mut events = Vec::new();
    while let Some(ev) = rx.recv().await {
        events.push(ev);
    }
    events
}

fn invalid_type_value(expected_type: &str) -> serde_json::Value {
    match expected_type {
        "string" => serde_json::json!({ "invalid": true }),
        "number" | "integer" => serde_json::json!("not_a_number"),
        "boolean" => serde_json::json!("not_a_boolean"),
        "array" => serde_json::json!("not_an_array"),
        "object" => serde_json::json!("not_an_object"),
        _ => serde_json::json!(null),
    }
}

fn invalid_required_args(def: &ToolDef) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for param in def.parameters.iter().filter(|p| p.required) {
        map.insert(param.name.clone(), invalid_type_value(&param.param_type));
    }
    serde_json::Value::Object(map)
}

fn base_messages(user_text: String) -> Vec<ChatMessage> {
    vec![
        ChatMessage {
            role: "system".into(),
            content: "Test system prompt".into(),
            name: None,
            images: None,
        },
        ChatMessage {
            role: "user".into(),
            content: user_text,
            name: None,
            images: None,
        },
    ]
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

fn spawn_mock_chat_server(
    responses: Vec<serde_json::Value>,
) -> (String, std::thread::JoinHandle<()>) {
    fn read_one_http_request(stream: &mut std::net::TcpStream) -> bool {
        let mut buf = Vec::<u8>::new();
        let mut tmp = [0u8; 2048];

        loop {
            match stream.read(&mut tmp) {
                Ok(0) => return false,
                Ok(n) => {
                    buf.extend_from_slice(&tmp[..n]);
                    if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                Err(_) => return false,
            }
        }

        let header_end = match buf.windows(4).position(|w| w == b"\r\n\r\n") {
            Some(i) => i + 4,
            None => return false,
        };

        let header_text = String::from_utf8_lossy(&buf[..header_end]);
        let mut content_len = 0usize;
        for line in header_text.lines() {
            let lower = line.to_ascii_lowercase();
            if let Some(v) = lower.strip_prefix("content-length:") {
                content_len = v.trim().parse::<usize>().unwrap_or(0);
                break;
            }
        }

        let needed = header_end + content_len;
        while buf.len() < needed {
            match stream.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&tmp[..n]),
                Err(_) => break,
            }
        }

        true
    }

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock chat server");
    let addr = listener.local_addr().expect("local addr");
    let handle = std::thread::spawn(move || {
        for body in responses {
            let (mut stream, _) = listener.accept().expect("accept connection");
            if !read_one_http_request(&mut stream) {
                continue;
            }

            let payload = body.to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                payload.len(),
                payload
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });

    (format!("http://{}/v1", addr), handle)
}

fn loop_tool_call_response(tool_name: &str, arguments: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {
                        "name": tool_name,
                        "arguments": arguments.to_string()
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 1,
            "completion_tokens": 1,
            "total_tokens": 2
        }
    })
}

fn text_response(text: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": text
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 1,
            "completion_tokens": 1,
            "total_tokens": 2
        }
    })
}

#[test]
fn registry_every_tool_has_handler_and_valid_schema() {
    let reg = registry::build_default_registry();
    let defs = reg.list_defs();

    assert!(
        defs.len() >= 50,
        "expected broad tool inventory, found {}",
        defs.len()
    );

    let mut unique_names = BTreeSet::new();

    for def in defs {
        assert!(
            unique_names.insert(def.name.clone()),
            "duplicate tool name in registry: {}",
            def.name
        );
        assert!(
            reg.get_handler(&def.name).is_some(),
            "missing handler for registered tool {}",
            def.name
        );

        let schema = def.to_function_schema();
        assert_eq!(
            schema["type"].as_str(),
            Some("function"),
            "schema type must be function for {}",
            def.name
        );
        assert_eq!(
            schema["function"]["name"].as_str(),
            Some(def.name.as_str()),
            "schema function.name mismatch for {}",
            def.name
        );
        assert!(
            schema["function"]["parameters"]["properties"].is_object(),
            "schema properties must be object for {}",
            def.name
        );

        let required_from_def: BTreeSet<String> = def
            .parameters
            .iter()
            .filter(|p| p.required)
            .map(|p| p.name.clone())
            .collect();
        let required_from_schema: BTreeSet<String> = schema["function"]["parameters"]["required"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default();

        assert_eq!(
            required_from_def, required_from_schema,
            "required param mismatch for {}",
            def.name
        );
    }
}

#[tokio::test]
async fn pipeline_every_registered_tool_reaches_tool_end_and_terminal_events() {
    let reg = Arc::new(registry::build_default_registry());
    let mut defs = reg.list_defs();
    defs.sort_by(|a, b| a.name.cmp(&b.name));

    let mut failures = Vec::new();

    for def in defs {
        let mut mount = ToolMountManager::new();
        mount.define_group("hidden_target", vec![def.name.clone()], false);
        let mount_mgr = Arc::new(RwLock::new(mount));

        let (api_url, _server_handle) = spawn_mock_chat_server(vec![
            loop_tool_call_response(&def.name, invalid_required_args(&def)),
            text_response("done"),
        ]);

        let agent_loop = build_test_agent_loop(api_url, reg.clone(), mount_mgr.clone());
        let (tx, rx) = mpsc::unbounded_channel();
        let mut messages = base_messages(format!("#tool:{} please execute", def.name));

        let session_id = format!("pipeline-{}", def.name);
        let run_result = timeout(
            Duration::from_secs(8),
            agent_loop.run(&session_id, &mut messages, tx),
        )
        .await;

        if run_result.is_err() {
            failures.push(format!("{}: agent loop run timed out", def.name));
            continue;
        }

        let events = collect_events(rx).await;

        let Some((result, success)) = extract_tool_end(&events, &def.name) else {
            failures.push(format!("{}: missing ToolEnd event", def.name));
            continue;
        };

        if success {
            failures.push(format!(
                "{}: expected ToolEnd success=false because tool is intentionally unmounted",
                def.name
            ));
            continue;
        }

        let err_msg = result
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if err_msg.is_empty() {
            failures.push(format!("{}: ToolEnd error message is empty", def.name));
            continue;
        }

        let has_terminal = events
            .iter()
            .any(|ev| matches!(ev, StreamEvent::Done(_) | StreamEvent::Error(_)));
        if !has_terminal {
            failures.push(format!("{}: missing terminal Done/Error event", def.name));
        }
    }

    assert!(
        failures.is_empty(),
        "{} tool pipeline failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[tokio::test]
async fn green_tools_with_required_params_return_structured_results_on_invalid_types() {
    let reg = registry::build_default_registry();
    let mut defs = reg.list_defs();
    defs.sort_by(|a, b| a.name.cmp(&b.name));

    let mut probed = 0usize;
    let mut failures = Vec::new();

    for def in defs {
        if def.default_tier != RiskLevel::Green {
            continue;
        }

        let has_required = def.parameters.iter().any(|p| p.required);
        if !has_required {
            continue;
        }

        let Some(handler) = reg.get_handler(&def.name) else {
            failures.push(format!("{}: missing handler", def.name));
            continue;
        };

        let tool_name = def.name.clone();
        let args = invalid_required_args(&def);
        let task = tokio::spawn(async move { handler.execute(args).await });

        match timeout(Duration::from_secs(6), task).await {
            Err(_) => failures.push(format!("{}: handler timed out", tool_name)),
            Ok(Err(join_err)) => {
                failures.push(format!("{}: handler panicked: {}", tool_name, join_err))
            }
            Ok(Ok(result)) => {
                if !is_structured_tool_result(&result) {
                    failures.push(format!(
                        "{}: returned malformed ToolResult envelope",
                        tool_name
                    ));
                }
            }
        }

        probed += 1;
    }

    assert!(
        probed >= 10,
        "expected to probe a broad set of GREEN tools, but only probed {}",
        probed
    );

    assert!(
        failures.is_empty(),
        "{} green tool contract failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn is_structured_tool_result(result: &ToolResult) -> bool {
    if result.success {
        result.error.is_none()
    } else {
        result
            .error
            .as_ref()
            .is_some_and(|msg| !msg.trim().is_empty())
    }
}
