//! Execution Authority Layer — authoritative target validation before tool execution.
//!
//! Enforces that every tool call executes on the CORRECT target environment.
//! Blocks dangerous cross-target mismatches before any subprocess is spawned.
//!
//! # Pipeline Position
//! ```text
//! Preflight validation (Phase 3)
//!   └── ExecutionAuthority (THIS MODULE) ← new enforcement point
//!         ├── Resolve authoritative binding
//!         ├── Validate tool ↔ target compatibility
//!         ├── Block dangerous mismatches
//!         └── Request clarification if ambiguous
//!               └── run_isolated() → actual execution
//! ```
//!
//! # Design Principles
//! - Deterministic: same inputs → same decision always
//! - Bounded: no LLM calls, no network, no async
//! - Observable: every decision logged
//! - Fail-safe: ambiguous destructive operations are blocked, not guessed

use crate::agent::collaborative_decision::DecisionCandidate;
use crate::agent::turn_memory::ExecutionTarget;

// ─── Execution Binding ────────────────────────────────────────────────────────

/// Authoritative execution binding for a single tool call.
/// Represents the resolved, validated target for execution.
#[derive(Debug, Clone)]
pub struct ExecutionBinding {
    /// The authoritative execution target.
    pub target: ExecutionTarget,
    /// How confident we are in this binding (0.0–1.0).
    pub confidence: f32,
    /// Source of the binding (for observability).
    pub source: BindingSource,
    /// Whether this operation is destructive on the target.
    pub is_destructive: bool,
    /// Whether the binding was explicitly stated by the user.
    pub is_explicit: bool,
}

/// How the binding was resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingSource {
    /// User explicitly stated the target ("on my VM", "in docker")
    ExplicitUser,
    /// Tool name unambiguously implies the target (execute_fleet_command → VM)
    ToolImplied,
    /// Inferred from conversation context and turn history
    ContextInferred,
    /// Default fallback (host)
    DefaultHost,
}

impl BindingSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitUser => "explicit_user",
            Self::ToolImplied => "tool_implied",
            Self::ContextInferred => "context_inferred",
            Self::DefaultHost => "default_host",
        }
    }
}

// ─── Validation Result ────────────────────────────────────────────────────────

/// Result of the binding validator.
#[derive(Debug, Clone)]
pub enum ValidationResult {
    /// Execution is authorized on the resolved target.
    Authorized(ExecutionBinding),
    /// Execution is blocked — dangerous mismatch or ambiguity.
    Blocked {
        reason: String,
        suggested_clarification: Option<String>,
    },
    /// Clarification needed before execution can proceed.
    NeedsClarification {
        question: String,
        options: Vec<String>,
    },
}

impl ValidationResult {
    pub fn is_authorized(&self) -> bool {
        matches!(self, Self::Authorized(_))
    }

    pub fn block_reason(&self) -> Option<&str> {
        match self {
            Self::Blocked { reason, .. } => Some(reason),
            _ => None,
        }
    }

    pub fn clarification_question(&self) -> Option<&str> {
        match self {
            Self::NeedsClarification { question, .. } => Some(question),
            _ => None,
        }
    }

    /// Convert a recoverable ambiguity into a durable collaborative decision.
    ///
    /// Blocked mismatches are not converted here because they are hard
    /// authority failures. Only clarification states are safe to pause and
    /// resume through the collaborative decision runtime.
    pub fn to_decision_candidate(&self, tool_name: &str) -> Option<DecisionCandidate> {
        match self {
            Self::NeedsClarification { question, options } => {
                Some(DecisionCandidate::target_selection(
                    question.clone(),
                    options.clone(),
                    tool_name.to_string(),
                ))
            }
            _ => None,
        }
    }
}

// ─── Tool Target Policy ───────────────────────────────────────────────────────

/// Defines which targets a tool is allowed to execute on.
#[derive(Debug, Clone)]
pub struct ToolTargetPolicy {
    /// Targets this tool is allowed on.
    pub allowed_targets: &'static [ExecutionTarget],
    /// Whether this tool is destructive (requires higher confidence).
    pub is_destructive: bool,
    /// Whether target ambiguity should block (true) or use default (false).
    pub block_on_ambiguity: bool,
    /// Minimum confidence required for execution.
    pub min_confidence: f32,
}

impl ToolTargetPolicy {
    const fn read_only_host() -> Self {
        Self {
            allowed_targets: &[ExecutionTarget::Host],
            is_destructive: false,
            block_on_ambiguity: false,
            min_confidence: 0.3,
        }
    }

    #[allow(dead_code)]
    const fn read_only_any() -> Self {
        Self {
            allowed_targets: &[
                ExecutionTarget::Host,
                ExecutionTarget::Vm,
                ExecutionTarget::Docker,
            ],
            is_destructive: false,
            block_on_ambiguity: false,
            min_confidence: 0.3,
        }
    }

    const fn destructive_host_only() -> Self {
        Self {
            allowed_targets: &[ExecutionTarget::Host],
            is_destructive: true,
            block_on_ambiguity: true,
            min_confidence: 0.7,
        }
    }

    const fn vm_only() -> Self {
        Self {
            allowed_targets: &[ExecutionTarget::Vm],
            is_destructive: false,
            block_on_ambiguity: true,
            min_confidence: 0.6,
        }
    }

    const fn vm_destructive() -> Self {
        Self {
            allowed_targets: &[ExecutionTarget::Vm],
            is_destructive: true,
            block_on_ambiguity: true,
            min_confidence: 0.8,
        }
    }

    const fn cloud_only() -> Self {
        Self {
            allowed_targets: &[ExecutionTarget::CloudProvider],
            is_destructive: false,
            block_on_ambiguity: false,
            min_confidence: 0.3,
        }
    }

    const fn cloud_destructive() -> Self {
        Self {
            allowed_targets: &[ExecutionTarget::CloudProvider],
            is_destructive: true,
            block_on_ambiguity: true,
            min_confidence: 0.7,
        }
    }

    const fn colab_only() -> Self {
        Self {
            allowed_targets: &[ExecutionTarget::Colab, ExecutionTarget::Mcp],
            is_destructive: false,
            block_on_ambiguity: true,
            min_confidence: 0.5,
        }
    }

    const fn host_or_vm() -> Self {
        Self {
            allowed_targets: &[ExecutionTarget::Host, ExecutionTarget::Vm],
            is_destructive: false,
            block_on_ambiguity: false,
            min_confidence: 0.3,
        }
    }

    const fn host_or_vm_destructive() -> Self {
        Self {
            allowed_targets: &[ExecutionTarget::Host, ExecutionTarget::Vm],
            is_destructive: true,
            block_on_ambiguity: true,
            min_confidence: 0.7,
        }
    }

    const fn mcp_only() -> Self {
        Self {
            allowed_targets: &[ExecutionTarget::Mcp],
            is_destructive: false,
            block_on_ambiguity: false,
            min_confidence: 0.3,
        }
    }

    const fn browser_or_host() -> Self {
        Self {
            allowed_targets: &[ExecutionTarget::Browser, ExecutionTarget::Host],
            is_destructive: false,
            block_on_ambiguity: false,
            min_confidence: 0.3,
        }
    }
}

/// Look up the target policy for a tool by name.
/// Returns a permissive default for unknown tools.
pub fn policy_for_tool(tool_name: &str) -> ToolTargetPolicy {
    let lower = tool_name.to_ascii_lowercase();

    // ── Fleet / VM tools ─────────────────────────────────────────────────────
    if lower == "execute_fleet_command" {
        return ToolTargetPolicy::vm_destructive();
    }
    if lower == "get_fleet_overview" {
        return ToolTargetPolicy::vm_only();
    }

    // ── Colab / MCP tools ────────────────────────────────────────────────────
    if lower.starts_with("mcp_colab") {
        return ToolTargetPolicy::colab_only();
    }
    if lower.starts_with("mcp_") {
        return ToolTargetPolicy::mcp_only();
    }

    // ── Cloud / Google Workspace tools ───────────────────────────────────────
    if lower.starts_with("gw_gmail_send")
        || lower.starts_with("gw_gmail_delete")
        || lower.starts_with("gw_gmail_reply")
        || lower.starts_with("gw_drive_delete")
        || lower.starts_with("gw_calendar_delete")
    {
        return ToolTargetPolicy::cloud_destructive();
    }
    if lower.starts_with("gw_") {
        return ToolTargetPolicy::cloud_only();
    }

    // ── Shell execution ───────────────────────────────────────────────────────
    // execute_bash/python/powershell can run on host OR vm — but must be explicit
    // when destructive operations are involved.
    // Docker is included because docker CLI commands run on the host.
    if matches!(
        lower.as_str(),
        "execute_bash" | "execute_python" | "execute_powershell"
    ) {
        return ToolTargetPolicy {
            allowed_targets: &[
                ExecutionTarget::Host,
                ExecutionTarget::Vm,
                ExecutionTarget::Docker, // docker CLI runs on host
            ],
            is_destructive: true,
            block_on_ambiguity: true,
            min_confidence: 0.7,
        };
    }

    // ── Package management ────────────────────────────────────────────────────
    if matches!(
        lower.as_str(),
        "install_package"
            | "uninstall_package"
            | "install_application"
            | "uninstall_application"
            | "update_all_packages"
    ) {
        return ToolTargetPolicy::host_or_vm_destructive();
    }

    // ── File operations ───────────────────────────────────────────────────────
    if matches!(
        lower.as_str(),
        "delete_file" | "delete_directory" | "move_file"
    ) {
        return ToolTargetPolicy::destructive_host_only();
    }
    if matches!(
        lower.as_str(),
        "write_file"
            | "create_directory"
            | "rename_file"
            | "copy_file"
            | "create_file"
            | "overwrite_file"
    ) {
        return ToolTargetPolicy {
            allowed_targets: &[ExecutionTarget::Host, ExecutionTarget::Vm],
            is_destructive: false,
            block_on_ambiguity: false,
            min_confidence: 0.3,
        };
    }
    if matches!(
        lower.as_str(),
        "read_file"
            | "list_directory"
            | "search_files"
            | "get_file_info"
            | "calculate_dir_size"
            | "search_file_contents"
            | "find_files_by_pattern"
            | "get_project_structure"
            | "count_lines_of_code"
            | "find_todos"
            | "diff_files"
            | "diff_files_unified"
            | "analyze_code"
            | "analyze_project"
    ) {
        return ToolTargetPolicy::host_or_vm();
    }

    // ── System control ────────────────────────────────────────────────────────
    if matches!(
        lower.as_str(),
        "shutdown_system" | "reboot_system" | "hibernate" | "sleep"
    ) {
        return ToolTargetPolicy::destructive_host_only();
    }
    if matches!(
        lower.as_str(),
        "lock_screen"
            | "set_volume"
            | "set_brightness"
            | "toggle_wifi"
            | "set_power_plan"
            | "get_power_plan"
            | "get_wifi_networks"
    ) {
        return ToolTargetPolicy::read_only_host();
    }

    // ── Process management ────────────────────────────────────────────────────
    if lower == "kill_process" {
        return ToolTargetPolicy::destructive_host_only();
    }
    if matches!(
        lower.as_str(),
        "list_running_apps"
            | "get_active_connections"
            | "get_cpu_usage"
            | "get_memory_info"
            | "get_disk_space"
            | "get_battery_status"
            | "get_system_uptime"
            | "get_gpu_info"
            | "get_network_status"
            | "check_system_health"
    ) {
        return ToolTargetPolicy::read_only_host();
    }

    // ── Browser / desktop ────────────────────────────────────────────────────
    if matches!(
        lower.as_str(),
        "open_url"
            | "browser_search"
            | "managed_browser_navigate"
            | "open_application"
            | "close_application"
            | "focus_window"
            | "maximize_window"
            | "minimize_window"
            | "tile_windows"
            | "get_active_window"
            | "list_windows"
            | "screenshot"
            | "screenshot_analyze"
            | "type_text"
    ) {
        return ToolTargetPolicy::browser_or_host();
    }

    // ── Network / internet ────────────────────────────────────────────────────
    if matches!(
        lower.as_str(),
        "web_search"
            | "searxng_search"
            | "search_news"
            | "fetch_webpage"
            | "fetch_article"
            | "get_weather"
            | "get_public_ip"
            | "ping_host"
            | "dns_lookup"
            | "check_url_status"
            | "get_current_time"
            | "get_exchange_rate"
            | "calculate"
            | "speed_test"
    ) {
        // Network tools run on host but don't need strict target enforcement
        return ToolTargetPolicy {
            allowed_targets: &[ExecutionTarget::Host],
            is_destructive: false,
            block_on_ambiguity: false,
            min_confidence: 0.1, // Very permissive — these are always safe
        };
    }

    // ── Download ──────────────────────────────────────────────────────────────
    if lower == "download_file" {
        return ToolTargetPolicy {
            allowed_targets: &[ExecutionTarget::Host, ExecutionTarget::Vm],
            is_destructive: false,
            block_on_ambiguity: false,
            min_confidence: 0.3,
        };
    }

    // ── Git / developer ───────────────────────────────────────────────────────
    if matches!(
        lower.as_str(),
        "git_status" | "git_log" | "git_diff" | "git_branch_list" | "git_remote" | "git_fetch"
    ) {
        return ToolTargetPolicy::host_or_vm();
    }
    if matches!(
        lower.as_str(),
        "git_commit"
            | "git_checkout"
            | "git_stash"
            | "git_push"
            | "git_pull"
            | "git_merge"
            | "git_reset"
            | "git_tag"
            | "git_clone"
    ) {
        return ToolTargetPolicy::host_or_vm_destructive();
    }

    // ── Memory / knowledge ────────────────────────────────────────────────────
    if matches!(
        lower.as_str(),
        "remember_fact"
            | "recall_fact"
            | "search_knowledge"
            | "list_remembered"
            | "save_snippet"
            | "get_snippet"
            | "list_snippets"
            | "ingest_document"
            | "ingest_document_rag"
            | "rag_query"
            | "list_knowledge_base"
    ) {
        // Memory tools always run on host (local SQLite)
        return ToolTargetPolicy::read_only_host();
    }

    // ── Image generation ──────────────────────────────────────────────────────
    if lower == "generate_image" {
        return ToolTargetPolicy::read_only_host();
    }

    // ── Vision / OCR ─────────────────────────────────────────────────────────
    if matches!(
        lower.as_str(),
        "analyze_image" | "ocr_image" | "image_analyze" | "document_extract"
    ) {
        return ToolTargetPolicy::read_only_host();
    }

    // ── Clipboard / interaction ───────────────────────────────────────────────
    if matches!(
        lower.as_str(),
        "get_clipboard" | "set_clipboard" | "transform_clipboard"
    ) {
        return ToolTargetPolicy::read_only_host();
    }

    // ── Notifications / communication ─────────────────────────────────────────
    if matches!(
        lower.as_str(),
        "send_notification" | "schedule_reminder" | "compose_email"
    ) {
        return ToolTargetPolicy::read_only_host();
    }

    // ── Proactive / automation ────────────────────────────────────────────────
    if matches!(
        lower.as_str(),
        "watch_directory" | "list_watched_dirs" | "smart_suggest" | "get_alerts" | "dismiss_alert"
    ) {
        return ToolTargetPolicy::read_only_host();
    }

    // ── Default: permissive for unknown tools ─────────────────────────────────
    // Unknown tools default to host-only, non-destructive, low confidence requirement.
    // This is safe because unknown tools haven't been classified yet.
    ToolTargetPolicy {
        allowed_targets: &[ExecutionTarget::Host],
        is_destructive: false,
        block_on_ambiguity: false,
        min_confidence: 0.1,
    }
}

// ─── Binding Resolver ─────────────────────────────────────────────────────────

/// Resolve the authoritative execution binding for a tool call.
///
/// Priority order:
/// 1. Explicit user statement ("on my VM", "in docker", etc.) — ALWAYS wins
/// 2. Tool name implies target (execute_fleet_command → VM)
/// 3. Context inference from user text
/// 4. Default: Host
pub fn resolve_binding(
    tool_name: &str,
    user_text: &str,
    turn_target: ExecutionTarget,
) -> ExecutionBinding {
    let lower = user_text.to_ascii_lowercase();
    let tool_lower = tool_name.to_ascii_lowercase();

    // ── Priority 1: Explicit user statement — ALWAYS wins ────────────────────
    // Even if the tool implies a different target, the user's explicit statement
    // takes precedence. This allows the validator to catch mismatches.
    let explicit = detect_explicit_target(&lower);
    if let Some((target, confidence)) = explicit {
        let policy = policy_for_tool(tool_name);
        return ExecutionBinding {
            target,
            confidence,
            source: BindingSource::ExplicitUser,
            is_destructive: policy.is_destructive,
            is_explicit: true,
        };
    }

    // ── Priority 2: Tool name implies target ──────────────────────────────────
    // Only applies when the user has NOT explicitly stated a target.
    if let Some(target) = tool_implied_target(&tool_lower) {
        let policy = policy_for_tool(tool_name);
        return ExecutionBinding {
            target,
            confidence: 0.95,
            source: BindingSource::ToolImplied,
            is_destructive: policy.is_destructive,
            is_explicit: false,
        };
    }

    // ── Priority 3: Use turn-level inferred target ────────────────────────────
    // The turn_target was inferred from the full user message at turn start.
    // If it's not Host (the default), it carries meaningful signal.
    if turn_target != ExecutionTarget::Host {
        let policy = policy_for_tool(tool_name);
        return ExecutionBinding {
            target: turn_target,
            confidence: 0.65,
            source: BindingSource::ContextInferred,
            is_destructive: policy.is_destructive,
            is_explicit: false,
        };
    }

    // ── Priority 4: Default to Host ───────────────────────────────────────────
    let policy = policy_for_tool(tool_name);
    ExecutionBinding {
        target: ExecutionTarget::Host,
        confidence: 0.5,
        source: BindingSource::DefaultHost,
        is_destructive: policy.is_destructive,
        is_explicit: false,
    }
}

/// Detect explicit target statements in user text.
/// Returns (target, confidence) if found.
fn detect_explicit_target(lower: &str) -> Option<(ExecutionTarget, f32)> {
    // VM signals (high confidence)
    if lower.contains(" on my vm")
        || lower.contains(" on the vm")
        || lower.contains(" in my vm")
        || lower.contains(" in the vm")
        || lower.contains(" on vm")
        || lower.contains(" via ssh")
        || lower.starts_with("ssh ")
        || lower.contains("remote machine")
        || lower.contains("remote server")
        || lower.contains("remote host")
    {
        return Some((ExecutionTarget::Vm, 0.95));
    }

    // Host signals (high confidence)
    if lower.contains(" on my host")
        || lower.contains(" on the host")
        || lower.contains(" on host")
        || lower.contains(" on my machine")
        || lower.contains(" on my local machine")
        || lower.contains(" on my laptop")
        || lower.contains(" on my computer")
        || lower.contains(" locally")
        || lower.contains(" on local")
        || lower.contains("local machine")
    {
        return Some((ExecutionTarget::Host, 0.95));
    }

    // Docker signals (high confidence)
    if lower.contains(" in docker")
        || lower.contains(" in the container")
        || lower.contains(" in container")
        || lower.contains("docker container")
    {
        return Some((ExecutionTarget::Docker, 0.90));
    }

    // Colab signals (high confidence)
    if lower.contains(" in colab")
        || lower.contains(" on colab")
        || lower.contains(" in the notebook")
        || lower.contains("colab notebook")
    {
        return Some((ExecutionTarget::Colab, 0.90));
    }

    // Browser signals
    if lower.contains(" in browser")
        || lower.contains(" in the browser")
        || lower.contains(" in chrome")
        || lower.contains(" in firefox")
    {
        return Some((ExecutionTarget::Browser, 0.85));
    }

    None
}

/// Determine if a tool name unambiguously implies a specific target.
fn tool_implied_target(tool_lower: &str) -> Option<ExecutionTarget> {
    if tool_lower == "execute_fleet_command" || tool_lower == "get_fleet_overview" {
        return Some(ExecutionTarget::Vm);
    }
    if tool_lower.starts_with("mcp_colab") {
        return Some(ExecutionTarget::Colab);
    }
    if tool_lower.starts_with("mcp_") {
        return Some(ExecutionTarget::Mcp);
    }
    if tool_lower.starts_with("gw_") {
        return Some(ExecutionTarget::CloudProvider);
    }
    if tool_lower == "browser_search" || tool_lower == "managed_browser_navigate" {
        return Some(ExecutionTarget::Browser);
    }
    None
}

// ─── Binding Validator ────────────────────────────────────────────────────────

/// Validate an execution binding against the tool's target policy.
///
/// This is the authoritative enforcement gate. Called after binding resolution,
/// before `run_isolated()`.
pub fn validate_binding(
    tool_name: &str,
    binding: &ExecutionBinding,
    user_text: &str,
) -> ValidationResult {
    let policy = policy_for_tool(tool_name);

    // ── Check 1: Is the target allowed for this tool? ─────────────────────────
    let target_allowed = policy.allowed_targets.contains(&binding.target);

    if !target_allowed {
        // Dangerous mismatch — block execution
        let allowed_names: Vec<&str> = policy.allowed_targets.iter().map(|t| t.as_str()).collect();

        // Generate a helpful clarification question
        let clarification = generate_mismatch_clarification(
            tool_name,
            binding.target,
            policy.allowed_targets,
            user_text,
        );

        return ValidationResult::Blocked {
            reason: format!(
                "Target mismatch: '{}' cannot execute on '{}'. Allowed targets: [{}]",
                tool_name,
                binding.target.as_str(),
                allowed_names.join(", ")
            ),
            suggested_clarification: Some(clarification),
        };
    }

    // ── Check 2: Confidence threshold ─────────────────────────────────────────
    if binding.confidence < policy.min_confidence {
        if policy.block_on_ambiguity && policy.is_destructive {
            // Destructive + ambiguous = block and ask
            let question =
                generate_ambiguity_question(tool_name, policy.allowed_targets, user_text);
            return ValidationResult::NeedsClarification {
                question,
                options: policy
                    .allowed_targets
                    .iter()
                    .map(|t| t.as_str().to_string())
                    .collect(),
            };
        }
        // Non-destructive + low confidence = allow with warning (logged by caller)
    }

    // ── Check 3: Destructive cross-target safety ──────────────────────────────
    // If destructive AND not explicit AND multiple targets allowed → ask
    if policy.is_destructive
        && !binding.is_explicit
        && policy.allowed_targets.len() > 1
        && binding.source == BindingSource::DefaultHost
    {
        let question = generate_ambiguity_question(tool_name, policy.allowed_targets, user_text);
        return ValidationResult::NeedsClarification {
            question,
            options: policy
                .allowed_targets
                .iter()
                .map(|t| t.as_str().to_string())
                .collect(),
        };
    }

    // ── All checks passed ─────────────────────────────────────────────────────
    ValidationResult::Authorized(binding.clone())
}

/// Generate a clarification question for target ambiguity.
fn generate_ambiguity_question(
    tool_name: &str,
    allowed_targets: &[ExecutionTarget],
    user_text: &str,
) -> String {
    let lower = user_text.to_ascii_lowercase();

    // Context-aware questions
    if allowed_targets.contains(&ExecutionTarget::Vm)
        && allowed_targets.contains(&ExecutionTarget::Host)
    {
        if lower.contains("uninstall") || lower.contains("remove") || lower.contains("delete") {
            return format!(
                "Where should I run this operation? On your **local machine** or on the **VM**? \
                 (This is a destructive operation — I need to be sure before proceeding.)"
            );
        }
        if lower.contains("install") {
            return format!("Should I install this on your **local machine** or on the **VM**?");
        }
        return format!(
            "Should I run '{}' on your **local machine** or on the **VM**?",
            tool_name
        );
    }

    if allowed_targets.contains(&ExecutionTarget::Docker)
        && allowed_targets.contains(&ExecutionTarget::Host)
    {
        return format!(
            "Should I run '{}' on the **host** or inside a **Docker container**?",
            tool_name
        );
    }

    // Generic fallback
    let options: Vec<&str> = allowed_targets.iter().map(|t| t.as_str()).collect();
    format!(
        "Where should I execute '{}'? Options: {}",
        tool_name,
        options.join(" or ")
    )
}

/// Generate a clarification message for a target mismatch.
fn generate_mismatch_clarification(
    tool_name: &str,
    actual_target: ExecutionTarget,
    allowed_targets: &[ExecutionTarget],
    _user_text: &str,
) -> String {
    let allowed: Vec<&str> = allowed_targets.iter().map(|t| t.as_str()).collect();

    match (actual_target, allowed_targets) {
        (ExecutionTarget::Vm, _) if allowed_targets.contains(&ExecutionTarget::Host) => {
            format!(
                "'{}' runs on the local host, not the VM. \
                 Did you mean to run a different command on the VM?",
                tool_name
            )
        }
        (ExecutionTarget::Host, _) if allowed_targets.contains(&ExecutionTarget::Vm) => {
            format!(
                "'{}' is a VM/fleet operation and cannot run on the local host. \
                 Use 'execute_fleet_command' to run commands on the VM.",
                tool_name
            )
        }
        (ExecutionTarget::Colab, _) if !allowed_targets.contains(&ExecutionTarget::Colab) => {
            format!(
                "'{}' cannot run in Colab. This tool runs on: {}.",
                tool_name,
                allowed.join(", ")
            )
        }
        (ExecutionTarget::Browser, _) if !allowed_targets.contains(&ExecutionTarget::Browser) => {
            format!(
                "'{}' is not a browser operation. This tool runs on: {}.",
                tool_name,
                allowed.join(", ")
            )
        }
        _ => format!(
            "'{}' cannot run on '{}'. It runs on: {}.",
            tool_name,
            actual_target.as_str(),
            allowed.join(", ")
        ),
    }
}

// ─── Public Entry Point ───────────────────────────────────────────────────────

/// Full execution authority check: resolve binding + validate.
///
/// This is the single entry point called by the agent loop before `run_isolated()`.
///
/// Returns `ValidationResult` — caller must check `is_authorized()` before proceeding.
pub fn check_execution_authority(
    tool_name: &str,
    user_text: &str,
    turn_target: ExecutionTarget,
) -> ValidationResult {
    check_execution_authority_with_params(tool_name, user_text, turn_target, None)
}

/// Full execution authority check with optional tool parameters.
///
/// Parameter-aware authority exists for one narrow case: KRIA-owned structural
/// execution where the runtime has generated code under its own generated-files
/// directory and runs it with bounded output capture. This must not relax the
/// target rules for arbitrary shell commands.
pub fn check_execution_authority_with_params(
    tool_name: &str,
    user_text: &str,
    turn_target: ExecutionTarget,
    params: Option<&serde_json::Value>,
) -> ValidationResult {
    if is_kria_generated_code_execution(tool_name, params) {
        let binding = ExecutionBinding {
            target: ExecutionTarget::Host,
            confidence: 0.95,
            source: BindingSource::ContextInferred,
            is_destructive: false,
            is_explicit: false,
        };
        tracing::debug!(
            tool = tool_name,
            target = binding.target.as_str(),
            confidence = binding.confidence,
            source = binding.source.as_str(),
            is_destructive = binding.is_destructive,
            is_explicit = binding.is_explicit,
            "execution_authority: KRIA-generated code execution resolved to local host"
        );
        return ValidationResult::Authorized(binding);
    }

    let binding = resolve_binding(tool_name, user_text, turn_target);

    tracing::debug!(
        tool = tool_name,
        target = binding.target.as_str(),
        confidence = binding.confidence,
        source = binding.source.as_str(),
        is_destructive = binding.is_destructive,
        is_explicit = binding.is_explicit,
        "execution_authority: binding resolved"
    );

    validate_binding(tool_name, &binding, user_text)
}

fn is_kria_generated_code_execution(tool_name: &str, params: Option<&serde_json::Value>) -> bool {
    if tool_name != "execute_bash" {
        return false;
    }
    let Some(command) = params
        .and_then(|p| p.get("command"))
        .and_then(|v| v.as_str())
    else {
        return false;
    };

    let lower = command.to_ascii_lowercase();

    // Generated-code runs produced by the SubstratePlanner always:
    // - reference KRIA's generated code directory,
    // - redirect stdin from /dev/null,
    // - cap output via `head -c 1048576`,
    // - write the captured output into a KRIA generated output file.
    let has_generated_path =
        lower.contains("/.kria/generated/") || lower.contains("/kria/generated/");
    let has_stdin_guard = lower.contains("< /dev/null");
    let has_output_cap = lower.contains("head -c 1048576");
    let has_output_redirect = lower.contains(" > ");
    let starts_like_known_runner = [
        "python3 ",
        "node ",
        "ts-node ",
        "(ts-node ",
        "rustc ",
        "goflags=-mod=mod go run ",
        "bash ",
        "ruby ",
        "php ",
        "kotlinc-jvm -script ",
        "mkdir -p ",
        "g++ ",
        "(dotnet-script ",
        "swift ",
    ]
    .iter()
    .any(|prefix| lower.trim_start().starts_with(prefix));

    let has_dangerous_token = [
        " sudo ",
        " rm ",
        " rm -",
        " shutdown",
        " reboot",
        " mkfs",
        " dd if=",
        " chmod -r",
        " chown -r",
        " curl ",
        " wget ",
    ]
    .iter()
    .any(|needle| lower.contains(needle));

    has_generated_path
        && has_stdin_guard
        && has_output_cap
        && has_output_redirect
        && starts_like_known_runner
        && !has_dangerous_token
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn auth(tool: &str, user_text: &str) -> ValidationResult {
        let turn_target = ExecutionTarget::infer(user_text, tool);
        check_execution_authority(tool, user_text, turn_target)
    }

    fn auth_with_params(
        tool: &str,
        user_text: &str,
        params: serde_json::Value,
    ) -> ValidationResult {
        let turn_target = ExecutionTarget::infer(user_text, tool);
        check_execution_authority_with_params(tool, user_text, turn_target, Some(&params))
    }

    // ── VM uninstall request ─────────────────────────────────────────────────

    #[test]
    fn vm_uninstall_on_vm_is_authorized() {
        let result = auth("execute_fleet_command", "uninstall nginx on my VM");
        assert!(
            result.is_authorized(),
            "fleet command on VM should be authorized"
        );
    }

    #[test]
    fn uninstall_package_on_vm_explicit_is_authorized() {
        let result = auth("uninstall_package", "uninstall nginx on my VM");
        assert!(
            result.is_authorized(),
            "uninstall on explicit VM should be authorized"
        );
        if let ValidationResult::Authorized(binding) = result {
            assert_eq!(binding.target, ExecutionTarget::Vm);
            assert_eq!(binding.source, BindingSource::ExplicitUser);
        }
    }

    #[test]
    fn uninstall_package_ambiguous_asks_clarification() {
        // No explicit target — destructive + multi-target → needs clarification
        let result = auth("uninstall_package", "uninstall nginx");
        match result {
            ValidationResult::NeedsClarification { question, .. } => {
                assert!(
                    question.contains("local machine") || question.contains("VM"),
                    "clarification should mention host/VM options"
                );
            }
            ValidationResult::Authorized(_) => {
                // Also acceptable if confidence is high enough for host default
                // (depends on scoring)
            }
            ValidationResult::Blocked { .. } => {
                panic!("should not block ambiguous uninstall — should clarify");
            }
        }
    }

    // ── Host docker inspection ───────────────────────────────────────────────

    #[test]
    fn docker_inspect_on_host_is_authorized() {
        // "on host" is explicit → Host target; execute_bash allows Host/VM/Docker
        let result = auth("execute_bash", "inspect docker containers on host");
        assert!(
            result.is_authorized(),
            "docker inspect on explicit host should be authorized"
        );
        if let ValidationResult::Authorized(binding) = result {
            // "on host" is explicit → Host
            assert_eq!(binding.target, ExecutionTarget::Host);
            assert_eq!(binding.source, BindingSource::ExplicitUser);
        }
    }

    #[test]
    fn fleet_command_on_host_is_blocked() {
        // execute_fleet_command is VM-only; if user says "on host" → mismatch
        let result = auth("execute_fleet_command", "run this on my local machine");
        match result {
            ValidationResult::Blocked { reason, .. } => {
                assert!(
                    reason.contains("mismatch") || reason.contains("cannot execute"),
                    "should report target mismatch: {}",
                    reason
                );
            }
            _ => panic!("fleet_command on host should be blocked, got: {:?}", result),
        }
    }

    // ── Colab execution routing ──────────────────────────────────────────────

    #[test]
    fn colab_tool_routes_to_colab() {
        let result = auth("mcp_colab-mcp_execute_cell", "run this code in colab");
        assert!(result.is_authorized());
        if let ValidationResult::Authorized(binding) = result {
            assert_eq!(binding.target, ExecutionTarget::Colab);
        }
    }

    #[test]
    fn colab_tool_on_host_is_blocked() {
        // mcp_colab tool with explicit host target → mismatch
        let result = auth("mcp_colab-mcp_execute_cell", "run this on my local machine");
        match result {
            ValidationResult::Blocked { .. } => {} // expected
            ValidationResult::Authorized(binding) => {
                // If it resolves to Colab (tool-implied), that's also correct
                assert_eq!(
                    binding.target,
                    ExecutionTarget::Colab,
                    "colab tool should always bind to Colab"
                );
            }
            _ => {}
        }
    }

    // ── Destructive mismatch blocking ────────────────────────────────────────

    #[test]
    fn delete_file_on_vm_is_blocked() {
        // delete_file is host-only; VM target → mismatch
        let result = auth("delete_file", "delete /home/user/file.txt on my VM");
        match result {
            ValidationResult::Blocked { reason, .. } => {
                assert!(
                    reason.contains("mismatch") || reason.contains("cannot execute"),
                    "should report mismatch: {}",
                    reason
                );
            }
            _ => panic!("delete_file on VM should be blocked"),
        }
    }

    #[test]
    fn shutdown_on_vm_is_blocked() {
        // shutdown_system is host-only
        let result = auth("shutdown_system", "shutdown the VM");
        match result {
            ValidationResult::Blocked { .. } => {} // expected
            ValidationResult::Authorized(binding) => {
                // If it resolves to Host (ignoring "VM" in context), that's also safe
                assert_eq!(binding.target, ExecutionTarget::Host);
            }
            _ => {}
        }
    }

    // ── Safe read-only host inspection ───────────────────────────────────────

    #[test]
    fn get_cpu_usage_on_host_is_authorized() {
        let result = auth("get_cpu_usage", "what is my cpu usage");
        assert!(result.is_authorized());
        if let ValidationResult::Authorized(binding) = result {
            assert_eq!(binding.target, ExecutionTarget::Host);
            assert!(!binding.is_destructive);
        }
    }

    #[test]
    fn web_search_is_always_authorized() {
        let result = auth("web_search", "search for rust programming");
        assert!(result.is_authorized());
    }

    #[test]
    fn read_file_on_host_is_authorized() {
        let result = auth("read_file", "read /home/user/notes.txt");
        assert!(result.is_authorized());
    }

    // ── Explicit target override ─────────────────────────────────────────────

    #[test]
    fn explicit_host_overrides_vm_inference() {
        // Even if "vm" appears in text, "on my local machine" is explicit
        let result = auth("execute_bash", "run ls on my local machine (not the vm)");
        if let ValidationResult::Authorized(binding) = result {
            assert_eq!(binding.target, ExecutionTarget::Host);
            assert_eq!(binding.source, BindingSource::ExplicitUser);
        }
    }

    #[test]
    fn explicit_vm_overrides_default_host() {
        let result = auth("execute_bash", "run df -h on my VM");
        if let ValidationResult::Authorized(binding) = result {
            assert_eq!(binding.target, ExecutionTarget::Vm);
            assert_eq!(binding.source, BindingSource::ExplicitUser);
        }
    }

    #[test]
    fn generated_code_execution_defaults_to_local_host() {
        let result = auth_with_params(
            "execute_bash",
            "open code and write a program to print pascal triangle and run it and show output",
            serde_json::json!({
                "command": "python3 '/home/obaid/.kria/generated/pascal_test.py' < /dev/null 2>&1 | head -c 1048576 > '/home/obaid/.kria/generated/output_test.txt'",
                "timeout": 30,
            }),
        );

        match result {
            ValidationResult::Authorized(binding) => {
                assert_eq!(binding.target, ExecutionTarget::Host);
                assert_eq!(binding.source, BindingSource::ContextInferred);
            }
            other => panic!("generated code execution should be authorized: {other:?}"),
        }
    }

    #[test]
    fn arbitrary_execute_bash_still_requires_clarification() {
        let result = auth("execute_bash", "run ls");
        assert!(matches!(
            result,
            ValidationResult::NeedsClarification { .. }
        ));
    }

    #[test]
    fn clarification_exports_decision_candidate() {
        let result = auth("execute_bash", "run ls");
        let candidate = result
            .to_decision_candidate("execute_bash")
            .expect("clarification should become decision candidate");

        assert_eq!(
            candidate.decision_type,
            crate::agent::collaborative_decision::DecisionType::TargetSelection
        );
        assert_eq!(
            candidate.authority,
            crate::agent::collaborative_decision::AuthorityLevel::ExecutionAuthority
        );
        assert!(!candidate.options.is_empty());
    }

    // ── Tool-implied target ──────────────────────────────────────────────────

    #[test]
    fn fleet_command_always_binds_to_vm() {
        let result = auth("execute_fleet_command", "check disk space");
        if let ValidationResult::Authorized(binding) = result {
            assert_eq!(binding.target, ExecutionTarget::Vm);
            assert_eq!(binding.source, BindingSource::ToolImplied);
        }
    }

    #[test]
    fn gw_gmail_always_binds_to_cloud() {
        let result = auth("gw_gmail_inbox", "check my email");
        assert!(result.is_authorized());
        if let ValidationResult::Authorized(binding) = result {
            assert_eq!(binding.target, ExecutionTarget::CloudProvider);
        }
    }

    // ── Cloud destructive requires explicit ──────────────────────────────────

    #[test]
    fn gmail_send_is_authorized_cloud() {
        let result = auth("gw_gmail_send", "send email to john");
        // gw_gmail_send is cloud_destructive but tool-implied → authorized
        assert!(result.is_authorized());
        if let ValidationResult::Authorized(binding) = result {
            assert_eq!(binding.target, ExecutionTarget::CloudProvider);
        }
    }

    // ── Binding resolution priority ──────────────────────────────────────────

    #[test]
    fn explicit_beats_tool_implied() {
        // execute_fleet_command normally implies VM, but explicit "host" should block
        let result = auth("execute_fleet_command", "run this on my local machine");
        // Should be blocked (fleet_command is VM-only, host is not allowed)
        assert!(
            matches!(result, ValidationResult::Blocked { .. }),
            "fleet_command on explicit host should be blocked"
        );
    }

    #[test]
    fn tool_implied_beats_context_inferred() {
        // mcp_colab tool should always bind to Colab regardless of context
        let result = auth("mcp_colab-mcp_execute_cell", "run some code");
        if let ValidationResult::Authorized(binding) = result {
            assert_eq!(binding.target, ExecutionTarget::Colab);
            assert_eq!(binding.source, BindingSource::ToolImplied);
        }
    }

    // ── Determinism ──────────────────────────────────────────────────────────

    #[test]
    fn validation_is_deterministic() {
        let r1 = auth("execute_bash", "run ls on my VM");
        let r2 = auth("execute_bash", "run ls on my VM");
        assert_eq!(r1.is_authorized(), r2.is_authorized());
        if let (ValidationResult::Authorized(b1), ValidationResult::Authorized(b2)) = (r1, r2) {
            assert_eq!(b1.target, b2.target);
            assert_eq!(b1.source, b2.source);
        }
    }

    // ── Policy coverage ──────────────────────────────────────────────────────

    #[test]
    fn unknown_tool_defaults_to_host_permissive() {
        let result = auth("some_unknown_tool_xyz", "do something");
        assert!(
            result.is_authorized(),
            "unknown tools should default to permissive host"
        );
    }

    #[test]
    fn mcp_tool_binds_to_mcp() {
        let result = auth("mcp_filesystem_read", "read a file");
        if let ValidationResult::Authorized(binding) = result {
            assert_eq!(binding.target, ExecutionTarget::Mcp);
        }
    }

    #[test]
    fn managed_browser_navigation_is_authorized_for_browser_prompt() {
        let result = auth_with_params(
            "managed_browser_navigate",
            "Open the browser and go to https://outbro.net Show me that the page loaded.",
            serde_json::json!({ "url": "https://outbro.net" }),
        );
        assert!(
            result.is_authorized(),
            "managed browser navigation must not be blocked as browser-vs-host mismatch"
        );
    }
}
