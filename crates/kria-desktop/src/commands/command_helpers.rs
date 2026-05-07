use super::*;

pub(crate) fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

fn trim_forensic_evidence(raw: &str) -> String {
    let bytes = raw.as_bytes();
    if bytes.len() <= IRONCLAD_FORENSIC_EVIDENCE_MAX_BYTES {
        return raw.to_string();
    }

    let truncation_notice = format!(
        "\n...[truncated forensic evidence by {} bytes]",
        bytes
            .len()
            .saturating_sub(IRONCLAD_FORENSIC_EVIDENCE_MAX_BYTES)
    );

    let allowed = IRONCLAD_FORENSIC_EVIDENCE_MAX_BYTES.saturating_sub(truncation_notice.len());
    let mut clipped = String::from_utf8_lossy(&bytes[..allowed]).to_string();
    clipped.push_str(&truncation_notice);
    clipped
}

fn detect_last_gasp_signature(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("last_gasp")
        || lower.contains("last-gasp")
        || (lower.contains("terminal_state") && lower.contains("command_id"))
}

fn make_ironclad_forensic_record(
    category: &str,
    severity: &str,
    summary: String,
    evidence: String,
    source: &str,
) -> IroncladForensicRecord {
    let evidence_trimmed = trim_forensic_evidence(&evidence);
    let last_gasp_detected =
        detect_last_gasp_signature(&evidence_trimmed) || detect_last_gasp_signature(&summary);

    IroncladForensicRecord {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp_unix_ms: unix_now_ms(),
        category: category.to_string(),
        severity: severity.to_string(),
        summary,
        source: source.to_string(),
        evidence: evidence_trimmed,
        last_gasp_detected,
    }
}

pub(crate) async fn append_ironclad_forensic_record(
    log: &Arc<RwLock<Vec<IroncladForensicRecord>>>,
    app: &AppHandle,
    category: &str,
    severity: &str,
    summary: impl Into<String>,
    evidence: impl Into<String>,
    source: &str,
) -> IroncladForensicRecord {
    let record =
        make_ironclad_forensic_record(category, severity, summary.into(), evidence.into(), source);

    {
        let mut guard = log.write().await;
        guard.push(record.clone());
        if guard.len() > IRONCLAD_FORENSIC_MAX_ENTRIES {
            let overflow = guard.len() - IRONCLAD_FORENSIC_MAX_ENTRIES;
            guard.drain(0..overflow);
        }
    }

    let _ = app.emit("ironclad:forensic", serde_json::json!(record.clone()));
    record
}

fn discover_from_roots(file_name: &str) -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            roots.push(parent.to_path_buf());
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }

    for start in roots {
        let mut dir = Some(start.as_path());
        while let Some(current) = dir {
            let candidate = current.join(file_name);
            if candidate.exists() {
                return Some(candidate);
            }

            dir = current.parent();
            if dir.map(|path| path == Path::new("/")).unwrap_or(true) {
                break;
            }
        }
    }

    None
}

fn resolve_ironclad_config_path() -> PathBuf {
    if let Ok(path) = std::env::var("KRIA_SYSTEM_CONFIG_PATH") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    discover_from_roots("kria_config.toml").unwrap_or_else(|| PathBuf::from("kria_config.toml"))
}

pub(crate) fn load_ironclad_system_config_with_path() -> (PathBuf, KriaSystemConfig) {
    let path = resolve_ironclad_config_path();
    let config = KriaSystemConfig::load(Some(path.as_path()));
    (path, config)
}

pub(crate) fn merge_ironclad_config_document(
    mut document: toml::Value,
    config: &KriaSystemConfig,
) -> Result<toml::Value, String> {
    if !document.is_table() {
        document = toml::Value::Table(toml::map::Map::new());
    }

    let Some(table) = document.as_table_mut() else {
        return Err("Unable to access TOML root table".to_string());
    };

    table.insert(
        "qos".to_string(),
        toml::Value::try_from(config.qos.clone()).map_err(|e| e.to_string())?,
    );
    table.insert(
        "target_pool".to_string(),
        toml::Value::try_from(config.target_pool.clone()).map_err(|e| e.to_string())?,
    );
    table.insert(
        "snapshot".to_string(),
        toml::Value::try_from(config.snapshot.clone()).map_err(|e| e.to_string())?,
    );

    Ok(document)
}

pub(crate) fn persist_ironclad_system_config(
    path: &Path,
    config: &KriaSystemConfig,
) -> Result<(), String> {
    let existing = if path.exists() {
        std::fs::read_to_string(path).map_err(|e| e.to_string())?
    } else {
        String::new()
    };

    let parsed = if existing.trim().is_empty() {
        toml::Value::Table(toml::map::Map::new())
    } else {
        toml::from_str::<toml::Value>(&existing).map_err(|e| e.to_string())?
    };

    let merged = merge_ironclad_config_document(parsed, config)?;
    let encoded = toml::to_string_pretty(&merged).map_err(|e| e.to_string())?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    std::fs::write(path, encoded).map_err(|e| e.to_string())
}

pub(crate) fn is_colab_bootstrap_tool_name(tool_name: &str) -> bool {
    tool_name
        .to_ascii_lowercase()
        .ends_with(COLAB_BROWSER_BOOTSTRAP_TOOL)
}

pub(crate) fn build_tool_only_fallback_message(
    name: &str,
    success: bool,
    result: &serde_json::Value,
) -> String {
    let metadata = compute_tool_result_metadata(name, result);
    let summary = summarize_tool_turn_for_history(name, success, result, &metadata);

    if success {
        format!(
            "{summary}\n\n⚠️ Local model became unavailable while preparing the final response. Tool output above is complete."
        )
    } else {
        format!("{summary}\n\n⚠️ Local model became unavailable after a tool failure.")
    }
}

// Tiny probe image used by unit tests that validate native image thumbnail fallback.
#[cfg(test)]
pub(crate) const OCR_HEALTH_PROBE_IMAGE_BYTES: &[u8] = b"P3\n1 1\n255\n255 255 255\n";

#[derive(Debug)]
pub(crate) struct OcrProbeState {
    pub(crate) in_flight: bool,
    pub(crate) next_allowed_at: std::time::Instant,
    pub(crate) consecutive_failures: u32,
}

impl Default for OcrProbeState {
    fn default() -> Self {
        Self {
            in_flight: false,
            next_allowed_at: std::time::Instant::now(),
            consecutive_failures: 0,
        }
    }
}

static OCR_PROBE_STATE: std::sync::OnceLock<tokio::sync::Mutex<OcrProbeState>> =
    std::sync::OnceLock::new();

pub(crate) fn ocr_probe_state() -> &'static tokio::sync::Mutex<OcrProbeState> {
    OCR_PROBE_STATE.get_or_init(|| tokio::sync::Mutex::new(OcrProbeState::default()))
}

pub(crate) async fn finalize_ocr_probe_schedule(success: bool) {
    let mut state = ocr_probe_state().lock().await;
    state.in_flight = false;
    if success {
        state.consecutive_failures = 0;
        state.next_allowed_at = std::time::Instant::now() + std::time::Duration::from_secs(30);
    } else {
        let failures = state.consecutive_failures.saturating_add(1).min(6);
        state.consecutive_failures = failures;
        let backoff_secs = (10u64.saturating_mul(1u64 << (failures.saturating_sub(1)))).min(300);
        state.next_allowed_at =
            std::time::Instant::now() + std::time::Duration::from_secs(backoff_secs);
    }
}

fn encode_base64_bytes(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

pub(crate) fn build_native_preprocessed_attachment(path: &str) -> Option<ImageAttachment> {
    build_native_preprocessed_attachment_with_max(path, 768)
}

pub(crate) fn build_native_preprocessed_attachment_with_max(
    path: &str,
    max_dim: u32,
) -> Option<ImageAttachment> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }

    let path_obj = Path::new(trimmed);
    if !path_obj.exists() {
        return None;
    }

    // Native fallback preprocessing: generate a normalized PNG thumbnail.
    let thumb_bytes =
        kria_core::preprocessing::image::ImageProcessor::thumbnail(path_obj, max_dim).ok()?;

    Some(ImageAttachment {
        data: encode_base64_bytes(&thumb_bytes),
        mime_type: "image/png".to_string(),
    })
}

/// Find a binary on the system PATH.
pub(crate) fn which_binary(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(name))
            .find(|p| p.exists())
    })
}

pub(crate) fn local_api_base_url(host: &str, port: u16) -> String {
    let probe_host = match host {
        "0.0.0.0" | "::" => "127.0.0.1",
        other => other,
    };
    format!("http://{probe_host}:{port}")
}

pub(crate) fn build_tool_descriptions_for_prompt(tool_defs: &[registry::ToolDef]) -> String {
    // Categories whose tools are so numerous that listing them individually
    // would crowd out other categories. They are collapsed into a single
    // summary line so important tools (image, internet, shell, …) always appear.
    const COLLAPSED_CATEGORIES: &[&str] = &["google_workspace"];

    // Minimum number of lines reserved for non-collapsed tools.
    const MAX_TOOL_LINES: usize = 80;

    // Separate collapsed from normal tools.
    let mut collapsed_groups: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut normal_defs: Vec<registry::ToolDef> = Vec::new();

    for def in tool_defs {
        if COLLAPSED_CATEGORIES.contains(&def.category.as_str()) {
            collapsed_groups
                .entry(def.category.clone())
                .or_default()
                .push(def.name.clone());
        } else {
            normal_defs.push(def.clone());
        }
    }

    // Sort non-collapsed tools: category then name.
    normal_defs.sort_by(|a, b| a.category.cmp(&b.category).then(a.name.cmp(&b.name)));

    let total = tool_defs.len();
    let visible_defs: Vec<registry::ToolDef> =
        normal_defs.into_iter().take(MAX_TOOL_LINES).collect();
    let omitted = total.saturating_sub(
        visible_defs.len() + collapsed_groups.values().map(|v| v.len()).sum::<usize>(),
    );

    let mut lines = Vec::with_capacity(visible_defs.len() + collapsed_groups.len() + 4);
    lines.push(format!(
        "You can call {} tools via function-calling. Use tool schemas for exact arguments.",
        total
    ));
    lines.push("Tool catalog (name [category]: summary):".to_string());
    if omitted > 0 {
        lines.push(format!(
            "{} additional low-priority tools are available via function schemas.",
            omitted
        ));
    }

    // Emit collapsed category summaries first.
    for (cat, names) in &collapsed_groups {
        lines.push(format!(
            "- [{}]: {} tools ({}) — call any by exact name via tool schema.",
            cat,
            names.len(),
            names.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
                + if names.len() > 5 { ", …" } else { "" }
        ));
    }

    // Emit individual tool lines.
    for def in visible_defs {
        let mut line = format!(
            "- {} [{}]: {}",
            def.name,
            def.category,
            kria_core::infra::pipeline_trace::sanitize_text_for_logs(&def.description, 96)
        );

        let param_names: Vec<&str> = def
            .parameters
            .iter()
            .take(3)
            .map(|p| p.name.as_str())
            .collect();
        if !param_names.is_empty() {
            if def.parameters.len() > 3 {
                line.push_str(&format!(" | params: {}, ...", param_names.join(", ")));
            } else {
                line.push_str(&format!(" | params: {}", param_names.join(", ")));
            }
        }

        lines.push(line);
    }

    lines.join("\n")
}

fn telegram_api_url(config: &KriaConfig) -> String {
    format!("http://{}:{}", config.server.host, config.server.port)
}

fn update_server_env_var(
    env: &mut std::collections::HashMap<String, String>,
    key: &str,
    value: Option<String>,
) -> bool {
    match value.filter(|v| !v.trim().is_empty()) {
        Some(next) => {
            if env.get(key) == Some(&next) {
                false
            } else {
                env.insert(key.to_string(), next);
                true
            }
        }
        None => env.remove(key).is_some(),
    }
}

fn should_manage_local_telegram_api_url(current: Option<&String>) -> bool {
    current
        .map(|url| {
            let lower = url.to_ascii_lowercase();
            lower.contains("127.0.0.1") || lower.contains("localhost") || lower.contains("0.0.0.0")
        })
        .unwrap_or(true)
}

pub(crate) fn sync_telegram_mcp_server_config(config: &mut KriaConfig) -> bool {
    let mut changed = false;
    let desired_enabled = config.telegram.enabled;
    let desired_bot_token = config.telegram.bot_token.clone();
    let desired_chat_ids = config.telegram.allowed_chat_ids.clone();
    let desired_api_url = telegram_api_url(config);

    if let Some(server) = config
        .mcp
        .servers
        .iter_mut()
        .find(|s| s.name.eq_ignore_ascii_case("telegram"))
    {
        if server.enabled != desired_enabled {
            server.enabled = desired_enabled;
            changed = true;
        }

        changed |= update_server_env_var(
            &mut server.env,
            "TELEGRAM_BOT_TOKEN",
            Some(desired_bot_token),
        );
        changed |=
            update_server_env_var(&mut server.env, "TELEGRAM_CHAT_IDS", Some(desired_chat_ids));

        if should_manage_local_telegram_api_url(server.env.get("KRIA_API_URL")) {
            changed |=
                update_server_env_var(&mut server.env, "KRIA_API_URL", Some(desired_api_url));
        }
    }

    changed
}

fn default_google_mcp_config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    PathBuf::from(home).join(".google-mcp")
}

pub(crate) fn configured_google_workspace_server(
    config: &KriaConfig,
) -> Option<&kria_core::config::McpServerConfig> {
    config
        .mcp
        .servers
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case("gworkspace"))
}

pub(crate) fn google_mcp_config_dir_from_config(config: &KriaConfig) -> PathBuf {
    configured_google_workspace_server(config)
        .and_then(|server| server.env.get(GOOGLE_MCP_CONFIG_DIR_ENV).cloned())
        .filter(|v| !v.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(default_google_mcp_config_dir)
}

pub(crate) fn google_account_from_config(config: &KriaConfig) -> String {
    configured_google_workspace_server(config)
        .and_then(|server| server.env.get(GOOGLE_ACCOUNT_ENV_KEY).cloned())
        .filter(|v| !v.trim().is_empty())
        .or_else(|| std::env::var(GOOGLE_ACCOUNT_ENV_KEY).ok())
        .unwrap_or_else(|| GOOGLE_DEFAULT_ACCOUNT.into())
}

pub(crate) fn apply_google_runtime_env_from_config(config: &KriaConfig) {
    let account = google_account_from_config(config);
    let config_dir = google_mcp_config_dir_from_config(config);

    std::env::set_var(GOOGLE_ACCOUNT_ENV_KEY, account);
    std::env::set_var(
        GOOGLE_MCP_CONFIG_DIR_ENV,
        config_dir.to_string_lossy().to_string(),
    );
}

pub(crate) fn sync_google_workspace_server_config(
    config: &mut KriaConfig,
    account: Option<&str>,
) -> bool {
    let mut changed = false;
    let desired_account = account
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .unwrap_or_else(|| google_account_from_config(config));

    if let Some(server) = config
        .mcp
        .servers
        .iter_mut()
        .find(|s| s.name.eq_ignore_ascii_case("gworkspace"))
    {
        changed |= update_server_env_var(
            &mut server.env,
            GOOGLE_ACCOUNT_ENV_KEY,
            Some(desired_account),
        );
    }

    changed
}

pub(crate) fn emit_agent_stage(
    app: &AppHandle,
    step: &str,
    message: &str,
    detail: Option<serde_json::Value>,
) {
    let detail_value = detail.unwrap_or(serde_json::Value::Null);
    let payload = serde_json::json!({
        "step": step,
        "message": message,
        "detail": detail_value.clone(),
        "ts": Utc::now().to_rfc3339(),
    });
    let _ = app.emit("agent:stage", payload);

    if kria_core::infra::pipeline_trace::pipeline_debug_enabled() {
        tracing::debug!(
            target: "kria_pipeline",
            step = step,
            message = message,
            detail = ?detail_value,
            "agent stage emitted"
        );
    }
}

pub(crate) async fn emit_colab_status_event(app: &AppHandle, state: &AppState) {
    let payload = collect_colab_tier_status(state).await;
    let _ = app.emit("colab:status", payload);
}
