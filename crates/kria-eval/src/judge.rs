use crate::report::{EvalCase, EvalObservation, EvalVerdict};
use kria_core::config::KriaConfig;
use kria_core::llm::{ChatMessage, ModelRouter};

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
        Err(error) => EvalVerdict {
            case_id: case.id.clone(),
            stage_a_pass: true,
            judge_grade: "FAIL".to_string(),
            confidence: 1.0,
            reasons: vec![format!("Judge JSON parse error: {error}")],
            artifacts: serde_json::json!({
                "stage": "B",
                "failure_kind": "judge_json_parse_error",
                "raw_llm_output": llm_response.content,
            }),
        },
    }
}
