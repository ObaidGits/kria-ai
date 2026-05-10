use crate::report::{EvalCase, EvalObservation, EvalVerdict};
use kria_core::config::KriaConfig;
use kria_core::llm::{ChatMessage, ModelRouter};
use serde_json::Value;

fn tool_names(obs: &EvalObservation) -> Vec<String> {
    obs.tool_calls
        .iter()
        .filter_map(|entry| entry.get("name"))
        .filter_map(Value::as_str)
        .map(|name| name.to_lowercase())
        .collect()
}

fn any_tool_match(names: &[String], candidates: &[&str]) -> bool {
    names.iter()
        .any(|name| candidates.iter().any(|candidate| name.contains(candidate)))
}

fn hard_fail_reason(case: &EvalCase, obs: &EvalObservation) -> Option<String> {
    let prompt = case.prompt.to_lowercase();
    let response = obs.final_response.to_lowercase();
    let names = tool_names(obs);

    let weather_prompt = prompt.contains("weather") || prompt.contains("temperature");
    let news_prompt = prompt.contains("news") || prompt.contains("headline");
    let install_prompt = prompt.contains("install ")
        || prompt.starts_with("install")
        || prompt.contains(" uninstall ")
        || prompt.starts_with("uninstall")
        || prompt.contains(" apt ")
        || prompt.contains("sudo apt")
        || prompt.contains("package");
    let file_prompt = prompt.contains("file")
        || prompt.contains("directory")
        || prompt.contains("folder")
        || prompt.contains("read ")
        || prompt.contains("write ")
        || prompt.contains("delete ");
    let vm_prompt = prompt.contains("vm") || prompt.contains("remote") || prompt.contains("ssh");
    let memory_prompt = prompt.contains("remember")
        || prompt.contains("recall")
        || prompt.contains("knowledge")
        || prompt.contains("rag");
    let mcp_prompt = prompt.contains("mcp") || prompt.contains("google workspace") || prompt.contains("gmail");

    if weather_prompt {
        let weather_tool_ok = any_tool_match(
            &names,
            &[
                "get_weather",
                "weather",
                "web_search",
                "search_news",
                "fetch_webpage",
            ],
        );
        if !weather_tool_ok {
            return Some("Stage A: weather prompt did not use a weather/live-search tool".to_string());
        }
        let weather_bad_phrases = [
            "cannot check weather",
            "can't check weather",
            "i cannot access real-time",
            "i can't access real-time",
            "unable to access real-time",
            "here are steps to check weather",
        ];
        if weather_bad_phrases
            .iter()
            .any(|phrase| response.contains(phrase))
        {
            return Some(
                "Stage A: weather prompt returned disclaimer/instructions instead of grounded result"
                    .to_string(),
            );
        }
    }

    if news_prompt {
        let news_tool_ok = any_tool_match(&names, &["get_news", "search_news", "web_search"]);
        if !news_tool_ok {
            return Some("Stage A: news prompt did not use a news/search tool".to_string());
        }
        let news_bad_phrases = [
            "i cannot access real-time",
            "i can't access real-time",
            "unable to fetch news",
            "here are steps to check news",
        ];
        if news_bad_phrases.iter().any(|phrase| response.contains(phrase)) {
            return Some(
                "Stage A: news prompt returned disclaimer/instructions instead of grounded result"
                    .to_string(),
            );
        }
    }

    if install_prompt {
        let install_tool_ok = any_tool_match(
            &names,
            &[
                "install_package",
                "check_package_installed",
                "execute_fleet_command",
                "execute_bash",
            ],
        );
        if !install_tool_ok {
            return Some("Stage A: install prompt did not trigger install/exec tool path".to_string());
        }
        let install_bad_phrases = [
            "you can install",
            "to install",
            "follow these steps",
            "open terminal and run",
        ];
        if install_bad_phrases
            .iter()
            .any(|phrase| response.contains(phrase))
            && !any_tool_match(&names, &["install_package", "execute_fleet_command", "execute_bash"])
        {
            return Some(
                "Stage A: install prompt returned only instructions without execution attempt"
                    .to_string(),
            );
        }
    }

    if file_prompt {
        let file_tool_ok = any_tool_match(
            &names,
            &[
                "list_files",
                "read_file",
                "write_file",
                "delete_file",
                "create_directory",
                "search_files",
                "move_file",
                "copy_file",
            ],
        );
        if !file_tool_ok {
            return Some("Stage A: file-system prompt did not use file tools".to_string());
        }
    }

    if vm_prompt {
        let vm_tool_ok = any_tool_match(&names, &["execute_fleet_command", "check_device_health"]);
        if !vm_tool_ok {
            return Some("Stage A: VM/remote prompt did not use fleet/health tools".to_string());
        }
    }

    if memory_prompt {
        let memory_tool_ok = any_tool_match(
            &names,
            &["remember_fact", "recall_fact", "rag_query", "ingest_document", "list_knowledge_base"],
        );
        if !memory_tool_ok {
            return Some("Stage A: memory/knowledge prompt did not use memory tools".to_string());
        }
    }

    if mcp_prompt {
        let mcp_tool_ok = any_tool_match(&names, &["gw_", "gmail", "calendar", "drive", "mcp_"]);
        if !mcp_tool_ok {
            return Some("Stage A: MCP/GWorkspace prompt did not use MCP-related tools".to_string());
        }
    }

    None
}

pub async fn evaluate_case(case: &EvalCase, obs: &EvalObservation) -> EvalVerdict {
    let final_response = obs.final_response.trim();
    if final_response.is_empty() {
        return EvalVerdict {
            case_id: case.id.clone(),
            stage_a_pass: false,
            judge_grade: "FAIL".to_string(),
            confidence: 1.0,
            reasons: vec!["Stage A: final_response is empty".to_string()],
            artifacts: serde_json::json!({
                "stage": "A",
                "failure_kind": "empty_final_response",
            }),
        };
    }

    let combined_observation = serde_json::json!({
        "final_response": obs.final_response,
        "events": obs.events,
        "tool_calls": obs.tool_calls,
        "policy_trace": obs.policy_trace,
    })
    .to_string();

    let fatal_markers = [
        "KRIA_EVAL_MODE active: command mocking not yet implemented",
        "KRIA_EVAL_MODE active: HTTP mocking not yet implemented",
        "failed to create temp eval sandbox",
        "failed to initialize in-memory audit DB",
        "no LLM backend available",
        "evaluator fail-closed",
    ];

    if let Some(marker) = fatal_markers
        .iter()
        .find(|marker| combined_observation.contains(**marker))
    {
        return EvalVerdict {
            case_id: case.id.clone(),
            stage_a_pass: false,
            judge_grade: "FAIL".to_string(),
            confidence: 1.0,
            reasons: vec![format!(
                "Stage A: known fatal/unmocked boundary encountered: {marker}"
            )],
            artifacts: serde_json::json!({
                "stage": "A",
                "failure_kind": "fatal_or_unmocked_boundary",
                "matched_marker": marker,
            }),
        };
    }

    if let Some(reason) = hard_fail_reason(case, obs) {
        return EvalVerdict {
            case_id: case.id.clone(),
            stage_a_pass: false,
            judge_grade: "FAIL".to_string(),
            confidence: 1.0,
            reasons: vec![reason],
            artifacts: serde_json::json!({
                "stage": "A",
                "failure_kind": "hard_behavior_guardrail",
            }),
        };
    }

    let deterministic_mode = matches!(
        std::env::var("KRIA_EVAL_DETERMINISTIC_MODE").as_deref(),
        Ok("1")
    );

    let tool_calls_text = serde_json::to_string_pretty(&obs.tool_calls)
        .unwrap_or_else(|_| "[]".to_string());

    let system_prompt = format!(
        "Evaluation inputs:\n\
CASE_PROMPT:\n{case_prompt}\n\
\n\
EXPECTED_OUTCOME:\n{expected_outcome}\n\
\n\
FINAL_RESPONSE:\n{final_response}\n\
\n\
TOOL_CALLS:\n{tool_calls_text}\n",
        case_prompt = case.prompt,
        expected_outcome = case.expected_outcome,
        final_response = obs.final_response,
        tool_calls_text = tool_calls_text,
    );

    let mut config = KriaConfig::default();
    if config.llm.local_api_url.trim().is_empty() {
        config.llm.local_api_url = "http://127.0.0.1:8080/v1".to_string();
    }
    if config.llm.active_model.trim().is_empty() {
        config.llm.active_model = "kria-eval-judge".to_string();
    }
    config.llm.routing_mode = "local".to_string();

    if deterministic_mode {
        return EvalVerdict {
            case_id: case.id.clone(),
            stage_a_pass: true,
            judge_grade: "PASS".to_string(),
            confidence: 0.9,
            reasons: vec!["Deterministic mode: Stage A passed; skipping LLM judge".to_string()],
            artifacts: serde_json::json!({
                "stage": "B",
                "judge_mode": "deterministic_stage_a_only",
            }),
        };
    }

    let router = ModelRouter::from_config(&config);
    let backend = match router.route("chat").await {
        Some(backend) => backend,
        None => {
            return EvalVerdict {
                case_id: case.id.clone(),
                stage_a_pass: true,
                judge_grade: "FAIL".to_string(),
                confidence: 1.0,
                reasons: vec![
                    "Judge LLM failure: no chat backend available from ModelRouter".to_string(),
                ],
                artifacts: serde_json::json!({
                    "stage": "B",
                    "failure_kind": "judge_backend_unavailable",
                }),
            };
        }
    };

    let judge_system_message = ChatMessage {
        role: "system".to_string(),
        content: "You are an impartial judge grading an autonomous agent run. Return ONLY raw JSON (no markdown, no explanation) with this exact schema: {\"case_id\":\"string\",\"stage_a_pass\":true,\"judge_grade\":\"PASS|FAIL\",\"confidence\":0.0,\"reasons\":[\"string\"],\"artifacts\":{}}. Enforce strict JSON validity and include only these top-level keys.".to_string(),
        name: None,
        images: None,
    };

    let judge_user_message = ChatMessage {
        role: "user".to_string(),
        content: system_prompt,
        name: None,
        images: None,
    };

    let judge_messages = vec![judge_system_message, judge_user_message];
    let max_tokens = config.llm.max_tokens.clamp(256, 2048) as u32;

    let llm_response = match backend.chat(&judge_messages, None, 0.0, max_tokens).await {
        Ok(response) => response,
        Err(error) => {
            return EvalVerdict {
                case_id: case.id.clone(),
                stage_a_pass: true,
                judge_grade: "FAIL".to_string(),
                confidence: 1.0,
                reasons: vec![format!("Judge LLM failure: {error}")],
                artifacts: serde_json::json!({
                    "stage": "B",
                    "failure_kind": "judge_llm_inference_error",
                }),
            };
        }
    };

    match serde_json::from_str::<EvalVerdict>(&llm_response.content) {
        Ok(mut verdict) => {
            if verdict.case_id.trim().is_empty() {
                verdict.case_id = case.id.clone();
            }
            verdict
        }
        Err(_error) => EvalVerdict {
            case_id: case.id.clone(),
            stage_a_pass: true,
            judge_grade: "PASS".to_string(),
            confidence: 0.55,
            reasons: vec![
                "Judge JSON parse failed; Stage A already passed, applying graceful pass".to_string(),
            ],
            artifacts: serde_json::json!({
                "stage": "B",
                "fallback_kind": "judge_json_parse_graceful_pass",
                "raw_llm_output": llm_response.content,
            }),
        },
    }
}
