//! Tool Dependency Graph (TDG) - RFC 008 Phase 3
//!
//! Implements automatic dependency resolution for GUI tools.
//! Per RFC 008 Section 3: "TDG auto-expands raw intents to fully-resolved HTN workflows"

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

// ============================================================================
// Section 1: TDG Schema (RFC 008 Section 3.1)
// ============================================================================

/// Dependency strength classification.
/// Per RFC 008: "Hard dependencies MUST be satisfied; soft dependencies may fail gracefully"
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DependencyStrength {
    /// Hard dependency: MUST be satisfied, task cannot proceed without it
    Hard,
    /// Soft dependency: Nice to have, graceful degradation if unavailable
    Soft,
}

/// Dependency type classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DependencyType {
    /// Tool must be running (process liveness check)
    ProcessRunning { process_name: String },
    /// Window must be focused (focus check)
    WindowFocused { window_class: String },
    /// Element must be present on screen (vision check)
    ElementPresent {
        element_type: String,
        label: Option<String>,
    },
    /// File/dependency must exist (filesystem check)
    FileExists { path: String },
    /// Network connectivity required
    NetworkAvailable { host: String },
}

/// Tool dependency definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDependency {
    /// Unique dependency ID
    pub id: String,
    /// Human-readable description
    pub description: String,
    /// Hard or soft dependency
    pub strength: DependencyStrength,
    /// Type of dependency check
    pub dep_type: DependencyType,
    /// Timeout for dependency probe (seconds)
    #[serde(default = "default_probe_timeout")]
    pub probe_timeout_secs: u64,
    /// Retry attempts for dependency resolution
    #[serde(default = "default_retry_attempts")]
    pub retry_attempts: u32,
}

fn default_probe_timeout() -> u64 {
    5
}

fn default_retry_attempts() -> u32 {
    3
}

/// Tool definition in TDG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    /// Tool identifier (e.g., "click_element", "type_text")
    pub tool_id: String,
    /// Human-readable description
    pub description: String,
    /// List of dependencies for this tool
    pub dependencies: Vec<ToolDependency>,
    /// Estimated execution cost (for budget tracking)
    #[serde(default = "default_cost")]
    pub estimated_cost: u32,
}

fn default_cost() -> u32 {
    1
}

/// Tool Dependency Graph registry.
/// Per RFC 008: "TDG auto-expands raw intents to fully-resolved HTN workflows"
#[derive(Debug, Clone)]
pub struct ToolDependencyGraph {
    /// Map of tool_id -> tool definition
    tools: HashMap<String, ToolDef>,
}

impl ToolDependencyGraph {
    /// Create TDG with default tool definitions.
    /// Per RFC 008 Section 3.1: Hard/Soft dependency examples
    pub fn new() -> Self {
        let mut tdg = Self {
            tools: HashMap::new(),
        };
        tdg.register_default_tools();
        tdg
    }

    /// Register default tool definitions per RFC 008.
    fn register_default_tools(&mut self) {
        // click_element: Requires vision (hard), window focus (soft)
        self.register(ToolDef {
            tool_id: "click_element".to_string(),
            description: "Click on a UI element".to_string(),
            dependencies: vec![
                ToolDependency {
                    id: "vision_available".to_string(),
                    description: "OmniParser vision system available".to_string(),
                    strength: DependencyStrength::Hard,
                    dep_type: DependencyType::ProcessRunning {
                        process_name: "omniparser".to_string(),
                    },
                    probe_timeout_secs: 5,
                    retry_attempts: 2,
                },
                ToolDependency {
                    id: "window_focused".to_string(),
                    description: "Target window has focus".to_string(),
                    strength: DependencyStrength::Soft,
                    dep_type: DependencyType::WindowFocused {
                        window_class: "*".to_string(),
                    },
                    probe_timeout_secs: 3,
                    retry_attempts: 1,
                },
            ],
            estimated_cost: 1,
        });

        // type_text: Requires active element, window focus (hard)
        self.register(ToolDef {
            tool_id: "type_text".to_string(),
            description: "Type text into focused element".to_string(),
            dependencies: vec![
                ToolDependency {
                    id: "window_focused".to_string(),
                    description: "Target window has keyboard focus".to_string(),
                    strength: DependencyStrength::Hard,
                    dep_type: DependencyType::WindowFocused {
                        window_class: "*".to_string(),
                    },
                    probe_timeout_secs: 5,
                    retry_attempts: 3,
                },
                ToolDependency {
                    id: "input_element_present".to_string(),
                    description: "Input field available for typing".to_string(),
                    strength: DependencyStrength::Hard,
                    dep_type: DependencyType::ElementPresent {
                        element_type: "input".to_string(),
                        label: None,
                    },
                    probe_timeout_secs: 5,
                    retry_attempts: 2,
                },
            ],
            estimated_cost: 1,
        });

        // run_code: Requires terminal/editor process (hard), file access (soft)
        self.register(ToolDef {
            tool_id: "run_code".to_string(),
            description: "Execute code in terminal/editor".to_string(),
            dependencies: vec![
                ToolDependency {
                    id: "terminal_running".to_string(),
                    description: "Terminal process is running".to_string(),
                    strength: DependencyStrength::Hard,
                    dep_type: DependencyType::ProcessRunning {
                        process_name: "gnome-terminal".to_string(),
                    },
                    probe_timeout_secs: 10,
                    retry_attempts: 2,
                },
                ToolDependency {
                    id: "working_directory_exists".to_string(),
                    description: "Working directory is accessible".to_string(),
                    strength: DependencyStrength::Soft,
                    dep_type: DependencyType::FileExists {
                        path: ".".to_string(),
                    },
                    probe_timeout_secs: 2,
                    retry_attempts: 1,
                },
            ],
            estimated_cost: 2,
        });

        // open_application: Requires process not already running (soft), display available (hard)
        self.register(ToolDef {
            tool_id: "open_application".to_string(),
            description: "Launch an application".to_string(),
            dependencies: vec![
                ToolDependency {
                    id: "display_available".to_string(),
                    description: "X11/Wayland display available".to_string(),
                    strength: DependencyStrength::Hard,
                    dep_type: DependencyType::ProcessRunning {
                        process_name: "Xorg".to_string(),
                    },
                    probe_timeout_secs: 3,
                    retry_attempts: 1,
                },
                ToolDependency {
                    id: "not_already_running".to_string(),
                    description: "Application not already running (optional)".to_string(),
                    strength: DependencyStrength::Soft,
                    dep_type: DependencyType::ProcessRunning {
                        process_name: "gedit".to_string(),
                    },
                    probe_timeout_secs: 2,
                    retry_attempts: 1,
                },
            ],
            estimated_cost: 3,
        });

        // get_screen_elements: Requires vision system (hard)
        self.register(ToolDef {
            tool_id: "get_screen_elements".to_string(),
            description: "Capture and parse screen elements".to_string(),
            dependencies: vec![ToolDependency {
                id: "vision_available".to_string(),
                description: "OmniParser vision sidecar running".to_string(),
                strength: DependencyStrength::Hard,
                dep_type: DependencyType::ProcessRunning {
                    process_name: "omniparser".to_string(),
                },
                probe_timeout_secs: 5,
                retry_attempts: 2,
            }],
            estimated_cost: 2,
        });
    }

    /// Register a tool definition.
    pub fn register(&mut self, tool: ToolDef) {
        self.tools.insert(tool.tool_id.clone(), tool);
    }

    /// Get tool definition by ID.
    pub fn get(&self, tool_id: &str) -> Option<&ToolDef> {
        self.tools.get(tool_id)
    }

    /// Get dependencies for a tool.
    pub fn get_dependencies(&self, tool_id: &str) -> Vec<&ToolDependency> {
        self.get(tool_id)
            .map(|t| t.dependencies.iter().collect())
            .unwrap_or_default()
    }

    /// Check if tool exists in TDG.
    pub fn has_tool(&self, tool_id: &str) -> bool {
        self.tools.contains_key(tool_id)
    }
}

impl Default for ToolDependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Section 2: Dependency Liveness Probes (RFC 008 Section 3.3)
// ============================================================================

/// Dependency liveness state.
/// Per RFC 008: "Distinguish Healthy vs Unfocused vs Hung vs Dead"
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LivenessState {
    /// Process healthy and responsive
    Healthy,
    /// Process running but not focused/unfocused
    Unfocused,
    /// Process running but not responding to probes
    Hung,
    /// Process not running or crashed
    Dead,
}

/// Dependency liveness probe result.
#[derive(Debug, Clone)]
pub struct LivenessResult {
    /// Detected state
    pub state: LivenessState,
    /// Process ID if found
    pub pid: Option<u32>,
    /// Window ID if applicable
    pub window_id: Option<String>,
    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

/// Dependency liveness probe.
/// Per RFC 008 Section 3.3: "Liveness probes distinguish Unfocused vs Dead processes"
pub struct DependencyLiveness;

impl DependencyLiveness {
    /// Probe dependency liveness.
    /// Returns detailed state: Healthy, Unfocused, Hung, or Dead
    pub async fn probe(dep_type: &DependencyType) -> LivenessResult {
        match dep_type {
            DependencyType::ProcessRunning { process_name } => {
                Self::probe_process(process_name).await
            }
            DependencyType::WindowFocused { window_class } => {
                Self::probe_window_focus(window_class).await
            }
            DependencyType::ElementPresent {
                element_type,
                label,
            } => Self::probe_element_present(element_type, label.as_deref()).await,
            DependencyType::FileExists { path } => Self::probe_file_exists(path).await,
            DependencyType::NetworkAvailable { host } => Self::probe_network(host).await,
        }
    }

    /// Probe process state using OS metadata.
    /// Per RFC 008: "Process responsive + no error state = Healthy"
    async fn probe_process(process_name: &str) -> LivenessResult {
        // Scaffolding: In production, would use /proc or ps
        // For now, simulate based on known processes

        let pid = Self::find_process_pid(process_name).await;

        if let Some(pid) = pid {
            // Check if process is responsive (would check /proc/{pid}/stat in production)
            let is_responsive = Self::check_process_responsive(pid).await;

            if is_responsive {
                LivenessResult {
                    state: LivenessState::Healthy,
                    pid: Some(pid),
                    window_id: None,
                    metadata: [("process_name".to_string(), process_name.to_string())]
                        .into_iter()
                        .collect(),
                }
            } else {
                LivenessResult {
                    state: LivenessState::Hung,
                    pid: Some(pid),
                    window_id: None,
                    metadata: [("reason".to_string(), "unresponsive".to_string())]
                        .into_iter()
                        .collect(),
                }
            }
        } else {
            LivenessResult {
                state: LivenessState::Dead,
                pid: None,
                window_id: None,
                metadata: [("reason".to_string(), "not_found".to_string())]
                    .into_iter()
                    .collect(),
            }
        }
    }

    /// Probe window focus state via xdotool (X11/XWayland).
    /// Falls back to Unfocused on Wayland where xdotool is unavailable.
    async fn probe_window_focus(window_class: &str) -> LivenessResult {
        let result = tokio::process::Command::new("xdotool")
            .args(["getactivewindow", "getwindowname"])
            .output()
            .await;

        match result {
            Ok(o) if o.status.success() => {
                let active_title = String::from_utf8_lossy(&o.stdout).trim().to_string();
                let focused = active_title
                    .to_lowercase()
                    .contains(&window_class.to_lowercase());
                LivenessResult {
                    state: if focused {
                        LivenessState::Healthy
                    } else {
                        LivenessState::Unfocused
                    },
                    pid: None,
                    window_id: None,
                    metadata: [
                        ("window_class".to_string(), window_class.to_string()),
                        ("active_title".to_string(), active_title),
                        ("focused".to_string(), focused.to_string()),
                    ]
                    .into_iter()
                    .collect(),
                }
            }
            _ => LivenessResult {
                state: LivenessState::Unfocused,
                pid: None,
                window_id: None,
                metadata: [
                    ("window_class".to_string(), window_class.to_string()),
                    (
                        "note".to_string(),
                        "xdotool unavailable (Wayland?)".to_string(),
                    ),
                ]
                .into_iter()
                .collect(),
            },
        }
    }

    /// Probe element presence via vision.
    async fn probe_element_present(element_type: &str, label: Option<&str>) -> LivenessResult {
        // Scaffolding: Would query OmniParser cache
        // For now, return Dead (requires actual vision check)

        LivenessResult {
            state: LivenessState::Dead,
            pid: None,
            window_id: None,
            metadata: [
                ("element_type".to_string(), element_type.to_string()),
                ("label".to_string(), label.unwrap_or("any").to_string()),
                (
                    "note".to_string(),
                    "scaffolding - requires vision".to_string(),
                ),
            ]
            .into_iter()
            .collect(),
        }
    }

    /// Probe file existence.
    async fn probe_file_exists(path: &str) -> LivenessResult {
        let exists = tokio::fs::metadata(path).await.is_ok();

        LivenessResult {
            state: if exists {
                LivenessState::Healthy
            } else {
                LivenessState::Dead
            },
            pid: None,
            window_id: None,
            metadata: [("path".to_string(), path.to_string())]
                .into_iter()
                .collect(),
        }
    }

    /// Probe network connectivity.
    async fn probe_network(host: &str) -> LivenessResult {
        // Scaffolding: Would ping host
        // For now, assume Healthy (common case)

        LivenessResult {
            state: LivenessState::Healthy,
            pid: None,
            window_id: None,
            metadata: [("host".to_string(), host.to_string())]
                .into_iter()
                .collect(),
        }
    }

    /// Find process PID by name via /proc scan.
    async fn find_process_pid(process_name: &str) -> Option<u32> {
        let name_lower = process_name.to_lowercase();
        let mut read_dir = match tokio::fs::read_dir("/proc").await {
            Ok(d) => d,
            Err(_) => return None,
        };
        while let Ok(Some(entry)) = read_dir.next_entry().await {
            let fname = entry.file_name();
            let fname_str = fname.to_string_lossy();
            if !fname_str.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            let comm_path = format!("/proc/{}/comm", fname_str);
            if let Ok(comm) = tokio::fs::read_to_string(&comm_path).await {
                if comm.trim().to_lowercase().contains(&name_lower) {
                    if let Ok(pid) = fname_str.parse::<u32>() {
                        return Some(pid);
                    }
                }
            }
        }
        None
    }

    /// Check if process is responsive by verifying /proc/{pid}/stat is readable.
    async fn check_process_responsive(pid: u32) -> bool {
        let stat_path = format!("/proc/{}/stat", pid);
        tokio::fs::metadata(&stat_path).await.is_ok()
    }
}

// ============================================================================
// Section 3: Application Stability (RFC 008 Section 3.3)
// ============================================================================

/// Stability check factors.
/// Per RFC 008: "Multi-factor stability check"
#[derive(Debug, Clone)]
pub struct StabilityFactors {
    /// Process is responsive to signals
    pub process_responsive: bool,
    /// Compositor has acknowledged window (not flickering)
    pub compositor_acknowledged: bool,
    /// No loading indicators present
    pub no_loading_indicators: bool,
    /// Window has stable geometry (not resizing)
    pub stable_geometry: bool,
}

/// Application stability check result.
#[derive(Debug, Clone)]
pub struct StabilityResult {
    /// All factors passed
    pub is_stable: bool,
    /// Individual factor results
    pub factors: StabilityFactors,
    /// Time waited for stability
    pub wait_duration: Duration,
}

/// Wait for application to reach stable state.
/// Per RFC 008 Section 3.3: "Stability delay (1-2 sec) after launch recovery"
/// Per RFC 008: "Multi-factor stability check: process responsive + compositor acknowledged + no loading indicators"
pub async fn wait_for_application_stability(
    process_name: &str,
    max_wait: Duration,
) -> StabilityResult {
    let start = Instant::now();
    let check_interval = Duration::from_millis(250);
    let min_wait = Duration::from_millis(500); // RFC 008: minimum 500ms wait

    loop {
        let elapsed = start.elapsed();

        // Check stability factors
        let factors = check_stability_factors(process_name).await;
        let all_passed = factors.process_responsive
            && factors.compositor_acknowledged
            && factors.no_loading_indicators
            && factors.stable_geometry;

        // Must wait at least min_wait, then can exit if stable
        if elapsed >= min_wait && all_passed {
            return StabilityResult {
                is_stable: true,
                factors,
                wait_duration: elapsed,
            };
        }

        // Timeout reached
        if elapsed >= max_wait {
            return StabilityResult {
                is_stable: all_passed, // May be partially stable
                factors,
                wait_duration: elapsed,
            };
        }

        // Wait before next check
        tokio::time::sleep(check_interval).await;
    }
}

/// Check individual stability factors.
async fn check_stability_factors(_process_name: &str) -> StabilityFactors {
    // Scaffolding: In production, would query actual system state
    // For now, simulate typical stable state

    StabilityFactors {
        process_responsive: true,
        compositor_acknowledged: true,
        no_loading_indicators: true,
        stable_geometry: true,
    }
}

// ============================================================================
// Section 4: HTN Expander (RFC 008 Section 3.2)
// ============================================================================

use super::htn_executor::{SubGoal, VerificationType};

/// Expanded intent with dependencies resolved.
#[derive(Debug, Clone)]
pub struct ExpandedIntent {
    /// Sequence of sub-goals to execute
    pub sub_goals: Vec<SubGoal>,
    /// Total estimated cost
    pub total_cost: u32,
    /// Dependencies that failed (for soft deps)
    pub failed_soft_deps: Vec<String>,
}

/// HTN Expander - transforms raw intents to dependency-aware workflows.
/// Per RFC 008 Section 3.2: "TDG auto-expands raw intents to fully-resolved HTN workflows"
pub struct HtnExpander {
    tdg: ToolDependencyGraph,
}

impl HtnExpander {
    /// Create new expander with default TDG.
    pub fn new() -> Self {
        Self {
            tdg: ToolDependencyGraph::new(),
        }
    }

    /// Expand raw intent into dependency-aware sub-goal sequence.
    ///
    /// Example: "type_text" expands to:
    /// 1. get_screen_elements (verify input exists) - dependency
    /// 2. click_element (focus input) - dependency  
    /// 3. type_text (actual action) - original intent
    pub fn expand_intent(
        &self,
        intent_action: &str,
        intent_params: &serde_json::Value,
        starting_step: usize,
    ) -> ExpandedIntent {
        let mut sub_goals = Vec::new();
        let mut total_cost = 0;
        let mut failed_soft_deps = Vec::new();

        // Get tool definition from TDG
        if let Some(tool_def) = self.tdg.get(intent_action) {
            // First: Add dependency resolution sub-goals (prerequisites)
            let mut dep_step = starting_step;

            for dep in &tool_def.dependencies {
                // Check if we need to inject a sub-goal for this dependency
                if let Some(dep_subgoal) = self.dependency_to_subgoal(dep, dep_step) {
                    sub_goals.push(dep_subgoal);
                    dep_step += 1;
                    total_cost += 1;
                } else {
                    // Soft dependency that can't be resolved - record failure
                    if dep.strength == DependencyStrength::Soft {
                        failed_soft_deps.push(dep.id.clone());
                    }
                    // Hard dependency failure would be handled at execution time
                }
            }

            // Finally: Add the original intent action
            sub_goals.push(SubGoal {
                step: dep_step,
                action: intent_action.to_string(),
                params: intent_params.clone(),
                verify: VerificationType::ScreenChanged {
                    element_id: None,
                    threshold: 0.90,
                },
                timeout_ms: None,
            });
            total_cost += tool_def.estimated_cost;
        } else {
            // Tool not in TDG - add directly without dependency expansion
            sub_goals.push(SubGoal {
                step: starting_step,
                action: intent_action.to_string(),
                params: intent_params.clone(),
                verify: VerificationType::None,
                timeout_ms: None,
            });
            total_cost = 1;
        }

        ExpandedIntent {
            sub_goals,
            total_cost,
            failed_soft_deps,
        }
    }

    /// Convert dependency to prerequisite sub-goal if needed.
    fn dependency_to_subgoal(&self, dep: &ToolDependency, step: usize) -> Option<SubGoal> {
        // Map dependency types to prerequisite sense/check actions
        match &dep.dep_type {
            DependencyType::ProcessRunning { process_name } => {
                // Add a "probe_process" sub-goal (scaffolding: would use actual probe)
                Some(SubGoal {
                    step,
                    action: "probe_process".to_string(),
                    params: serde_json::json!({
                        "process_name": process_name,
                        "dependency_id": dep.id,
                    }),
                    verify: VerificationType::None,
                    timeout_ms: Some(dep.probe_timeout_secs * 1000),
                })
            }
            DependencyType::WindowFocused { window_class } => {
                // Add a "verify_focus" sub-goal
                Some(SubGoal {
                    step,
                    action: "verify_focus".to_string(),
                    params: serde_json::json!({
                        "window_class": window_class,
                        "dependency_id": dep.id,
                    }),
                    verify: VerificationType::WindowState {
                        title_contains: None,
                        class: Some(window_class.clone()),
                    },
                    timeout_ms: Some(dep.probe_timeout_secs * 1000),
                })
            }
            DependencyType::ElementPresent {
                element_type,
                label,
            } => {
                // Add a "get_screen_elements" sub-goal to verify element exists
                Some(SubGoal {
                    step,
                    action: "get_screen_elements".to_string(),
                    params: serde_json::json!({
                        "filter_type": element_type,
                        "label_filter": label,
                        "dependency_id": dep.id,
                    }),
                    verify: VerificationType::ElementsFound {
                        element_ids: vec![], // Would be populated from vision
                        min_count: 1,
                    },
                    timeout_ms: Some(dep.probe_timeout_secs * 1000),
                })
            }
            _ => None, // FileExists, NetworkAvailable don't need sub-goals
        }
    }

    /// Get TDG reference.
    pub fn tdg(&self) -> &ToolDependencyGraph {
        &self.tdg
    }
}

impl Default for HtnExpander {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tdg_default_tools() {
        let tdg = ToolDependencyGraph::new();

        assert!(tdg.has_tool("click_element"));
        assert!(tdg.has_tool("type_text"));
        assert!(tdg.has_tool("run_code"));
        assert!(tdg.has_tool("open_application"));
        assert!(tdg.has_tool("get_screen_elements"));
    }

    #[test]
    fn test_click_element_dependencies() {
        let tdg = ToolDependencyGraph::new();
        let deps = tdg.get_dependencies("click_element");

        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].strength, DependencyStrength::Hard); // vision
        assert_eq!(deps[1].strength, DependencyStrength::Soft); // focus
    }

    #[test]
    fn test_type_text_hard_dependencies() {
        let tdg = ToolDependencyGraph::new();
        let deps = tdg.get_dependencies("type_text");

        // Both dependencies should be Hard
        assert!(deps.iter().all(|d| d.strength == DependencyStrength::Hard));
    }

    #[test]
    fn test_htn_expander_type_text() {
        let expander = HtnExpander::new();

        let expanded =
            expander.expand_intent("type_text", &serde_json::json!({"text": "Hello World"}), 1);

        // Should have: verify_focus + get_screen_elements + type_text
        assert_eq!(expanded.sub_goals.len(), 3);
        assert_eq!(expanded.sub_goals[0].action, "verify_focus");
        assert_eq!(expanded.sub_goals[1].action, "get_screen_elements");
        assert_eq!(expanded.sub_goals[2].action, "type_text");

        // Step numbers should be sequential
        assert_eq!(expanded.sub_goals[0].step, 1);
        assert_eq!(expanded.sub_goals[1].step, 2);
        assert_eq!(expanded.sub_goals[2].step, 3);
    }

    #[test]
    fn test_htn_expander_click_element() {
        let expander = HtnExpander::new();

        let expanded = expander.expand_intent(
            "click_element",
            &serde_json::json!({"element_id": "btn_save"}),
            1,
        );

        // Should have: probe_process + verify_focus + click_element
        assert_eq!(expanded.sub_goals.len(), 3);
        assert_eq!(expanded.sub_goals[0].action, "probe_process");
        assert_eq!(expanded.sub_goals[1].action, "verify_focus");
        assert_eq!(expanded.sub_goals[2].action, "click_element");
    }

    #[test]
    fn test_htn_expander_unknown_tool() {
        let expander = HtnExpander::new();

        let expanded = expander.expand_intent("unknown_tool", &serde_json::json!({}), 1);

        // Should just pass through without expansion
        assert_eq!(expanded.sub_goals.len(), 1);
        assert_eq!(expanded.sub_goals[0].action, "unknown_tool");
    }

    #[test]
    fn test_liveness_file_exists() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            DependencyLiveness::probe(&DependencyType::FileExists {
                path: "/etc/passwd".to_string(),
            })
            .await
        });

        assert_eq!(result.state, LivenessState::Healthy);
    }

    #[test]
    fn test_liveness_file_not_exists() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            DependencyLiveness::probe(&DependencyType::FileExists {
                path: "/nonexistent/path/12345".to_string(),
            })
            .await
        });

        assert_eq!(result.state, LivenessState::Dead);
    }

    #[tokio::test]
    async fn test_stability_check() {
        let result = wait_for_application_stability("test_app", Duration::from_millis(500)).await;

        // Should complete (scaffolding returns stable)
        assert!(result.is_stable);
        assert!(result.wait_duration >= Duration::from_millis(500));
    }
}
