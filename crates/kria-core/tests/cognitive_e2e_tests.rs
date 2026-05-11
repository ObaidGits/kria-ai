// ─────────────────────────────────────────────────────────────────────────────
//  cognitive_e2e_tests.rs
//
//  Zone 6: Cognitive E2E — Prompt-to-Tool routing validation.
//
//  Parses the structured prompt matrices from TestPrompts.txt and
//  VMTestPrompts.txt, runs each prompt through the IntentRouter, and
//  verifies the correct tool is selected.  This is the behavioral
//  intelligence gate that ensures the assistant routes user intent
//  correctly across 200+ real-world prompts.
//
//  Run standalone:
//    cargo test -p kria-core --test cognitive_e2e_tests
//
//  Run via kria-test runner (Zone 6):
//    cargo kria-test --mode FULL
// ─────────────────────────────────────────────────────────────────────────────

mod common;

use kria_core::agent::router::{Intent, IntentRouter};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

fn env_threshold(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(default)
}

// ═══════════════════════════════════════════════════════════════════════════
//  Cognitive Score Tracking
// ═══════════════════════════════════════════════════════════════════════════

static COGNITIVE_RESULTS: Mutex<Vec<CognitiveResult>> = Mutex::new(Vec::new());

struct CognitiveResult {
    prompt_id: String,
    prompt: String,
    expected_tool: String,
    actual_tool: Option<String>,
    pass: bool,
    source: String, // "TestPrompts" or "VMTestPrompts"
}

fn record_cognitive(result: CognitiveResult) {
    let mut results = COGNITIVE_RESULTS.lock().unwrap();
    results.push(result);
}

fn write_cognitive_score(report: &serde_json::Value) {
    let root = find_workspace_root();
    let logs_dir = root.join("tests-logs");
    if std::fs::create_dir_all(&logs_dir).is_err() {
        return;
    }
    let path = logs_dir.join("cognitive-score.json");
    if let Ok(json) = serde_json::to_string_pretty(report) {
        let _ = std::fs::write(path, json);
    }
}

fn flush_cognitive_report() {
    let results = COGNITIVE_RESULTS.lock().unwrap();
    let total = results.len();
    let passed = results.iter().filter(|r| r.pass).count();
    let failed = total - passed;
    let score = if total > 0 {
        (passed as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    let report = serde_json::json!({
        "zone": "cognitive_e2e",
        "total_prompts": total,
        "passed": passed,
        "failed": failed,
        "cognitive_score": format!("{:.1}%", score),
        "by_source": {
            "TestPrompts": {
                "total": results.iter().filter(|r| r.source == "TestPrompts").count(),
                "passed": results.iter().filter(|r| r.source == "TestPrompts" && r.pass).count(),
            },
            "VMTestPrompts": {
                "total": results.iter().filter(|r| r.source == "VMTestPrompts").count(),
                "passed": results.iter().filter(|r| r.source == "VMTestPrompts" && r.pass).count(),
            },
        },
        "failures": results.iter().filter(|r| !r.pass).map(|r| {
            serde_json::json!({
                "id": r.prompt_id,
                "prompt": r.prompt,
                "expected": r.expected_tool,
                "actual": r.actual_tool,
                "source": r.source,
            })
        }).collect::<Vec<_>>(),
    });

    write_cognitive_score(&report);

    eprintln!("\n═══════════════════════════════════════════════════");
    eprintln!("  COGNITIVE E2E SCORE: {score:.1}%  ({passed}/{total} passed)");
    eprintln!(
        "  TestPrompts:  {}/{}",
        results
            .iter()
            .filter(|r| r.source == "TestPrompts" && r.pass)
            .count(),
        results.iter().filter(|r| r.source == "TestPrompts").count()
    );
    eprintln!(
        "  VMTestPrompts: {}/{}",
        results
            .iter()
            .filter(|r| r.source == "VMTestPrompts" && r.pass)
            .count(),
        results
            .iter()
            .filter(|r| r.source == "VMTestPrompts")
            .count()
    );
    if failed > 0 {
        eprintln!("  FAILURES:");
        for r in results.iter().filter(|r| !r.pass) {
            eprintln!(
                "    [{}] '{}' → expected '{}', got {:?}",
                r.prompt_id, r.prompt, r.expected_tool, r.actual_tool
            );
        }
    }
    eprintln!("═══════════════════════════════════════════════════\n");
}

// ═══════════════════════════════════════════════════════════════════════════
//  Prompt Matrix Parser
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
struct PromptCase {
    id: String,
    prompt: String,
    expected_tool: String,
    source: String,
}

/// Parse a structured prompt matrix file into test cases.
///
/// Handles two formats:
///   Format 1 (TestPrompts.txt): Per-prompt Tool line
///     [ID]  Prompt : "the user prompt"
///           Tool   : tool_name
///
///   Format 2 (VMTestPrompts.txt): Section-level TOOL PATH
///     TOOL PATH: execute_fleet_command
///     [ID] Prompt : "the user prompt"
///
fn parse_prompt_matrix(path: &Path, source: &str) -> Vec<PromptCase> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut cases = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;
    let mut current_section_tool: Option<String> = None;

    while i < lines.len() {
        let line = lines[i].trim();

        // Track section-level TOOL PATH
        if line.starts_with("TOOL PATH:") {
            if let Some(colon_pos) = line.find(':') {
                let tool = line[colon_pos + 1..]
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_string();
                if !tool.is_empty() {
                    current_section_tool = Some(tool);
                }
            }
            i += 1;
            continue;
        }

        // Look for [ID] pattern
        if line.starts_with('[') && line.contains(']') {
            let id_end = line.find(']').unwrap();
            let id = line[1..id_end].trim().to_string();

            // Look for Prompt : on the same or next line
            let prompt_line = if line.contains("Prompt") && line.contains(':') {
                line.to_string()
            } else {
                let mut found = String::new();
                for j in (i + 1)..std::cmp::min(i + 4, lines.len()) {
                    let next = lines[j].trim();
                    if next.starts_with("Prompt") && next.contains(':') {
                        found = next.to_string();
                        break;
                    }
                    if next.starts_with('[') && next.contains(']') {
                        break;
                    }
                }
                found
            };

            if prompt_line.is_empty() {
                i += 1;
                continue;
            }

            let prompt = extract_quoted(&prompt_line).unwrap_or_default();
            if prompt.is_empty() {
                i += 1;
                continue;
            }

            // Look for per-prompt Tool : line first, fall back to section TOOL PATH
            let mut expected_tool = String::new();
            for j in (i + 1)..std::cmp::min(i + 6, lines.len()) {
                let next = lines[j].trim();
                if next.starts_with("Tool") && next.contains(':') {
                    if let Some(colon_pos) = next.find(':') {
                        let after_colon = next[colon_pos + 1..].trim();
                        let tool = after_colon
                            .split('+')
                            .next()
                            .unwrap_or("")
                            .split_whitespace()
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        if !tool.is_empty() {
                            expected_tool = tool;
                        }
                    }
                    break;
                }
                if next.starts_with('[') && next.contains(']') {
                    break;
                }
            }

            // Fall back to section-level TOOL PATH
            if expected_tool.is_empty() {
                if let Some(ref section_tool) = current_section_tool {
                    expected_tool = section_tool.clone();
                }
            }

            if !expected_tool.is_empty() && !prompt.is_empty() {
                cases.push(PromptCase {
                    id,
                    prompt,
                    expected_tool,
                    source: source.to_string(),
                });
            }
        }
        i += 1;
    }

    cases
}

fn extract_quoted(line: &str) -> Option<String> {
    let first = line.find('"')?;
    let rest = &line[first + 1..];
    let second = rest.find('"')?;
    Some(rest[..second].to_string())
}

// ═══════════════════════════════════════════════════════════════════════════
//  Tool Name Normalization
// ═══════════════════════════════════════════════════════════════════════════

/// Map expected tool names from the prompt matrix to the actual tool names
/// registered in the IntentRouter.  The prompt matrix uses human-readable
/// names; the router uses snake_case identifiers.
fn normalize_tool_name(raw: &str) -> Vec<String> {
    let raw = raw.trim().to_lowercase();
    let mut candidates = vec![raw.clone()];

    // Common aliases
    let aliases: HashMap<&str, &[&str]> = HashMap::from([
        (
            "get_cpu_usage",
            &["cpu_usage", "get_cpu", "system_stats"] as &[&str],
        ),
        ("get_memory_info", &["memory_info", "get_memory", "get_ram"]),
        ("get_disk_space", &["disk_space", "get_disk", "disk_usage"]),
        (
            "get_battery_status",
            &["battery_status", "get_battery", "battery"],
        ),
        (
            "get_system_uptime",
            &["system_uptime", "get_uptime", "uptime"],
        ),
        (
            "check_system_health",
            &["system_health", "health_check", "check_health"],
        ),
        ("get_alerts", &["alerts", "show_alerts", "list_alerts"]),
        ("dismiss_alert", &["alert_dismiss"]),
        ("get_gpu_info", &["gpu_info", "get_gpu", "nvidia_smi"]),
        (
            "get_network_status",
            &["network_status", "get_network", "network_info"],
        ),
        ("get_volume", &["volume"]),
        ("set_volume", &["change_volume"]),
        ("lock_screen", &["screen_lock"]),
        (
            "search_files",
            &["find_files", "file_search", "search_file"],
        ),
        ("browser_search", &["web_search", "search_web", "open_url"]),
        (
            "open_application",
            &["open_app", "launch_app", "launch_application"],
        ),
        ("list_files", &["ls", "list_dir"]),
        ("read_file", &["cat_file", "show_file"]),
        (
            "execute_fleet_command",
            &["fleet_command", "remote_command", "vm_command"],
        ),
        ("list_packages", &["package_list", "apt_list"]),
        ("install_package", &["apt_install", "package_install"]),
        ("send_whatsapp", &["whatsapp_send", "whatsapp"]),
        ("send_email", &["email_send", "gmail_send"]),
        ("get_weather", &["weather", "weather_forecast"]),
        ("play_music", &["music_play", "play_song"]),
        ("set_reminder", &["reminder", "create_reminder"]),
        (
            "get_screen_brightness",
            &["brightness", "screen_brightness"],
        ),
        ("set_screen_brightness", &["change_brightness"]),
        ("toggle_wifi", &["wifi_toggle", "wifi_on", "wifi_off"]),
        ("list_wifi_networks", &["wifi_list", "wifi_networks"]),
        ("close_application", &["close_app", "kill_app"]),
        ("list_running_apps", &["running_apps", "ps_list"]),
        ("kill_process", &["process_kill", "kill_pid"]),
        ("get_active_connections", &["active_connections", "netstat"]),
        ("set_environment_variable", &["set_env", "export_env"]),
        ("get_environment_variable", &["get_env", "echo_env"]),
        ("list_environment_variables", &["env_vars", "printenv"]),
        ("execute_bash", &["service_status", "systemctl"]),
        ("get_power_plan", &["power_plan"]),
        ("set_power_plan", &["change_power_plan"]),
        ("shutdown_system", &["shutdown", "power_off"]),
        ("reboot_system", &["reboot", "restart"]),
    ]);

    // Check if raw matches any alias key or value
    for (canonical, alts) in &aliases {
        if raw == *canonical || alts.contains(&raw.as_str()) {
            candidates.push(canonical.to_string());
            for alt in *alts {
                candidates.push(alt.to_string());
            }
        }
    }

    candidates.sort();
    candidates.dedup();
    candidates
}

// ═══════════════════════════════════════════════════════════════════════════
//  Test Harness
// ═══════════════════════════════════════════════════════════════════════════

fn find_workspace_root() -> PathBuf {
    let mut dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    loop {
        if dir.join("TestPrompts.txt").exists() {
            return dir;
        }
        if !dir.pop() {
            break;
        }
    }
    // Fallback
    for candidate in &["/media/obaid/SSD/KRIA", "../.."] {
        let p = PathBuf::from(candidate);
        if p.join("TestPrompts.txt").exists() {
            return p;
        }
    }
    PathBuf::from(".")
}

fn run_prompt_matrix(filename: &str, source: &str) {
    let root = find_workspace_root();
    let matrix_path = root.join(filename);
    if !matrix_path.exists() {
        eprintln!("SKIP: {filename} not found at {}", matrix_path.display());
        return;
    }

    let cases = parse_prompt_matrix(&matrix_path, source);
    if cases.is_empty() {
        eprintln!(
            "SKIP: no parseable prompt cases in {}",
            matrix_path.display()
        );
        return;
    }

    eprintln!(
        "Loaded {} prompt cases from {}",
        cases.len(),
        matrix_path.display()
    );

    let mut failure_count = 0usize;
    for case in &cases {
        let router_result = IntentRouter::classify(&case.prompt);
        let actual_tool = match &router_result.intent {
            Intent::DirectTool(t) => Some(t.clone()),
            Intent::ComplexTask => Some("complex_task".to_string()),
            Intent::Conversation => None,
        };

        let expected_candidates = normalize_tool_name(&case.expected_tool);
        let pass = actual_tool
            .as_ref()
            .map(|t| expected_candidates.iter().any(|c| c == t))
            .unwrap_or(false);

        if !pass {
            failure_count += 1;
            eprintln!(
                "  WARN [{}] '{}' → expected '{}', got {:?}",
                case.id, case.prompt, case.expected_tool, actual_tool
            );
        }

        record_cognitive(CognitiveResult {
            prompt_id: case.id.clone(),
            prompt: case.prompt.clone(),
            expected_tool: case.expected_tool.clone(),
            actual_tool: actual_tool.clone(),
            pass,
            source: source.to_string(),
        });
    }

    // Production gate with configurable thresholds.
    let total = cases.len();
    let passed = total - failure_count;
    let score = if total > 0 {
        (passed as f64 / total as f64) * 100.0
    } else {
        0.0
    };
    eprintln!("  {source} score: {score:.1}% ({passed}/{total})");

    let threshold = match source {
        "VMTestPrompts" => env_threshold("KRIA_COGNITIVE_MIN_VM", 95.0),
        _ => env_threshold("KRIA_COGNITIVE_MIN_MAIN", 70.0),
    };
    if score < threshold {
        panic!(
            "COGNITIVE {source}: below threshold ({score:.1}% < {threshold:.1}%) ({passed}/{total})"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Test Entry Points
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn cognitive_testprompts_matrix() {
    run_prompt_matrix("TestPrompts.txt", "TestPrompts");
    flush_cognitive_report();
}

#[test]
fn cognitive_vmtestprompts_matrix() {
    run_prompt_matrix("VMTestPrompts.txt", "VMTestPrompts");
    flush_cognitive_report();
}

// ── Aggregate score test (runs last, after both matrices) ──────────────

#[test]
fn cognitive_aggregate_score_report() {
    // Parse both files and compute aggregate score without asserting per-prompt.
    // This test always passes but emits the full cognitive score report.
    let root = find_workspace_root();
    let test_cases = parse_prompt_matrix(&root.join("TestPrompts.txt"), "TestPrompts");
    let vm_cases = parse_prompt_matrix(&root.join("VMTestPrompts.txt"), "VMTestPrompts");

    let all_cases: Vec<PromptCase> = test_cases.into_iter().chain(vm_cases).collect();
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut failures = Vec::new();

    for case in &all_cases {
        let router_result = IntentRouter::classify(&case.prompt);
        let actual_tool = match &router_result.intent {
            Intent::DirectTool(t) => Some(t.clone()),
            Intent::ComplexTask => Some("complex_task".to_string()),
            Intent::Conversation => None,
        };

        let expected_candidates = normalize_tool_name(&case.expected_tool);
        let pass = actual_tool
            .as_ref()
            .map(|t| expected_candidates.iter().any(|c| c == t))
            .unwrap_or(false);

        if pass {
            passed += 1;
        } else {
            failed += 1;
            failures.push(format!(
                "[{}] '{}' → expected '{}', got {:?}",
                case.id, case.prompt, case.expected_tool, actual_tool
            ));
        }
    }

    let total = all_cases.len();
    let score = if total > 0 {
        (passed as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    let report = serde_json::json!({
        "zone": "cognitive_e2e",
        "total_prompts": total,
        "passed": passed,
        "failed": failed,
        "cognitive_score": format!("{:.1}%", score),
        "failures": failures,
    });

    write_cognitive_score(&report);

    eprintln!("\n═══════════════════════════════════════════════════");
    eprintln!("  COGNITIVE E2E AGGREGATE SCORE: {score:.1}%  ({passed}/{total})");
    eprintln!("═══════════════════════════════════════════════════\n");

    let aggregate_threshold = env_threshold("KRIA_COGNITIVE_MIN_AGGREGATE", 78.0);
    assert!(
        score >= aggregate_threshold,
        "COGNITIVE aggregate below threshold: {:.1}% < {:.1}%",
        score,
        aggregate_threshold
    );
}
