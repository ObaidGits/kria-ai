use serde::Serialize;

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct EvalCase {
    pub id: String,
    pub prompt: String,
    pub expected_outcome: String,
    pub tags: Vec<String>,
    pub fixtures_ref: String,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct EvalObservation {
    pub case_id: String,
    pub events: Vec<serde_json::Value>,
    pub tool_calls: Vec<serde_json::Value>,
    pub policy_trace: Vec<serde_json::Value>,
    pub final_response: String,
    pub timings: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct EvalVerdict {
    pub case_id: String,
    pub stage_a_pass: bool,
    pub judge_grade: String,
    pub confidence: f64,
    pub reasons: Vec<String>,
    pub artifacts: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct EvalCaseResult {
    pub observation: EvalObservation,
    pub verdict: EvalVerdict,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct EvalRunReport {
    pub run_id: String,
    pub summary: serde_json::Value,
    pub case_results: Vec<EvalVerdict>,
    pub environment: serde_json::Value,
}
