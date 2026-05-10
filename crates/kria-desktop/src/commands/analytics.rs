use super::*;
use std::collections::HashMap;
use std::path::PathBuf;

// ═══════════════════════════════════════════════════════════════════════════
//  Analytics Dashboard — Aggregates all KRIA data sources into one payload
// ═══════════════════════════════════════════════════════════════════════════

#[derive(serde::Serialize, Default)]
pub struct DashboardPayload {
    pub timestamp_unix_ms: u64,
    pub uptime_secs: u64,
    pub overview: OverviewStats,
    pub memory: MemoryStats,
    pub mcp_servers: Vec<McpServerView>,
    pub mcp_failure_history: Vec<McpFailureHistoryEntry>,
    pub config: ConfigSnapshot,
    pub test_reports: Vec<TestReportEntry>,
    pub cognitive_score: Option<CognitiveScoreView>,
    pub system_health: SystemHealthSnapshot,
    pub orchestrator: OrchestratorView,
    pub colab: ColabView,
    pub tool_registry: ToolRegistryView,
}

#[derive(serde::Serialize, Default)]
pub struct OverviewStats {
    pub total_sessions: u64,
    pub total_turns: u64,
    pub total_facts: u64,
    pub total_snippets: u64,
    pub total_documents: u64,
    pub total_tools: u64,
    pub mcp_servers_running: u64,
    pub mcp_servers_total: u64,
}

#[derive(serde::Serialize, Default)]
pub struct MemoryStats {
    pub sessions: Vec<SessionEntry>,
    pub recent_facts: Vec<FactEntry>,
    pub snippets: Vec<String>,
    pub documents: Vec<DocumentEntry>,
    pub facts_by_category: HashMap<String, u64>,
    pub facts_by_source: HashMap<String, u64>,
}

#[derive(serde::Serialize)]
pub struct SessionEntry {
    pub session_id: String,
    pub turn_count: i64,
    pub last_active: String,
}

#[derive(serde::Serialize)]
pub struct FactEntry {
    pub id: i64,
    pub text: String,
    pub category: String,
    pub source: String,
    pub decay_score: f64,
    pub access_count: i32,
}

#[derive(serde::Serialize)]
pub struct DocumentEntry {
    pub doc_name: String,
    pub doc_type: String,
    pub chunk_count: i64,
}

#[derive(serde::Serialize)]
pub struct McpServerView {
    pub name: String,
    pub command: String,
    pub enabled: bool,
    pub state: String,
    pub tool_count: usize,
    pub error: Option<String>,
    pub health: String,
    pub tags: Vec<String>,
    pub remediation: Option<String>,
    pub last_failure: Option<McpFailureRecordView>,
}

#[derive(serde::Serialize, Clone)]
pub struct McpFailureRecordView {
    pub timestamp_unix_ms: u64,
    pub state: String,
    pub reason: String,
}

#[derive(serde::Serialize)]
pub struct McpFailureHistoryEntry {
    pub server_name: String,
    pub failures: Vec<McpFailureRecordView>,
}

#[derive(serde::Serialize, Default)]
pub struct ConfigSnapshot {
    pub llm_routing_mode: String,
    pub llm_primary_model: String,
    pub voice_enabled: bool,
    pub voice_stt_model: String,
    pub voice_tts_voice: String,
    pub safety_max_concurrent_tools: usize,
    pub orchestrator_enabled: bool,
    pub colab_enabled: bool,
    pub telegram_enabled: bool,
    pub memory_max_facts: usize,
    pub memory_decay_threshold: f32,
    pub executive_enabled: bool,
    pub planner_enabled: bool,
    pub uncertainty_enabled: bool,
    pub skill_compiler_enabled: bool,
    pub curiosity_enabled: bool,
    pub browser_agent_enabled: bool,
    pub hardware_tier: String,
}

#[derive(serde::Serialize)]
pub struct TestReportEntry {
    pub filename: String,
    pub modified_unix_ms: u64,
    pub mode: String,
    pub passed: u64,
    pub failed: u64,
    pub skipped: u64,
}

#[derive(serde::Serialize, Default)]
pub struct CognitiveScoreView {
    pub zone: String,
    pub total_prompts: u64,
    pub passed: u64,
    pub failed: u64,
    pub score_pct: f64,
    pub top_failures: Vec<CognitiveFailureEntry>,
}

#[derive(serde::Serialize)]
pub struct CognitiveFailureEntry {
    pub prompt_id: String,
    pub expected: String,
    pub actual: String,
}

#[derive(serde::Serialize, Default)]
pub struct SystemHealthSnapshot {
    pub cpu_cores: usize,
    pub ram_total_mb: u64,
    pub vram_mb: Option<u64>,
    pub vram_free_mb: u64,
    pub gpu_name: String,
    pub hostname: String,
    pub uptime_secs: u64,
}

#[derive(serde::Serialize, Default)]
pub struct OrchestratorView {
    pub active: bool,
    pub backend: String,
    pub ngl: u32,
    pub context_window: u32,
    pub degradation: String,
    pub server_healthy: bool,
    pub active_turns: u32,
}

#[derive(serde::Serialize, Default)]
pub struct ColabView {
    pub state: String,
    pub server_name: String,
    pub selected_notebook: String,
    pub last_error: Option<String>,
}

#[derive(serde::Serialize, Default)]
pub struct ToolRegistryView {
    pub total_tools: u64,
    pub by_category: HashMap<String, u64>,
    pub by_risk_level: HashMap<String, u64>,
}

// ═══════════════════════════════════════════════════════════════════════════
//  Main Dashboard Command
// ═══════════════════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn get_analytics_dashboard(
    state: State<'_, AppStateCell>,
) -> Result<DashboardPayload, String> {
    let state = state
        .get()
        .ok_or_else(|| "Runtime not initialized".to_string())?;

    let mut payload = DashboardPayload {
        timestamp_unix_ms: now_unix_ms(),
        uptime_secs: state.started_at.elapsed().as_secs(),
        ..Default::default()
    };

    // ── Memory Store ───────────────────────────────────────────────────
    if let Ok(sessions) = state.memory_store.list_sessions() {
        payload.overview.total_sessions = sessions.len() as u64;
        payload.memory.sessions = sessions
            .iter()
            .map(|(sid, turns, ts)| SessionEntry {
                session_id: sid.clone(),
                turn_count: *turns,
                last_active: ts.clone(),
            })
            .collect();
        payload.overview.total_turns = sessions.iter().map(|(_, t, _)| *t as u64).sum();
    }

    if let Ok(facts) = state.memory_store.search_facts("", 500) {
        payload.overview.total_facts = facts.len() as u64;
        let mut by_cat: HashMap<String, u64> = HashMap::new();
        let mut by_src: HashMap<String, u64> = HashMap::new();
        for f in &facts {
            *by_cat.entry(f.category.clone()).or_default() += 1;
            *by_src.entry(f.source.clone()).or_default() += 1;
        }
        payload.memory.facts_by_category = by_cat;
        payload.memory.facts_by_source = by_src;
        payload.memory.recent_facts = facts
            .into_iter()
            .take(50)
            .map(|f| FactEntry {
                id: f.id.unwrap_or(0),
                text: f.text,
                category: f.category,
                source: f.source,
                decay_score: f.decay_score,
                access_count: f.access_count,
            })
            .collect();
    }

    if let Ok(snippets) = state.memory_store.list_snippets(None) {
        payload.overview.total_snippets = snippets.len() as u64;
        payload.memory.snippets = snippets;
    }

    if let Ok(docs) = state.memory_store.list_documents() {
        payload.overview.total_documents = docs.len() as u64;
        payload.memory.documents = docs
            .iter()
            .map(|(name, dtype, _, count)| DocumentEntry {
                doc_name: name.clone(),
                doc_type: dtype.clone(),
                chunk_count: *count,
            })
            .collect();
    }

    // ── MCP Servers ───────────────────────────────────────────────────
    {
        let manager = state.mcp_manager.lock().await;
        let statuses = manager.status().await;
        let configs = { state.config.read().await.mcp.servers.clone() };
        let failure_history = { state.mcp_failure_history.read().await.clone() };

        payload.overview.mcp_servers_total = configs.len() as u64;

        for cfg in &configs {
            let runtime = statuses.iter().find(|s| s.name == cfg.name);
            let hist = failure_history.get(&cfg.name).cloned().unwrap_or_default();
            let running = runtime
                .map(|r| r.state == McpServerState::Running)
                .unwrap_or(false);
            if running {
                payload.overview.mcp_servers_running += 1;
            }

            let health = if !cfg.enabled {
                "disabled"
            } else if runtime
                .map(|r| r.state == McpServerState::Running && r.tool_count > 0)
                .unwrap_or(false)
            {
                "healthy"
            } else if runtime
                .map(|r| r.state == McpServerState::Running)
                .unwrap_or(false)
            {
                "degraded"
            } else {
                "error"
            };

            let error = runtime.and_then(|r| r.error.clone());
            let last = hist.last().cloned();
            payload.mcp_servers.push(McpServerView {
                name: cfg.name.clone(),
                command: cfg.command.clone(),
                enabled: cfg.enabled,
                state: runtime
                    .map(|r| mcp_state_name(r.state).to_string())
                    .unwrap_or_else(|| "stopped".to_string()),
                tool_count: runtime.map(|r| r.tool_count).unwrap_or(0),
                error: error.clone(),
                health: health.to_string(),
                tags: infer_mcp_tags_static(&cfg.name, &cfg.command, &cfg.args),
                remediation: compute_remediation(cfg, error.as_deref()),
                last_failure: last.map(|f| McpFailureRecordView {
                    timestamp_unix_ms: f.timestamp_unix_ms,
                    state: f.state,
                    reason: f.reason,
                }),
            });

            if !hist.is_empty() {
                payload.mcp_failure_history.push(McpFailureHistoryEntry {
                    server_name: cfg.name.clone(),
                    failures: hist
                        .into_iter()
                        .map(|f| McpFailureRecordView {
                            timestamp_unix_ms: f.timestamp_unix_ms,
                            state: f.state,
                            reason: f.reason,
                        })
                        .collect(),
                });
            }
        }
    }

    // ── Config ────────────────────────────────────────────────────────
    {
        let cfg = state.config.read().await;
        payload.config = ConfigSnapshot {
            llm_routing_mode: cfg.llm.routing_mode.clone(),
            llm_primary_model: cfg
                .llm
                .models
                .first()
                .map(|m| m.name.clone())
                .unwrap_or_default(),
            voice_enabled: cfg.voice.enabled,
            voice_stt_model: cfg.voice.stt_model.clone(),
            voice_tts_voice: cfg.voice.tts_voice.clone(),
            safety_max_concurrent_tools: cfg.safety.max_concurrent_tools,
            orchestrator_enabled: cfg.orchestrator.enabled,
            colab_enabled: cfg.colab.enabled,
            telegram_enabled: cfg.telegram.enabled,
            memory_max_facts: cfg.memory.max_facts,
            memory_decay_threshold: cfg.memory.decay_threshold,
            executive_enabled: cfg.executive.enabled,
            planner_enabled: cfg.planner.enabled,
            uncertainty_enabled: cfg.uncertainty.enabled,
            skill_compiler_enabled: cfg.skill_compiler.enabled,
            curiosity_enabled: cfg.curiosity.enabled,
            browser_agent_enabled: cfg.browser_agent.enabled,
            hardware_tier: state.hardware_info.tier.as_str().to_string(),
        };
    }

    // ── Tool Registry ─────────────────────────────────────────────────
    {
        let defs = state.tool_registry.list_defs();
        payload.overview.total_tools = defs.len() as u64;
        let mut by_cat: HashMap<String, u64> = HashMap::new();
        let mut by_risk: HashMap<String, u64> = HashMap::new();
        for def in &defs {
            *by_cat.entry(def.category.clone()).or_default() += 1;
            *by_risk
                .entry(format!("{:?}", def.default_tier))
                .or_default() += 1;
        }
        payload.tool_registry = ToolRegistryView {
            total_tools: defs.len() as u64,
            by_category: by_cat,
            by_risk_level: by_risk,
        };
    }

    // ── Colab ─────────────────────────────────────────────────────────
    {
        let snap = state.colab_runtime.read().await;
        payload.colab = ColabView {
            state: snap.state.as_str().to_string(),
            server_name: snap.sidecar_server_name.clone(),
            selected_notebook: snap.selected_notebook.clone().unwrap_or_default(),
            last_error: snap.last_error.clone(),
        };
    }

    // ── Test Reports ──────────────────────────────────────────────────
    let logs_dir = find_workspace_root().join("tests-logs");
    if logs_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&logs_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                if !name.starts_with("KRIA_TEST_REPORT_") || !name.ends_with(".md") {
                    continue;
                }
                let modified = entry
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);

                let content = std::fs::read_to_string(&path).unwrap_or_default();
                let passed = extract_report_count(&content, "Passed");
                let failed = extract_report_count(&content, "Failed");
                let skipped = extract_report_count(&content, "Skipped");
                let mode = content
                    .lines()
                    .find(|l| l.contains("Mode:"))
                    .map(|l| l.trim_start_matches("- Mode:").trim().to_string())
                    .unwrap_or_default();

                payload.test_reports.push(TestReportEntry {
                    filename: name,
                    modified_unix_ms: modified,
                    mode,
                    passed,
                    failed,
                    skipped,
                });
            }
        }
        payload
            .test_reports
            .sort_by(|a, b| b.modified_unix_ms.cmp(&a.modified_unix_ms));
    }

    // ── Cognitive Score ───────────────────────────────────────────────
    let cognitive_path = logs_dir.join("cognitive-score.json");
    if let Ok(raw) = std::fs::read_to_string(&cognitive_path) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) {
            let total = json["total_prompts"].as_u64().unwrap_or(0);
            let passed = json["passed"].as_u64().unwrap_or(0);
            let failed = json["failed"].as_u64().unwrap_or(0);
            let score_text = json["cognitive_score"].as_str().unwrap_or("0%");
            let score = score_text
                .trim_end_matches('%')
                .parse::<f64>()
                .unwrap_or(0.0);

            let mut top_failures = Vec::new();
            if let Some(failures) = json["failures"].as_array() {
                for f in failures.iter().take(20) {
                    if let Some(s) = f.as_str() {
                        let prompt_id = s
                            .split(']')
                            .next()
                            .unwrap_or("")
                            .trim_start_matches('[')
                            .trim()
                            .to_string();
                        let expected = s
                            .split("expected '")
                            .nth(1)
                            .and_then(|r| r.split('\'').next())
                            .unwrap_or("")
                            .to_string();
                        let actual = if s.contains("got Some(\"") {
                            s.split("got Some(\"")
                                .nth(1)
                                .and_then(|r| r.split("\")").next())
                                .unwrap_or("")
                                .to_string()
                        } else {
                            "None".to_string()
                        };
                        top_failures.push(CognitiveFailureEntry {
                            prompt_id,
                            expected,
                            actual,
                        });
                    }
                }
            }

            payload.cognitive_score = Some(CognitiveScoreView {
                zone: json["zone"]
                    .as_str()
                    .unwrap_or("cognitive_e2e")
                    .to_string(),
                total_prompts: total,
                passed,
                failed,
                score_pct: score,
                top_failures,
            });
        }
    }

    // ── Orchestrator ──────────────────────────────────────────────────
    {
        let orch = state.orchestrator.read().await;
        if let Some(ref o) = *orch {
            let snap = o.snapshot();
            payload.orchestrator = OrchestratorView {
                active: true,
                backend: format!("{:?}", snap.backend),
                ngl: snap.current_ngl,
                context_window: snap.current_context,
                degradation: format!("{:?}", snap.degradation),
                server_healthy: snap.server_healthy,
                active_turns: state
                    .orchestrator_active_turns
                    .load(std::sync::atomic::Ordering::Relaxed)
                    as u32,
            };
        }
    }

    // ── Hardware Info ─────────────────────────────────────────────────
    payload.system_health = SystemHealthSnapshot {
        cpu_cores: state.hardware_info.cpu_cores,
        ram_total_mb: state.hardware_info.total_ram_mb,
        vram_mb: state.hardware_info.vram_mb,
        vram_free_mb: state.hardware_info.vram_free_mb,
        gpu_name: state
            .hardware_info
            .gpu_name
            .clone()
            .unwrap_or_else(|| "CPU-only".to_string()),
        hostname: state.hardware_info.hostname.clone(),
        uptime_secs: get_system_uptime_secs(),
    };

    Ok(payload)
}

// ═══════════════════════════════════════════════════════════════════════════
//  Helpers
// ═══════════════════════════════════════════════════════════════════════════

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn find_workspace_root() -> PathBuf {
    let mut candidate = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    for _ in 0..10 {
        if let Ok(content) = std::fs::read_to_string(candidate.join("Cargo.toml")) {
            if content.contains("[workspace]") {
                return candidate;
            }
        }
        if !candidate.pop() {
            break;
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn extract_report_count(content: &str, label: &str) -> u64 {
    for line in content.lines() {
        if line.contains(label) && line.contains(':') {
            if let Some(val) = line.split(':').last() {
                return val.trim().parse::<u64>().unwrap_or(0);
            }
        }
    }
    0
}

fn infer_mcp_tags_static(name: &str, command: &str, args: &[String]) -> Vec<String> {
    let haystack = format!("{} {} {}", name, command, args.join(" ")).to_lowercase();
    let mut tags = Vec::new();
    if haystack.contains("google") || haystack.contains("gworkspace") || haystack.contains("gmail")
    {
        tags.push("google".into());
    }
    if haystack.contains("colab") {
        tags.push("colab".into());
    }
    if haystack.contains("filesystem") || name.eq_ignore_ascii_case("fs") {
        tags.push("filesystem".into());
    }
    if haystack.contains("npx") {
        tags.push("node".into());
    }
    if haystack.contains("uvx") || haystack.contains("python") {
        tags.push("python".into());
    }
    if tags.is_empty() {
        tags.push("custom".into());
    }
    tags
}

fn compute_remediation(
    cfg: &kria_core::config::McpServerConfig,
    error: Option<&str>,
) -> Option<String> {
    let error = error?;
    let lower = error.to_ascii_lowercase();
    if lower.contains("credentials.json") {
        return Some("OAuth credentials missing. Add credentials.json.".into());
    }
    if lower.contains("already exists") {
        return Some("Account exists without token. Reconnect to re-auth.".into());
    }
    if lower.contains("could not load token") {
        return Some("OAuth token missing. Reconnect to re-auth.".into());
    }
    if lower.contains("failed to spawn") || lower.contains("not found") {
        return Some(format!("Command '{}' not found. Install it.", cfg.command));
    }
    if lower.contains("exited") {
        return Some("Server exited unexpectedly. Restart it.".into());
    }
    let preview = if error.len() > 120 {
        &error[..120]
    } else {
        error
    };
    Some(format!("Check logs for: {}", preview))
}

fn get_system_uptime_secs() -> u64 {
    std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| {
            s.split_whitespace()
                .next()
                .and_then(|v| v.parse::<f64>().ok())
                .map(|v| v as u64)
        })
        .unwrap_or(0)
}
