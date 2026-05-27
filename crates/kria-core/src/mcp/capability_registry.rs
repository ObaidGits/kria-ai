//! MCP Capability Registry — structured capability metadata for all tools.
//!
//! Extends the basic `ToolDef` with rich capability metadata that enables:
//! - Semantic capability planning (reason about capabilities, not raw tool names)
//! - GUI-last policy enforcement (prefer API → MCP → CLI → GUI)
//! - Execution target validation
//! - Provider-aware routing
//! - Reliability and latency-aware selection
//!
//! # Design
//! - Zero LLM calls — all metadata is static or derived from tool names/descriptions
//! - Deterministic: same tool → same capability profile always
//! - Observable: all capability decisions are logged
//! - Additive: does not replace ToolDef, only augments it

use serde::Serialize;

// ─── Capability Tags ──────────────────────────────────────────────────────────

/// Semantic capability tags for a tool.
/// Used by the planner to reason about what a tool CAN do, not just its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityTag {
    // Communication
    EmailRead,
    EmailSend,
    EmailSearch,
    EmailDelete,
    EmailReply,
    EmailDraft,
    // Storage
    FileRead,
    FileWrite,
    FileDelete,
    FileSearch,
    FileUpload,
    FileDownload,
    FolderCreate,
    FolderList,
    // Calendar
    CalendarRead,
    CalendarCreate,
    CalendarDelete,
    // Code execution
    CodeExecute,
    ShellExecute,
    NotebookExecute,
    // Search & retrieval
    WebSearch,
    NewsSearch,
    KnowledgeSearch,
    // System
    SystemInfo,
    SystemControl,
    ProcessManagement,
    // Browser / GUI
    BrowserOpen,
    BrowserSearch,
    GuiAutomation,
    ScreenCapture,
    // Network
    NetworkRequest,
    NetworkDiagnostic,
    // Developer
    GitOperation,
    CodeAnalysis,
    DatabaseQuery,
    // AI / generation
    ImageGenerate,
    ImageAnalyze,
    TextGenerate,
    // Notifications
    NotificationSend,
    ReminderCreate,
    // Memory
    MemoryStore,
    MemoryRecall,
}

// ─── Execution Mode ───────────────────────────────────────────────────────────

/// How a tool executes — used for GUI-last policy enforcement.
/// Lower ordinal = preferred over higher ordinal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Direct API call (REST, gRPC) — most reliable, fastest
    Api = 0,
    /// Local computation — no external calls
    Local = 1,
    /// MCP protocol call — structured, reliable
    Mcp = 2,
    /// CLI/shell command — reliable but less structured
    Cli = 3,
    /// SSH/remote execution — reliable for remote targets
    Ssh = 4,
    /// Browser automation (Playwright, Selenium) — fragile, slow
    BrowserAutomation = 5,
    /// GUI automation (xdotool, ydotool) — LAST RESORT, most fragile
    GuiAutomation = 6,
}

// ─── Reliability Profile ──────────────────────────────────────────────────────

/// Reliability and performance profile for a tool.
#[derive(Debug, Clone, Serialize)]
pub struct ReliabilityProfile {
    /// Expected latency bucket
    pub latency: LatencyBucket,
    /// Whether this tool is idempotent (safe to retry)
    pub idempotent: bool,
    /// Whether this tool has side effects
    pub has_side_effects: bool,
    /// Whether this tool requires network access
    pub requires_network: bool,
    /// Whether this tool requires authentication
    pub requires_auth: bool,
    /// Estimated reliability (0.0–1.0, based on tool type)
    pub reliability_score: f32,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LatencyBucket {
    /// < 100ms (local computation, memory lookup)
    Instant,
    /// 100ms–1s (local file ops, fast API)
    Fast,
    /// 1s–5s (network API, MCP call)
    Medium,
    /// 5s–30s (LLM call, complex operation)
    Slow,
    /// > 30s (image generation, long-running task)
    VerySlow,
}

// ─── Capability Profile ───────────────────────────────────────────────────────

/// Full capability profile for a tool.
/// Augments `ToolDef` with semantic metadata for intelligent routing.
#[derive(Debug, Clone, Serialize)]
pub struct CapabilityProfile {
    /// Tool name (matches ToolDef.name)
    pub tool_name: String,
    /// Semantic capability tags
    pub tags: Vec<CapabilityTag>,
    /// How this tool executes
    pub execution_mode: ExecutionMode,
    /// Reliability and performance profile
    pub reliability: ReliabilityProfile,
    /// Whether GUI automation should be tried before this tool
    /// (false = prefer this tool over GUI)
    pub prefer_over_gui: bool,
    /// Alternative tools that can accomplish the same goal (fallback chain)
    pub alternatives: &'static [&'static str],
    /// Human-readable capability summary
    pub capability_summary: &'static str,
}

impl CapabilityProfile {
    /// Whether this tool should be preferred over GUI automation.
    pub fn is_gui_preferred(&self) -> bool {
        self.execution_mode == ExecutionMode::GuiAutomation
    }

    /// Whether this tool is a last-resort option.
    pub fn is_last_resort(&self) -> bool {
        matches!(
            self.execution_mode,
            ExecutionMode::GuiAutomation | ExecutionMode::BrowserAutomation
        )
    }
}

// ─── Capability Registry ──────────────────────────────────────────────────────

/// Look up the capability profile for a tool by name.
/// Returns a generic profile for unknown tools.
pub fn capability_profile(tool_name: &str) -> CapabilityProfile {
    let lower = tool_name.to_ascii_lowercase();

    // ── Gmail tools ───────────────────────────────────────────────────────────
    if lower == "gw_gmail_inbox" || lower == "gw_gmail_search" {
        return CapabilityProfile {
            tool_name: tool_name.to_string(),
            tags: vec![CapabilityTag::EmailRead, CapabilityTag::EmailSearch],
            execution_mode: ExecutionMode::Api,
            reliability: ReliabilityProfile {
                latency: LatencyBucket::Medium,
                idempotent: true,
                has_side_effects: false,
                requires_network: true,
                requires_auth: true,
                reliability_score: 0.92,
            },
            prefer_over_gui: true,
            alternatives: &["gw_gmail_read"],
            capability_summary: "Read and search Gmail inbox via Google Workspace API",
        };
    }
    if lower == "gw_gmail_send" {
        return CapabilityProfile {
            tool_name: tool_name.to_string(),
            tags: vec![CapabilityTag::EmailSend],
            execution_mode: ExecutionMode::Api,
            reliability: ReliabilityProfile {
                latency: LatencyBucket::Medium,
                idempotent: false,
                has_side_effects: true,
                requires_network: true,
                requires_auth: true,
                reliability_score: 0.90,
            },
            prefer_over_gui: true,
            alternatives: &["gw_gmail_reply"],
            capability_summary: "Send email via Gmail API (draft-then-send workflow)",
        };
    }
    if lower == "gw_gmail_reply" {
        return CapabilityProfile {
            tool_name: tool_name.to_string(),
            tags: vec![CapabilityTag::EmailReply, CapabilityTag::EmailSend],
            execution_mode: ExecutionMode::Api,
            reliability: ReliabilityProfile {
                latency: LatencyBucket::Medium,
                idempotent: false,
                has_side_effects: true,
                requires_network: true,
                requires_auth: true,
                reliability_score: 0.90,
            },
            prefer_over_gui: true,
            alternatives: &["gw_gmail_send"],
            capability_summary: "Reply to a Gmail thread via Google Workspace API",
        };
    }

    // ── Drive tools ───────────────────────────────────────────────────────────
    if lower.starts_with("gw_drive_") {
        let (tags, has_side_effects, idempotent) = match lower.as_str() {
            "gw_drive_search" | "gw_drive_list" | "gw_drive_read" => (
                vec![CapabilityTag::FileRead, CapabilityTag::FileSearch],
                false,
                true,
            ),
            "gw_drive_create_file" | "gw_drive_create_folder" => (
                vec![CapabilityTag::FileWrite, CapabilityTag::FolderCreate],
                true,
                false,
            ),
            "gw_drive_upload" => (vec![CapabilityTag::FileUpload], true, false),
            "gw_drive_move" | "gw_drive_rename" => (vec![CapabilityTag::FileWrite], true, false),
            "gw_drive_delete" => (vec![CapabilityTag::FileDelete], true, false),
            _ => (vec![CapabilityTag::FileRead], false, true),
        };
        return CapabilityProfile {
            tool_name: tool_name.to_string(),
            tags,
            execution_mode: ExecutionMode::Api,
            reliability: ReliabilityProfile {
                latency: LatencyBucket::Medium,
                idempotent,
                has_side_effects,
                requires_network: true,
                requires_auth: true,
                reliability_score: 0.91,
            },
            prefer_over_gui: true,
            alternatives: &[],
            capability_summary: "Google Drive file operations via Google Workspace API",
        };
    }

    // ── Git tools ─────────────────────────────────────────────────────────────
    if lower.starts_with("git_") {
        let (tags, has_side_effects, idempotent) = match lower.as_str() {
            "git_status" | "git_log" | "git_diff" | "git_branch_list" => (
                vec![CapabilityTag::GitOperation, CapabilityTag::FileRead],
                false,
                true,
            ),
            "git_commit" | "git_push" | "git_merge" | "git_rebase" => {
                (vec![CapabilityTag::GitOperation], true, false)
            }
            "git_pull" | "git_fetch" => (vec![CapabilityTag::GitOperation], true, true),
            "git_checkout" | "git_stash" => (vec![CapabilityTag::GitOperation], true, false),
            _ => (vec![CapabilityTag::GitOperation], false, true),
        };
        return CapabilityProfile {
            tool_name: tool_name.to_string(),
            tags,
            execution_mode: ExecutionMode::Cli,
            reliability: ReliabilityProfile {
                latency: LatencyBucket::Fast,
                idempotent,
                has_side_effects,
                requires_network: matches!(lower.as_str(), "git_push" | "git_pull" | "git_fetch"),
                requires_auth: false,
                reliability_score: 0.95,
            },
            prefer_over_gui: true,
            alternatives: &[],
            capability_summary: "Git version control operations via CLI",
        };
    }

    // ── Web search ────────────────────────────────────────────────────────────
    if matches!(lower.as_str(), "web_search" | "searxng_search") {
        return CapabilityProfile {
            tool_name: tool_name.to_string(),
            tags: vec![CapabilityTag::WebSearch],
            execution_mode: ExecutionMode::Api,
            reliability: ReliabilityProfile {
                latency: LatencyBucket::Medium,
                idempotent: true,
                has_side_effects: false,
                requires_network: true,
                requires_auth: false,
                reliability_score: 0.85,
            },
            prefer_over_gui: true,
            alternatives: &["search_news", "fetch_webpage"],
            capability_summary: "Web search via DuckDuckGo or SearxNG",
        };
    }

    // ── Browser / GUI automation ──────────────────────────────────────────────
    // browser_search and open_url are OS-level URL dispatch tools (xdg-open/gio open).
    // They are NOT browser automation — they open URLs in the system default handler.
    // They should NOT be classified as last-resort; they are the CORRECT tool when
    // the user explicitly asks to open a browser or navigate to a URL.
    if matches!(
        lower.as_str(),
        "browser_search" | "open_url" | "open_application"
    ) {
        return CapabilityProfile {
            tool_name: tool_name.to_string(),
            tags: vec![CapabilityTag::BrowserOpen, CapabilityTag::BrowserSearch],
            execution_mode: ExecutionMode::Local, // OS dispatch — not browser automation
            reliability: ReliabilityProfile {
                latency: LatencyBucket::Fast,
                idempotent: false,
                has_side_effects: true,
                requires_network: false, // xdg-open itself is local; the browser handles network
                requires_auth: false,
                reliability_score: 0.90,
            },
            prefer_over_gui: true,
            alternatives: &[], // No alternatives — this IS the correct tool for GUI-launch requests
            capability_summary: "Open URL or search in system default browser via OS dispatch (xdg-open/gio open). Use for explicit browser-open requests.",
        };
    }

    // GUI automation tools — always last resort
    if lower.starts_with("gui_") || lower == "type_text" || lower == "click_element" {
        return CapabilityProfile {
            tool_name: tool_name.to_string(),
            tags: vec![CapabilityTag::GuiAutomation],
            execution_mode: ExecutionMode::GuiAutomation,
            reliability: ReliabilityProfile {
                latency: LatencyBucket::Slow,
                idempotent: false,
                has_side_effects: true,
                requires_network: false,
                requires_auth: false,
                reliability_score: 0.60,
            },
            prefer_over_gui: false,
            alternatives: &[],
            capability_summary: "GUI automation — LAST RESORT. Prefer API/MCP/CLI alternatives",
        };
    }

    // ── MCP tools ─────────────────────────────────────────────────────────────
    if lower.starts_with("mcp_") {
        let is_colab = lower.contains("colab");
        return CapabilityProfile {
            tool_name: tool_name.to_string(),
            tags: if is_colab {
                vec![CapabilityTag::NotebookExecute, CapabilityTag::CodeExecute]
            } else {
                vec![]
            },
            execution_mode: ExecutionMode::Mcp,
            reliability: ReliabilityProfile {
                latency: LatencyBucket::Medium,
                idempotent: false,
                has_side_effects: true,
                requires_network: true,
                requires_auth: false,
                reliability_score: 0.80,
            },
            prefer_over_gui: true,
            alternatives: &[],
            capability_summary: "MCP protocol tool — structured external capability",
        };
    }

    // ── Shell execution ───────────────────────────────────────────────────────
    if matches!(
        lower.as_str(),
        "execute_bash" | "execute_python" | "execute_powershell"
    ) {
        return CapabilityProfile {
            tool_name: tool_name.to_string(),
            tags: vec![CapabilityTag::ShellExecute, CapabilityTag::CodeExecute],
            execution_mode: ExecutionMode::Cli,
            reliability: ReliabilityProfile {
                latency: LatencyBucket::Fast,
                idempotent: false,
                has_side_effects: true,
                requires_network: false,
                requires_auth: false,
                reliability_score: 0.90,
            },
            prefer_over_gui: true,
            alternatives: &[],
            capability_summary: "Shell/script execution on host or VM",
        };
    }

    // ── Default: generic profile ──────────────────────────────────────────────
    CapabilityProfile {
        tool_name: tool_name.to_string(),
        tags: vec![],
        execution_mode: ExecutionMode::Local,
        reliability: ReliabilityProfile {
            latency: LatencyBucket::Fast,
            idempotent: true,
            has_side_effects: false,
            requires_network: false,
            requires_auth: false,
            reliability_score: 0.85,
        },
        prefer_over_gui: true,
        alternatives: &[],
        capability_summary: "General tool",
    }
}

// ─── GUI-Last Policy ──────────────────────────────────────────────────────────

/// Check if a tool should be skipped in favor of a better alternative.
///
/// Implements the GUI-last policy:
/// API → MCP → CLI → SSH → Browser automation → GUI automation
///
/// Returns `Some(alternative_tool_name)` if a better alternative is available
/// and registered, `None` if this tool is the best available option.
pub fn find_better_alternative<'a>(
    tool_name: &str,
    available_tools: &'a std::collections::HashSet<String>,
) -> Option<&'a str> {
    let profile = capability_profile(tool_name);

    // Only suggest alternatives for last-resort tools
    if !profile.is_last_resort() {
        return None;
    }

    // Find the best available alternative
    for alt in profile.alternatives {
        if available_tools.contains(*alt) {
            let alt_profile = capability_profile(alt);
            if alt_profile.execution_mode < profile.execution_mode {
                return available_tools.get(*alt).map(|s| s.as_str());
            }
        }
    }

    None
}

/// Build a capability summary for a set of tools.
/// Used for system prompt injection and observability.
pub fn build_capability_summary(tool_names: &[String]) -> serde_json::Value {
    let mut by_mode: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let mut gui_tools: Vec<String> = Vec::new();
    let mut last_resort_count = 0usize;

    for name in tool_names {
        let profile = capability_profile(name);
        let mode_key = format!("{:?}", profile.execution_mode).to_lowercase();
        by_mode.entry(mode_key).or_default().push(name.clone());
        if profile.is_last_resort() {
            gui_tools.push(name.clone());
            last_resort_count += 1;
        }
    }

    serde_json::json!({
        "total_tools": tool_names.len(),
        "by_execution_mode": by_mode,
        "gui_last_resort_tools": gui_tools,
        "last_resort_count": last_resort_count,
        "policy": "API > MCP > CLI > SSH > Browser > GUI (last resort)",
    })
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gmail_send_is_api_not_gui() {
        let profile = capability_profile("gw_gmail_send");
        assert_eq!(profile.execution_mode, ExecutionMode::Api);
        assert!(profile.prefer_over_gui);
        assert!(!profile.is_last_resort());
    }

    #[test]
    fn gui_automation_is_last_resort() {
        let profile = capability_profile("gui_click");
        assert!(profile.is_last_resort());
        assert_eq!(profile.execution_mode, ExecutionMode::GuiAutomation);
    }

    #[test]
    fn browser_search_is_os_dispatch_not_browser_automation() {
        let browser = capability_profile("browser_search");
        assert_eq!(browser.execution_mode, ExecutionMode::Local);
        assert!(!browser.is_last_resort());
        assert!(browser.alternatives.is_empty());
    }

    #[test]
    fn git_cli_tools_are_preferred_over_gui() {
        let profile = capability_profile("git_commit");
        assert_eq!(profile.execution_mode, ExecutionMode::Cli);
        assert!(profile.prefer_over_gui);
    }

    #[test]
    fn mcp_colab_has_notebook_execute_tag() {
        let profile = capability_profile("mcp_colab-mcp_execute_cell");
        assert!(profile.tags.contains(&CapabilityTag::NotebookExecute));
        assert_eq!(profile.execution_mode, ExecutionMode::Mcp);
    }

    #[test]
    fn drive_read_is_idempotent() {
        let profile = capability_profile("gw_drive_read");
        assert!(profile.reliability.idempotent);
        assert!(!profile.reliability.has_side_effects);
    }

    #[test]
    fn drive_delete_has_side_effects() {
        let profile = capability_profile("gw_drive_delete");
        assert!(profile.reliability.has_side_effects);
    }

    #[test]
    fn capability_summary_counts_gui_tools() {
        let tools = vec![
            "web_search".to_string(),
            "gw_gmail_send".to_string(),
            "gui_click".to_string(),
            "browser_search".to_string(),
        ];
        let summary = build_capability_summary(&tools);
        // Only gui_click is last-resort now; browser_search is OS dispatch (Local)
        assert_eq!(summary["last_resort_count"].as_u64().unwrap(), 1);
    }
}
