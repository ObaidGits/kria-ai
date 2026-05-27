//! Active Turn Memory — lightweight per-turn operational state.
//!
//! Tracks what has been accomplished in the current turn so the agent can:
//! - Detect when the user's goal is satisfied (stop tool loops early)
//! - Prevent duplicate successful tool calls (memoization)
//! - Maintain execution target context (host/vm/docker/colab)
//! - Provide structured evidence for the verifier
//!
//! # Design Principles
//! - Zero LLM calls — all decisions are deterministic
//! - Bounded: cleared at turn start, never persists across turns
//! - Observable: all state changes are logged
//! - Minimal: only tracks what's needed for execution decisions

use std::collections::{HashMap, HashSet};
use std::time::Instant;

// ─── Execution Target ─────────────────────────────────────────────────────────

/// Where a tool should execute.
/// Resolved once per turn from context signals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecutionTarget {
    /// Local host machine (default)
    Host,
    /// Remote VM via SSH/fleet
    Vm,
    /// Docker container
    Docker,
    /// Google Colab (cloud notebook)
    Colab,
    /// Browser automation
    Browser,
    /// MCP server
    Mcp,
    /// Cloud provider API
    CloudProvider,
}

impl ExecutionTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Vm => "vm",
            Self::Docker => "docker",
            Self::Colab => "colab",
            Self::Browser => "browser",
            Self::Mcp => "mcp",
            Self::CloudProvider => "cloud_provider",
        }
    }

    /// Infer execution target from user text and tool name.
    /// Returns `Host` as the safe default.
    pub fn infer(user_text: &str, tool_name: &str) -> Self {
        let lower = user_text.to_ascii_lowercase();
        let tool_lower = tool_name.to_ascii_lowercase();

        // Explicit VM/SSH signals
        if lower.contains(" on my vm")
            || lower.contains(" on vm")
            || lower.contains(" in my vm")
            || lower.contains(" via ssh")
            || lower.starts_with("ssh ")
            || tool_lower == "execute_fleet_command"
            || tool_lower == "get_fleet_overview"
        {
            return Self::Vm;
        }

        // Docker signals
        if lower.contains("docker")
            || lower.contains("container")
            || tool_lower.starts_with("docker")
        {
            return Self::Docker;
        }

        // Colab signals
        if lower.contains("colab")
            || lower.contains("notebook")
            || tool_lower.starts_with("mcp_colab")
        {
            return Self::Colab;
        }

        // Browser signals
        if lower.contains("browser")
            || lower.contains("chrome")
            || lower.contains("firefox")
            || tool_lower == "browser_search"
            || tool_lower == "managed_browser_navigate"
            || tool_lower == "open_url"
        {
            return Self::Browser;
        }

        // MCP signals
        if tool_lower.starts_with("mcp_") {
            return Self::Mcp;
        }

        // Cloud provider signals
        if tool_lower.starts_with("gw_")
            || lower.contains("google workspace")
            || lower.contains("gmail")
            || lower.contains("google drive")
        {
            return Self::CloudProvider;
        }

        Self::Host
    }
}

// ─── Completed Action ─────────────────────────────────────────────────────────

/// A successfully completed tool action in the current turn.
#[derive(Debug, Clone)]
pub struct CompletedAction {
    pub tool_name: String,
    pub args_hash: u64,
    pub result_summary: String, // First 200 chars of result (for logging)
    pub result_data: serde_json::Value, // Full result data (for synthesis)
    pub target: ExecutionTarget,
    pub completed_at: Instant,
}

// ─── Turn Memory ──────────────────────────────────────────────────────────────

/// Per-turn operational memory.
/// Created fresh at turn start, dropped at turn end.
#[derive(Debug)]
pub struct TurnMemory {
    /// The user's original goal (first user message this turn).
    pub goal: String,
    /// Resolved execution target for this turn.
    pub primary_target: ExecutionTarget,
    /// Successfully completed actions (for satisfaction detection).
    completed_actions: Vec<CompletedAction>,
    /// Memoization cache: args_hash → result_summary.
    /// Prevents re-executing identical successful calls.
    memo_cache: HashMap<u64, String>,
    /// Set of tool names that have been called this turn (for duplicate detection).
    called_tools: HashSet<String>,
    /// Whether the turn goal appears to be satisfied.
    goal_satisfied: bool,
    /// Reason the goal was marked satisfied.
    satisfaction_reason: Option<String>,
    /// Active resources created this turn (file paths, IDs, etc.)
    pub active_resources: Vec<String>,
}

impl TurnMemory {
    pub fn new(goal: &str, primary_target: ExecutionTarget) -> Self {
        Self {
            goal: goal.to_string(),
            primary_target,
            completed_actions: Vec::new(),
            memo_cache: HashMap::new(),
            called_tools: HashSet::new(),
            goal_satisfied: false,
            satisfaction_reason: None,
            active_resources: Vec::new(),
        }
    }

    /// Record a successful tool execution.
    pub fn record_success(
        &mut self,
        tool_name: &str,
        args_hash: u64,
        result_summary: &str,
        target: ExecutionTarget,
    ) {
        self.record_success_with_data(
            tool_name,
            args_hash,
            result_summary,
            serde_json::Value::Null,
            target,
        );
    }

    /// Record a successful tool execution with full result data for synthesis.
    pub fn record_success_with_data(
        &mut self,
        tool_name: &str,
        args_hash: u64,
        result_summary: &str,
        result_data: serde_json::Value,
        target: ExecutionTarget,
    ) {
        self.called_tools.insert(tool_name.to_string());
        self.memo_cache
            .insert(args_hash, result_summary.to_string());
        self.completed_actions.push(CompletedAction {
            tool_name: tool_name.to_string(),
            args_hash,
            result_summary: result_summary.chars().take(200).collect(),
            result_data,
            target,
            completed_at: Instant::now(),
        });
    }

    /// Check if an identical successful call is already memoized.
    /// Returns the cached result summary if found.
    pub fn check_memo(&self, args_hash: u64) -> Option<&str> {
        self.memo_cache.get(&args_hash).map(|s| s.as_str())
    }

    /// Whether a tool has been called this turn (regardless of success/failure).
    pub fn was_called(&self, tool_name: &str) -> bool {
        self.called_tools.contains(tool_name)
    }

    /// Mark the turn goal as satisfied.
    pub fn mark_satisfied(&mut self, reason: impl Into<String>) {
        self.goal_satisfied = true;
        self.satisfaction_reason = Some(reason.into());
        tracing::info!(
            goal = %self.goal,
            reason = %self.satisfaction_reason.as_deref().unwrap_or(""),
            "TurnMemory: goal satisfied — tool loop can terminate"
        );
    }

    /// Whether the goal appears satisfied.
    pub fn is_satisfied(&self) -> bool {
        self.goal_satisfied
    }

    /// Satisfaction reason (for logging).
    pub fn satisfaction_reason(&self) -> Option<&str> {
        self.satisfaction_reason.as_deref()
    }

    /// Number of completed actions.
    pub fn completed_count(&self) -> usize {
        self.completed_actions.len()
    }

    /// Add an active resource (file path, ID, etc.) created this turn.
    pub fn add_resource(&mut self, resource: impl Into<String>) {
        self.active_resources.push(resource.into());
    }

    // ─── Search Dedup Guard ───────────────────────────────────────────────────

    /// Whether any search tool has been called this turn.
    /// Used to prevent redundant parallel search calls (web_search + search_news
    /// both firing for the same query).
    pub fn search_called_this_turn(&self) -> bool {
        self.called_tools.contains("web_search")
            || self.called_tools.contains("searxng_search")
            || self.called_tools.contains("search_news")
    }

    /// Whether a specific search tool has been called this turn.
    pub fn search_tool_called(&self, tool_name: &str) -> bool {
        self.called_tools.contains(tool_name)
    }

    /// Whether a successful search result is already available this turn.
    /// Returns the tool name that produced the result, if any.
    pub fn successful_search_this_turn(&self) -> Option<&str> {
        for action in &self.completed_actions {
            if matches!(
                action.tool_name.as_str(),
                "web_search" | "searxng_search" | "search_news"
            ) {
                return Some(&action.tool_name);
            }
        }
        None
    }

    /// Get completed actions for summary formatting.
    pub fn get_completed_actions(&self) -> &[CompletedAction] {
        &self.completed_actions
    }

    /// Snapshot for logging.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "goal_preview": &self.goal[..self.goal.len().min(100)],
            "primary_target": self.primary_target.as_str(),
            "completed_actions": self.completed_actions.len(),
            "memoized_calls": self.memo_cache.len(),
            "goal_satisfied": self.goal_satisfied,
            "satisfaction_reason": self.satisfaction_reason,
            "active_resources": self.active_resources.len(),
        })
    }
}

// ─── Task Satisfaction Detector ───────────────────────────────────────────────

/// Detect whether the user's goal has been satisfied based on completed actions.
///
/// This is a lightweight heuristic detector — it does NOT call the LLM.
/// It uses structural signals from the tool results and goal text.
pub fn detect_satisfaction(
    memory: &TurnMemory,
    tool_name: &str,
    result_success: bool,
) -> Option<String> {
    if !result_success {
        return None;
    }

    let goal_lower = memory.goal.to_ascii_lowercase();
    let tool_lower = tool_name.to_ascii_lowercase();

    // ── Single-tool satisfaction: tool matches the obvious goal ──
    let single_tool_goals = [
        // System info queries
        ("cpu", "get_cpu_usage"),
        ("memory", "get_memory_info"),
        ("disk", "get_disk_space"),
        ("battery", "get_battery_status"),
        ("uptime", "get_system_uptime"),
        ("gpu", "get_gpu_info"),
        ("network status", "get_network_status"),
        // Power actions
        ("lock screen", "lock_screen"),
        ("lock my screen", "lock_screen"),
        ("screenshot", "screenshot"),
        // Simple reads
        ("clipboard", "get_clipboard"),
        ("active window", "get_active_window"),
        ("list windows", "list_windows"),
        ("running apps", "list_running_apps"),
        ("alerts", "get_alerts"),
        ("power plan", "get_power_plan"),
        ("wifi networks", "get_wifi_networks"),
    ];

    for (goal_signal, expected_tool) in &single_tool_goals {
        if goal_lower.contains(goal_signal) && tool_lower == *expected_tool {
            return Some(format!("single_tool_goal_completed: {}", tool_name));
        }
    }

    // ── Multi-tool goals: check if all required tools have been called ──

    // System stats: CPU + memory + disk
    if (goal_lower.contains("system stat")
        || goal_lower.contains("system status")
        || goal_lower.contains("system vitals"))
        && memory.was_called("get_cpu_usage")
        && memory.was_called("get_memory_info")
        && memory.was_called("get_disk_space")
    {
        return Some("system_stats_complete: all three metrics gathered".into());
    }

    // Internet connectivity: at least one ping succeeded
    if (goal_lower.contains("internet")
        || goal_lower.contains("connected")
        || goal_lower.contains("online"))
        && tool_lower == "ping_host"
        && memory.completed_count() >= 1
    {
        return Some("connectivity_check_complete: ping succeeded".into());
    }

    // Search + save: if search succeeded and file was written
    if memory.was_called("web_search")
        || memory.was_called("search_news")
        || memory.was_called("searxng_search")
    {
        if memory.was_called("write_file") || memory.was_called("save_snippet") {
            return Some("search_and_save_complete".into());
        }
    }

    // ── Gmail goals: inbox/search satisfaction ──
    // "check inbox", "unread emails", "latest emails", "show emails" → done after inbox/search
    let gmail_intent_signals = [
        "inbox", "email", "emails", "mail", "gmail", "unread", "messages",
    ];
    let gmail_read_tools = ["gw_gmail_inbox", "gw_gmail_search", "gw_gmail_read"];
    let gmail_send_tools = ["gw_gmail_send", "gw_gmail_reply"];
    if gmail_intent_signals.iter().any(|s| goal_lower.contains(s))
        && gmail_read_tools.contains(&tool_lower.as_str())
        && !goal_lower.contains("send")
        && !goal_lower.contains("reply")
        && !goal_lower.contains("delete")
    {
        return Some(format!("gmail_read_complete: {}", tool_name));
    }
    if (goal_lower.contains("send") || goal_lower.contains("reply"))
        && gmail_send_tools.contains(&tool_lower.as_str())
    {
        return Some(format!("gmail_send_complete: {}", tool_name));
    }

    // ── Drive goals ──
    let drive_signals = ["drive", "files", "folders", "documents"];
    let drive_read_tools = ["gw_drive_list", "gw_drive_search", "gw_drive_read"];
    if drive_signals.iter().any(|s| goal_lower.contains(s))
        && drive_read_tools.contains(&tool_lower.as_str())
        && !goal_lower.contains("create")
        && !goal_lower.contains("delete")
        && !goal_lower.contains("upload")
    {
        return Some(format!("drive_read_complete: {}", tool_name));
    }

    // ── Filesystem goals: list/find files ──
    if (goal_lower.contains("list") || goal_lower.contains("find") || goal_lower.contains("show"))
        && (goal_lower.contains("file")
            || goal_lower.contains("folder")
            || goal_lower.contains("directory"))
        && matches!(
            tool_lower.as_str(),
            "list_directory"
                | "list_files"
                | "search_files"
                | "find_files_by_pattern"
                | "mcp_fs_search_files"
                | "mcp_fs_list_directory"
        )
    {
        return Some(format!("filesystem_read_complete: {}", tool_name));
    }

    // ── Webpage analysis ──
    if (goal_lower.contains("webpage")
        || goal_lower.contains("url")
        || goal_lower.contains("article")
        || goal_lower.contains("link")
        || goal_lower.contains("analyze")
        || goal_lower.contains("summarize"))
        && matches!(tool_lower.as_str(), "fetch_webpage" | "fetch_article")
    {
        return Some(format!("webpage_analysis_complete: {}", tool_name));
    }

    // ── GUI launch goals: opening apps / browsing to URLs ──
    // "open chrome and search youtube", "open firefox", "open gedit", etc.
    // Once the launch tool succeeds, the user-visible action is done; the LLM
    // should not invent additional retrieval calls (search_news, web_search, …).
    if matches!(
        tool_lower.as_str(),
        "browser_search" | "open_application" | "open_url" | "open_application_with_file"
    ) && (goal_lower.starts_with("open ")
        || goal_lower.starts_with("launch ")
        || goal_lower.starts_with("start ")
        || goal_lower.contains("open chrome")
        || goal_lower.contains("open firefox")
        || goal_lower.contains("open brave"))
    {
        return Some(format!("gui_launch_complete: {}", tool_name));
    }

    // ── Docker inspection ──
    // Match: "show docker containers", "list containers", "docker ps", etc.
    if (goal_lower.contains("docker") || goal_lower.contains("container"))
        && tool_lower == "execute_bash"
        && memory.completed_count() >= 1
    {
        return Some("docker_inspection_complete: execute_bash succeeded for docker query".into());
    }

    // ── Git inspection (read-only) ──
    if matches!(
        tool_lower.as_str(),
        "git_status" | "git_log" | "git_diff" | "git_branch_list" | "git_remote"
    ) && (goal_lower.contains("git")
        || goal_lower.contains("status")
        || goal_lower.contains("commit")
        || goal_lower.contains("branch"))
        && !goal_lower.contains("create")
        && !goal_lower.contains("push")
        && !goal_lower.contains("pull")
        && !goal_lower.contains("commit ")
    {
        return Some(format!("git_read_complete: {}", tool_name));
    }

    None
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_target_infers_vm_from_text() {
        assert_eq!(
            ExecutionTarget::infer("run this on my vm", "execute_bash"),
            ExecutionTarget::Vm
        );
        assert_eq!(
            ExecutionTarget::infer("ssh into server", "execute_bash"),
            ExecutionTarget::Vm
        );
    }

    #[test]
    fn execution_target_infers_colab_from_tool() {
        assert_eq!(
            ExecutionTarget::infer("run code", "mcp_colab-mcp_execute_cell"),
            ExecutionTarget::Colab
        );
    }

    #[test]
    fn execution_target_defaults_to_host() {
        assert_eq!(
            ExecutionTarget::infer("check my cpu", "get_cpu_usage"),
            ExecutionTarget::Host
        );
    }

    #[test]
    fn managed_browser_navigation_infers_browser_target() {
        assert_eq!(
            ExecutionTarget::infer(
                "Open the browser and go to https://outbro.net",
                "managed_browser_navigate",
            ),
            ExecutionTarget::Browser
        );
    }

    #[test]
    fn memo_cache_prevents_duplicate_calls() {
        let mut mem = TurnMemory::new("check cpu", ExecutionTarget::Host);
        let hash = 12345u64;

        assert!(mem.check_memo(hash).is_none());
        mem.record_success("get_cpu_usage", hash, "CPU: 45%", ExecutionTarget::Host);
        assert_eq!(mem.check_memo(hash), Some("CPU: 45%"));
    }

    #[test]
    fn satisfaction_detected_for_single_tool_goal() {
        let mut mem = TurnMemory::new("check my cpu usage", ExecutionTarget::Host);
        mem.record_success("get_cpu_usage", 1, "CPU: 45%", ExecutionTarget::Host);

        let reason = detect_satisfaction(&mem, "get_cpu_usage", true);
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("get_cpu_usage"));
    }

    #[test]
    fn satisfaction_detected_for_system_stats() {
        let mut mem = TurnMemory::new("show system stats", ExecutionTarget::Host);
        mem.record_success("get_cpu_usage", 1, "CPU: 45%", ExecutionTarget::Host);
        mem.record_success("get_memory_info", 2, "RAM: 8GB", ExecutionTarget::Host);
        mem.record_success("get_disk_space", 3, "Disk: 50%", ExecutionTarget::Host);

        let reason = detect_satisfaction(&mem, "get_disk_space", true);
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("system_stats_complete"));
    }

    #[test]
    fn no_satisfaction_on_failure() {
        let mem = TurnMemory::new("check cpu", ExecutionTarget::Host);
        let reason = detect_satisfaction(&mem, "get_cpu_usage", false);
        assert!(reason.is_none());
    }

    /// Regression: after a successful `browser_search`, the loop must mark
    /// the goal as satisfied so the LLM doesn't keep chaining `web_search`,
    /// `search_news`, etc. (the failure observed for "open chrome and search
    /// for youtube").
    #[test]
    fn satisfaction_detected_for_browser_search_after_open_chrome() {
        let mut mem = TurnMemory::new("Open chrome and search for youtube", ExecutionTarget::Host);
        mem.record_success(
            "browser_search",
            1,
            "{\"url\":\"https://www.youtube.com/\"}",
            ExecutionTarget::Host,
        );
        let reason = detect_satisfaction(&mem, "browser_search", true);
        assert!(reason.is_some(), "GUI launch should satisfy");
        assert!(reason.unwrap().contains("gui_launch_complete"));
    }

    #[test]
    fn satisfaction_detected_for_open_application_with_file() {
        let mut mem = TurnMemory::new("Open gedit and type a program", ExecutionTarget::Host);
        mem.record_success(
            "open_application_with_file",
            1,
            "{\"launched\":true}",
            ExecutionTarget::Host,
        );
        let reason = detect_satisfaction(&mem, "open_application_with_file", true);
        assert!(reason.is_some());
    }

    #[test]
    fn goal_satisfied_flag_stops_loop() {
        let mut mem = TurnMemory::new("lock screen", ExecutionTarget::Host);
        assert!(!mem.is_satisfied());
        mem.mark_satisfied("lock_screen completed");
        assert!(mem.is_satisfied());
    }

    #[test]
    fn was_called_tracks_tool_invocations() {
        let mut mem = TurnMemory::new("test", ExecutionTarget::Host);
        assert!(!mem.was_called("web_search"));
        mem.record_success("web_search", 1, "results", ExecutionTarget::Host);
        assert!(mem.was_called("web_search"));
    }

    #[test]
    fn search_dedup_guard_detects_any_search_tool() {
        let mut mem = TurnMemory::new("search for rust news", ExecutionTarget::Host);
        assert!(!mem.search_called_this_turn());
        assert!(mem.successful_search_this_turn().is_none());

        mem.record_success("web_search", 1, "results", ExecutionTarget::Host);
        assert!(mem.search_called_this_turn());
        assert_eq!(mem.successful_search_this_turn(), Some("web_search"));
    }

    #[test]
    fn search_dedup_guard_detects_searxng() {
        let mut mem = TurnMemory::new("search for news", ExecutionTarget::Host);
        mem.record_success("searxng_search", 1, "results", ExecutionTarget::Host);
        assert!(mem.search_called_this_turn());
        assert!(mem.search_tool_called("searxng_search"));
        assert!(!mem.search_tool_called("web_search"));
    }

    #[test]
    fn search_dedup_guard_detects_news_search() {
        let mut mem = TurnMemory::new("latest news", ExecutionTarget::Host);
        mem.record_success("search_news", 1, "articles", ExecutionTarget::Host);
        assert!(mem.search_called_this_turn());
        assert_eq!(mem.successful_search_this_turn(), Some("search_news"));
    }

    #[test]
    fn satisfaction_detects_gmail_inbox() {
        let mut mem = TurnMemory::new("show me unread emails", ExecutionTarget::CloudProvider);
        mem.record_success(
            "gw_gmail_inbox",
            1,
            "5 messages",
            ExecutionTarget::CloudProvider,
        );
        let reason = detect_satisfaction(&mem, "gw_gmail_inbox", true);
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("gmail_read_complete"));
    }

    #[test]
    fn satisfaction_detects_drive_list() {
        let mut mem = TurnMemory::new("list my drive folders", ExecutionTarget::CloudProvider);
        mem.record_success(
            "gw_drive_list",
            1,
            "folders",
            ExecutionTarget::CloudProvider,
        );
        let reason = detect_satisfaction(&mem, "gw_drive_list", true);
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("drive_read_complete"));
    }

    #[test]
    fn satisfaction_detects_filesystem_list() {
        let mut mem = TurnMemory::new("list txt files in Downloads", ExecutionTarget::Host);
        mem.record_success("list_directory", 1, "files", ExecutionTarget::Host);
        let reason = detect_satisfaction(&mem, "list_directory", true);
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("filesystem_read_complete"));
    }

    #[test]
    fn satisfaction_detects_webpage_analysis() {
        let mut mem = TurnMemory::new(
            "analyze this webpage https://example.com",
            ExecutionTarget::Host,
        );
        mem.record_success("fetch_webpage", 1, "content", ExecutionTarget::Host);
        let reason = detect_satisfaction(&mem, "fetch_webpage", true);
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("webpage_analysis_complete"));
    }

    #[test]
    fn satisfaction_detects_git_read() {
        let mut mem = TurnMemory::new("show git status", ExecutionTarget::Host);
        mem.record_success("git_status", 1, "clean", ExecutionTarget::Host);
        let reason = detect_satisfaction(&mem, "git_status", true);
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("git_read_complete"));
    }

    #[test]
    fn satisfaction_detects_docker_inspection() {
        let mut mem = TurnMemory::new(
            "Show all docker containers running on my host machine",
            ExecutionTarget::Host,
        );
        mem.record_success(
            "execute_bash",
            1,
            "CONTAINER ID IMAGE...",
            ExecutionTarget::Host,
        );
        let reason = detect_satisfaction(&mem, "execute_bash", true);
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("docker_inspection_complete"));
    }

    #[test]
    fn satisfaction_detects_container_list() {
        let mut mem = TurnMemory::new("list containers", ExecutionTarget::Host);
        mem.record_success("execute_bash", 1, "containers", ExecutionTarget::Host);
        let reason = detect_satisfaction(&mem, "execute_bash", true);
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("docker_inspection_complete"));
    }

    #[test]
    fn satisfaction_does_not_fire_when_send_intent_uses_read_tool() {
        // User asked to send an email but only the inbox was read — not satisfied
        let mut mem = TurnMemory::new("send email to alice", ExecutionTarget::CloudProvider);
        mem.record_success(
            "gw_gmail_inbox",
            1,
            "messages",
            ExecutionTarget::CloudProvider,
        );
        let reason = detect_satisfaction(&mem, "gw_gmail_inbox", true);
        assert!(
            reason.is_none(),
            "send intent must not be satisfied by inbox read"
        );
    }
}
