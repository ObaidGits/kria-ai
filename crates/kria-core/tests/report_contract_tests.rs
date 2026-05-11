use serde_json::Value;
use std::path::PathBuf;

fn logs_root() -> PathBuf {
    PathBuf::from("tests-logs")
}

fn require_reports_enabled() -> bool {
    matches!(std::env::var("KRIA_REQUIRE_REPORTS").as_deref(), Ok("1"))
}

#[test]
#[ignore = "requires KRIA_REQUIRE_REPORTS=1 and cognitive-score.json"]
fn cognitive_score_report_schema_is_valid() {
    let path = logs_root().join("cognitive-score.json");
    if !path.exists() {
        if require_reports_enabled() {
            panic!("missing required report: {}", path.display());
        }
        eprintln!("SKIP: {} not found", path.display());
        return;
    }

    let raw = std::fs::read_to_string(&path).expect("read cognitive-score report");
    let json: Value = serde_json::from_str(&raw).expect("parse cognitive-score report");

    assert!(json.get("zone").is_some(), "missing zone");
    assert!(json.get("total_prompts").is_some(), "missing total_prompts");
    assert!(json.get("passed").is_some(), "missing passed");
    assert!(json.get("failed").is_some(), "missing failed");
    assert!(
        json.get("cognitive_score").is_some(),
        "missing cognitive_score"
    );
}

#[test]
#[ignore = "requires KRIA_REQUIRE_REPORTS=1 and quality-report.json"]
fn quality_report_schema_is_valid() {
    let path = logs_root().join("quality-report.json");
    if !path.exists() {
        if require_reports_enabled() {
            panic!("missing required report: {}", path.display());
        }
        eprintln!("SKIP: {} not found", path.display());
        return;
    }

    let raw = std::fs::read_to_string(&path).expect("read quality report");
    let json: Value = serde_json::from_str(&raw).expect("parse quality report");
    assert!(json.is_array(), "quality report must be an array");
}

#[test]
#[ignore = "requires KRIA_REQUIRE_REPORTS=1 and KRIA_TREND_COGNITIVE_FLOOR"]
fn cognitive_trend_floor_guard() {
    let path = logs_root().join("cognitive-score.json");
    if !path.exists() {
        if require_reports_enabled() {
            panic!("missing required report: {}", path.display());
        }
        eprintln!("SKIP: {} not found", path.display());
        return;
    }
    let raw = std::fs::read_to_string(&path).expect("read cognitive-score report");
    let json: Value = serde_json::from_str(&raw).expect("parse cognitive-score report");
    let score_text = json
        .get("cognitive_score")
        .and_then(Value::as_str)
        .unwrap_or("0%");
    let score = score_text
        .trim_end_matches('%')
        .parse::<f64>()
        .unwrap_or(0.0);
    let floor = std::env::var("KRIA_TREND_COGNITIVE_FLOOR")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(60.0);
    assert!(
        score >= floor,
        "cognitive trend below floor: {:.1}% < {:.1}%",
        score,
        floor
    );
}
