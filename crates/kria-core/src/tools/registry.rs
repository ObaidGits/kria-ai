use crate::infra::environment::{
    EnvironmentProvider, LocalEnvironment, SharedShellState, ShellState,
};
use crate::infra::ToolResult;
use crate::safety::RiskLevel;
use crate::tools::ToolContext;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

/// Tool parameter schema for LLM function-calling format.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ParamDef {
    pub name: String,
    #[serde(rename = "type")]
    pub param_type: String,
    pub description: String,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
}

/// Full tool definition including name, description, parameter schema, and tier.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub category: String,
    pub parameters: Vec<ParamDef>,
    pub default_tier: RiskLevel,
    /// Minimum hardware tier ("lite" tools available on all hardware).
    pub min_tier: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ToolResumeCapability {
    DeterministicLocal,
    RequiresLiveGui,
    ExternalDelegated,
    Unsupported,
}

impl ToolDef {
    /// Convert to OpenAI-compatible function schema for LLM.
    pub fn to_function_schema(&self) -> serde_json::Value {
        let mut props = serde_json::Map::new();
        let mut required = Vec::new();

        for p in &self.parameters {
            props.insert(
                p.name.clone(),
                serde_json::json!({
                    "type": p.param_type,
                    "description": p.description,
                }),
            );
            if p.required {
                required.push(serde_json::Value::String(p.name.clone()));
            }
        }

        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": {
                    "type": "object",
                    "properties": props,
                    "required": required,
                }
            }
        })
    }
}

/// Trait for tool execution handlers.
#[async_trait]
pub trait ToolHandler: Send + Sync {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let _ = params;
        ToolResult::err("tool does not implement execute")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> ToolResult {
        self.execute(params).await
    }
}

/// Central tool registry. Holds all tool definitions and their handlers.
/// Thread-safe for dynamic registration (e.g. MCP servers connecting in background).
pub struct ToolRegistry {
    defs: RwLock<HashMap<String, ToolDef>>,
    handlers: RwLock<HashMap<String, Arc<dyn ToolHandler>>>,
    resume_capabilities: RwLock<HashMap<String, ToolResumeCapability>>,
    env_provider: RwLock<Arc<dyn EnvironmentProvider>>,
    shell_state: SharedShellState,
}

fn default_shell_state() -> ShellState {
    ShellState {
        cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        env_vars: HashMap::new(),
        generation: 0,
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        let env_provider: Arc<dyn EnvironmentProvider> = Arc::new(LocalEnvironment::new());
        Self {
            defs: RwLock::new(HashMap::new()),
            handlers: RwLock::new(HashMap::new()),
            resume_capabilities: RwLock::new(HashMap::new()),
            env_provider: RwLock::new(env_provider),
            shell_state: Arc::new(Mutex::new(default_shell_state())),
        }
    }

    pub fn set_environment_provider(&self, provider: Arc<dyn EnvironmentProvider>) {
        *self
            .env_provider
            .write()
            .expect("tool registry env provider lock poisoned") = provider;
    }

    pub fn environment_provider(&self) -> Arc<dyn EnvironmentProvider> {
        self.env_provider
            .read()
            .expect("tool registry env provider lock poisoned")
            .clone()
    }

    pub fn shell_state(&self) -> SharedShellState {
        Arc::clone(&self.shell_state)
    }

    pub fn make_tool_context(&self, cancellation: CancellationToken) -> ToolContext {
        ToolContext::new(
            self.environment_provider(),
            self.shell_state(),
            cancellation,
        )
    }

    /// Register a tool with its definition and handler.
    /// Thread-safe: can be called concurrently from background tasks.
    pub fn register(&self, def: ToolDef, handler: Arc<dyn ToolHandler>) {
        let name = def.name.clone();
        self.defs
            .write()
            .expect("tool registry defs lock poisoned")
            .insert(name.clone(), def);
        self.handlers
            .write()
            .expect("tool registry handlers lock poisoned")
            .insert(name, handler);
    }

    pub fn register_resume_capability(
        &self,
        name: impl Into<String>,
        capability: ToolResumeCapability,
    ) {
        self.resume_capabilities
            .write()
            .expect("tool registry resume capability lock poisoned")
            .insert(name.into(), capability);
    }

    pub fn resume_capability(&self, name: &str) -> ToolResumeCapability {
        if let Some(capability) = self
            .resume_capabilities
            .read()
            .expect("tool registry resume capability lock poisoned")
            .get(name)
            .copied()
        {
            return capability;
        }

        let Some(def) = self.get_def(name) else {
            return ToolResumeCapability::Unsupported;
        };

        // Compatibility shim while older tool registrations lack explicit
        // resume metadata. New tools should call `register_resume_capability`.
        match (def.category.as_str(), name) {
            ("file_ops", "write_file")
            | ("file_ops", "create_directory")
            | ("file_ops", "copy_file")
            | ("shell", "execute_bash")
            | ("shell", "execute_python") => ToolResumeCapability::DeterministicLocal,
            ("gui_automation", _)
            | ("vision_automation", _)
            | ("desktop", _)
            | ("internet", "browser_search")
            | ("internet", "open_url") => ToolResumeCapability::RequiresLiveGui,
            ("fleet", _) | ("google_workspace", _) | ("mcp", _) | ("n8n", _) | ("openclaw", _) => {
                ToolResumeCapability::ExternalDelegated
            }
            _ => ToolResumeCapability::Unsupported,
        }
    }

    /// Get a tool definition by name.
    pub fn get_def(&self, name: &str) -> Option<ToolDef> {
        self.defs
            .read()
            .expect("tool registry defs lock poisoned")
            .get(name)
            .cloned()
    }

    /// Get a tool handler by name.
    pub fn get_handler(&self, name: &str) -> Option<Arc<dyn ToolHandler>> {
        self.handlers
            .read()
            .expect("tool registry handlers lock poisoned")
            .get(name)
            .cloned()
    }

    /// List all tool definitions (for LLM system prompt).
    pub fn list_defs(&self) -> Vec<ToolDef> {
        self.defs
            .read()
            .expect("tool registry defs lock poisoned")
            .values()
            .cloned()
            .collect()
    }

    /// List tool definitions filtered by hardware tier.
    pub fn list_for_tier(&self, hw_tier: &str) -> Vec<ToolDef> {
        let tier_rank = |t: &str| -> u8 {
            match t {
                "lite" => 0,
                "standard" => 1,
                "performance" => 2,
                "high" => 3,
                _ => 0,
            }
        };
        let rank = tier_rank(hw_tier);
        self.defs
            .read()
            .expect("tool registry defs lock poisoned")
            .values()
            .filter(|d| tier_rank(d.min_tier) <= rank)
            .cloned()
            .collect()
    }

    /// List tools by category.
    pub fn list_by_category(&self, category: &str) -> Vec<ToolDef> {
        self.defs
            .read()
            .expect("tool registry defs lock poisoned")
            .values()
            .filter(|d| d.category == category)
            .cloned()
            .collect()
    }

    /// Remove all tools in a category.
    /// Returns the number of removed tools.
    pub fn unregister_category(&self, category: &str) -> usize {
        let names: Vec<String> = {
            let defs = self.defs.read().expect("tool registry defs lock poisoned");
            defs.values()
                .filter(|d| d.category == category)
                .map(|d| d.name.clone())
                .collect()
        };

        if names.is_empty() {
            return 0;
        }

        {
            let mut defs = self.defs.write().expect("tool registry defs lock poisoned");
            for name in &names {
                defs.remove(name);
            }
        }

        {
            let mut handlers = self
                .handlers
                .write()
                .expect("tool registry handlers lock poisoned");
            for name in &names {
                handlers.remove(name);
            }
        }

        names.len()
    }

    /// Remove a single tool by name (returns true if it existed). Used for hot uninstall of
    /// OpenClaw skills (bundle installer deactivation).
    pub fn unregister(&self, name: &str) -> bool {
        let removed_def = self
            .defs
            .write()
            .expect("tool registry defs lock poisoned")
            .remove(name)
            .is_some();
        self.handlers
            .write()
            .expect("tool registry handlers lock poisoned")
            .remove(name);
        self.resume_capabilities
            .write()
            .expect("tool registry resume capability lock poisoned")
            .remove(name);
        removed_def
    }

    /// Get all category names.
    pub fn categories(&self) -> Vec<String> {
        let defs = self.defs.read().expect("tool registry defs lock poisoned");
        let mut cats: Vec<String> = defs.values().map(|d| d.category.clone()).collect();
        cats.sort();
        cats.dedup();
        cats
    }

    /// Generate the function schemas array for the LLM.
    pub fn function_schemas(&self, hw_tier: &str) -> Vec<serde_json::Value> {
        self.list_for_tier(hw_tier)
            .iter()
            .map(|d| d.to_function_schema())
            .collect()
    }

    /// Total number of registered tools.
    pub fn len(&self) -> usize {
        self.defs
            .read()
            .expect("tool registry defs lock poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs
            .read()
            .expect("tool registry defs lock poisoned")
            .is_empty()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Build the full tool registry with all built-in tools.
pub fn build_default_registry() -> ToolRegistry {
    build_registry_with_store(None)
}

/// Build with MemoryStore only (no RAG).
pub fn build_registry_with_store(
    store: Option<std::sync::Arc<dyn crate::memory::MemoryRuntime>>,
) -> ToolRegistry {
    build_registry_full(store, None, None)
}

/// Build the full tool registry with a MemoryStore, optional RagEngine, and optional ProactiveEngine.
///
/// Pass `psdg` to enable persistent browser/IDE cognition. All browser and IDE tools
/// will write state to WorldModelStore after each operation when this is `Some`.
pub fn build_registry_full(
    store: Option<std::sync::Arc<dyn crate::memory::MemoryRuntime>>,
    rag: Option<std::sync::Arc<crate::memory::rag::RagEngine>>,
    proactive: Option<std::sync::Arc<crate::automation::proactive::ProactiveEngine>>,
) -> ToolRegistry {
    build_registry_full_with_psdg(store, rag, proactive, None)
}

/// Build the full tool registry with all optional components including PSDG.
/// For WorkflowContinuationRuntime support, use `build_registry_full_with_psdg_wcr`.
pub fn build_registry_full_with_psdg(
    store: Option<std::sync::Arc<dyn crate::memory::MemoryRuntime>>,
    rag: Option<std::sync::Arc<crate::memory::rag::RagEngine>>,
    proactive: Option<std::sync::Arc<crate::automation::proactive::ProactiveEngine>>,
    psdg: Option<crate::agent::psdg::PsdgHandle>,
) -> ToolRegistry {
    build_registry_full_with_psdg_wcr(store, rag, proactive, psdg, None)
}

/// Build the full tool registry with all optional components including PSDG and
/// `WorkflowContinuationRuntime` (enables the `resume_workflow` tool).
pub fn build_registry_full_with_psdg_wcr(
    store: Option<std::sync::Arc<dyn crate::memory::MemoryRuntime>>,
    rag: Option<std::sync::Arc<crate::memory::rag::RagEngine>>,
    proactive: Option<std::sync::Arc<crate::automation::proactive::ProactiveEngine>>,
    psdg: Option<crate::agent::psdg::PsdgHandle>,
    continuation_runtime: Option<
        std::sync::Arc<crate::agent::workflow_continuation::WorkflowContinuationRuntime>,
    >,
) -> ToolRegistry {
    let reg = ToolRegistry::new();

    super::system_info::register(&reg);
    super::file_ops::register(&reg);
    super::app_lifecycle::register(&reg);
    super::shell::register(&reg);
    super::internet::register(&reg);
    if let Some(s) = store {
        super::knowledge::register(&reg, s);
    } else {
        // Register without memory backing (stubs for testing)
        super::knowledge::register_stubs(&reg);
    }
    super::system_config::register(&reg);
    super::power::register(&reg);
    super::process::register(&reg);
    super::documents::register(&reg);
    super::communication::register(&reg);
    super::tasks::register(&reg);
    super::interaction::register(&reg);
    super::disk::register(&reg);
    super::packages::register(&reg);
    super::scheduler::register(&reg);
    super::vision::register(&reg, None, None);
    super::desktop::register(&reg);
    super::developer::register(&reg);
    super::gui_automation::register(&reg);
    super::atspi_tools::register(&reg);
    super::cognition_tools::register(&reg, psdg, continuation_runtime);
    super::vision_automation::register(&reg);
    super::i18n::register(&reg);

    // Keep Google Workspace tools visible in the core registry even when MCP is
    // not connected yet. Desktop runtime will later wire a live client into its
    // own GwClientRef and can re-register with the active sidecar bridge.
    let gw_ref = super::google_workspace::new_client_ref();
    let gh_ref = super::google_workspace::new_github_client_ref();
    let gw_sidecar = std::sync::Arc::new(crate::sidecar::SidecarBridge::new("python3", None));
    super::google_workspace::register(&reg, gw_ref, gh_ref, gw_sidecar);

    if let Some(rag_engine) = rag {
        super::rag::register(&reg, rag_engine);
    }
    if let Some(proactive_engine) = proactive {
        super::proactive::register(&reg, proactive_engine);
    }

    // ── Stub tools required by cognitive routing tests ──
    // These tools are referenced in TestPrompts.txt / VMTestPrompts.txt and must
    // appear in the registry even if their runtime is not fully wired yet.

    // execute_fleet_command — runs a command on a remote fleet target via SSH.
    {
        #[derive(Clone)]
        struct FleetCommandStub;
        #[async_trait::async_trait]
        impl ToolHandler for FleetCommandStub {
            async fn execute(&self, _params: serde_json::Value) -> crate::infra::ToolResult {
                crate::infra::ToolResult::err(
                    "execute_fleet_command: fleet runtime not connected. \
                     Ensure a fleet target is enrolled and the executive controller is enabled.",
                )
            }
        }
        reg.register(
            ToolDef {
                name: "execute_fleet_command".into(),
                description: "Execute a shell command on a remote fleet target (VM/server) via SSH"
                    .into(),
                category: "fleet".into(),
                default_tier: crate::safety::RiskLevel::Red,
                min_tier: "standard",
                parameters: vec![
                    ParamDef {
                        name: "command".into(),
                        param_type: "string".into(),
                        description: "The shell command to execute on the remote target".into(),
                        required: true,
                        default: None,
                    },
                    ParamDef {
                        name: "target_id".into(),
                        param_type: "string".into(),
                        description: "The fleet target ID or hostname (optional, uses default target)"
                            .into(),
                        required: false,
                        default: None,
                    },
                ],
            },
            std::sync::Arc::new(FleetCommandStub),
        );
    }

    // list_files — convenience alias that lists files in a directory (like ls -la).
    {
        #[derive(Clone)]
        struct ListFilesStub;
        #[async_trait::async_trait]
        impl ToolHandler for ListFilesStub {
            async fn execute(&self, params: serde_json::Value) -> crate::infra::ToolResult {
                // Delegate to the existing list_directory handler
                let path = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                let abs_path = std::path::PathBuf::from(path);
                if !abs_path.exists() {
                    return crate::infra::ToolResult::err(format!("Path does not exist: {path}"));
                }
                match std::fs::read_dir(&abs_path) {
                    Ok(entries) => {
                        let mut files = Vec::new();
                        for entry in entries.flatten() {
                            let name = entry.file_name().to_string_lossy().to_string();
                            let is_dir = entry.path().is_dir();
                            files.push(format!("{}{}", name, if is_dir { "/" } else { "" }));
                        }
                        files.sort();
                        crate::infra::ToolResult::ok(serde_json::json!(files.join("\n")))
                    }
                    Err(e) => crate::infra::ToolResult::err(format!("Failed to list files: {e}")),
                }
            }
        }
        reg.register(
            ToolDef {
                name: "list_files".into(),
                description: "List files and directories in a given path".into(),
                category: "file_ops".into(),
                default_tier: crate::safety::RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![ParamDef {
                    name: "path".into(),
                    param_type: "string".into(),
                    description: "Directory path to list".into(),
                    required: true,
                    default: None,
                }],
            },
            std::sync::Arc::new(ListFilesStub),
        );
    }

    tracing::info!(count = reg.len(), "tool registry built");
    reg
}
