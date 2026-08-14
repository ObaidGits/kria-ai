use crate::infra::environment::{
    EnvironmentProvider, LocalEnvironment, SharedShellState, ShellState,
};
use crate::infra::ToolResult;
use crate::os_control::{OsControlRuntime, ToolContractMetadata};
use crate::safety::RiskLevel;
use crate::tools::ToolContext;
use async_trait::async_trait;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
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

/// Typed construction error for the strict OS-contract registration path
/// (linux-os-control-production Task 1.2, design §15). Registering a native OS
/// tool must **fail closed** — never silently overwrite — when the registration
/// duplicates, is orphaned/unknown, drifts from the frozen §§10.1–10.4 manifest,
/// or is internally inconsistent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolRegistrationError {
    /// A tool definition (name) is already registered.
    DuplicateDefinition(String),
    /// A handler is already registered under this name.
    DuplicateHandler(String),
    /// The operation/oracle id is already claimed by a different tool (alias).
    DuplicateAlias {
        /// The colliding identifier.
        id: String,
        /// The tool already owning it.
        owner: String,
    },
    /// The tool is not a canonical operation in the frozen manifest.
    UnknownOsOperation(String),
    /// The supplied contract does not byte-match the frozen manifest entry.
    ManifestDrift {
        /// Offending tool name.
        tool: String,
        /// Redacted detail of the divergence.
        detail: String,
    },
    /// The contract is internally incomplete/unclassified/non-total.
    IncompleteContract {
        /// Offending tool name.
        tool: String,
        /// Reason.
        detail: String,
    },
}

impl std::fmt::Display for ToolRegistrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateDefinition(name) => {
                write!(
                    f,
                    "duplicate tool definition `{name}` (register must not overwrite)"
                )
            }
            Self::DuplicateHandler(name) => write!(f, "duplicate tool handler `{name}`"),
            Self::DuplicateAlias { id, owner } => {
                write!(f, "alias `{id}` already registered by tool `{owner}`")
            }
            Self::UnknownOsOperation(name) => {
                write!(f, "`{name}` is not a canonical frozen OS operation")
            }
            Self::ManifestDrift { tool, detail } => {
                write!(f, "manifest drift for `{tool}`: {detail}")
            }
            Self::IncompleteContract { tool, detail } => {
                write!(f, "incomplete contract for `{tool}`: {detail}")
            }
        }
    }
}

impl std::error::Error for ToolRegistrationError {}

impl ToolDef {
    /// Convert to OpenAI-compatible function schema for LLM.
    ///
    /// For OS tools carrying a strict [`ToolContractMetadata`] the closed nested
    /// input schema (with `additionalProperties:false` at every object) is used
    /// verbatim; otherwise the flat [`ParamDef`] list is projected as before.
    pub fn to_function_schema(&self) -> serde_json::Value {
        self.to_function_schema_with_contract(None)
    }

    /// As [`ToolDef::to_function_schema`], but if `contract` is `Some` its strict
    /// closed input schema is emitted instead of the flat parameter projection.
    pub fn to_function_schema_with_contract(
        &self,
        contract: Option<&ToolContractMetadata>,
    ) -> serde_json::Value {
        if let Some(contract) = contract {
            return serde_json::json!({
                "type": "function",
                "function": {
                    "name": self.name,
                    "description": self.description,
                    "parameters": contract.input_schema.clone(),
                }
            });
        }

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

/// Honest "unavailable" handler for a canonical OS operation whose live provider
/// is not composed yet (linux-os-control-production Task 1.2, design §15).
///
/// It reaches host effects **only** through the injected [`OsControlRuntime`] in
/// the [`ToolContext`]; when no provider is composed it returns the frozen
/// `Unavailable` envelope and **never** falls back to `LocalEnvironment` or a
/// direct host subprocess. Per-domain migration (Tasks 2.x) replaces this with a
/// provider-backed handler that still routes through the same runtime seam.
pub struct OsUnavailableHandler {
    tool: String,
}

impl OsUnavailableHandler {
    /// Create an unavailable handler for a canonical OS `tool`.
    #[must_use]
    pub fn new(tool: impl Into<String>) -> Self {
        Self { tool: tool.into() }
    }

    fn envelope(&self, runtime: Option<&Arc<OsControlRuntime>>) -> ToolResult {
        let err = match runtime {
            Some(rt) => rt.unavailable(&self.tool),
            // No runtime injected at all — still fail closed, never fall back.
            None => crate::os_control::OsControlError::Unavailable {
                provider: None,
                reason: crate::os_control::contract::SafeText::new(
                    "OS control runtime is not injected in this build",
                ),
                retryable: false,
            },
        };
        ToolResult::err_with_data(err.code(), err.to_envelope())
    }
}

#[async_trait]
impl ToolHandler for OsUnavailableHandler {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        // No context path: cannot reach the runtime; fail closed with the frozen
        // envelope rather than performing any host effect.
        self.envelope(None)
    }

    async fn execute_with_context(
        &self,
        _params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        self.envelope(ctx.os_runtime.as_ref())
    }
}

/// Central tool registry. Holds all tool definitions and their handlers.
/// Thread-safe for dynamic registration (e.g. MCP servers connecting in background).
pub struct ToolRegistry {
    defs: RwLock<HashMap<String, ToolDef>>,
    handlers: RwLock<HashMap<String, Arc<dyn ToolHandler>>>,
    resume_capabilities: RwLock<HashMap<String, ToolResumeCapability>>,
    /// Strict per-operation OS contract metadata (linux-os-control-production
    /// Task 1.2, design §15). This is the authoritative store that makes the
    /// live registry exactly implement the frozen §§10.1–10.4 manifest. It is
    /// keyed by canonical tool name; the per-operation resume policy lives here
    /// (not in a parallel resume map) and the closed input schema is served from
    /// here for OS tools.
    os_contracts: RwLock<BTreeMap<String, Arc<ToolContractMetadata>>>,
    /// The governed OS-control runtime seam injected into OS handler contexts.
    /// Raw `HostOsControl` stays private behind it. Defaults to a detached
    /// runtime (no provider ⇒ `Unavailable`); live composition roots replace it.
    os_runtime: RwLock<Option<Arc<OsControlRuntime>>>,
    env_provider: RwLock<Arc<dyn EnvironmentProvider>>,
    shell_state: SharedShellState,
    tools_enabled: AtomicBool,
    disabled_categories: RwLock<HashSet<String>>,
    disabled_tools: RwLock<HashSet<String>>,
    /// Optional runtime adapter injected into feature status/control tool contexts.
    feature_control_backend:
        RwLock<Option<Arc<dyn crate::tools::feature_control::FeatureControlBackend>>>,
    /// Optional live ConfigService, injected into every ToolContext so tools
    /// like `config_patch` can read/mutate configuration (settings-config-revamp).
    config_service: RwLock<Option<Arc<crate::config::ConfigService>>>,
    /// Injection-wall taint (settings-config-revamp Task 0/13): set true once the
    /// Turn-scoped config overlay (settings-config-revamp Task 14). Set by the agent
    /// loop when a user prompt requests a temporary, whitelisted setting change
    /// ("... for this one"). Injected into each `ToolContext` and dropped (cleared)
    /// at the next turn boundary — never persisted.
    turn_override: RwLock<Option<Arc<crate::config::RequestOverride>>>,
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
            os_contracts: RwLock::new(BTreeMap::new()),
            os_runtime: RwLock::new(Some(Arc::new(OsControlRuntime::detached()))),
            env_provider: RwLock::new(env_provider),
            shell_state: Arc::new(Mutex::new(default_shell_state())),
            tools_enabled: AtomicBool::new(true),
            disabled_categories: RwLock::new(HashSet::new()),
            disabled_tools: RwLock::new(HashSet::new()),
            feature_control_backend: RwLock::new(None),
            config_service: RwLock::new(None),
            turn_override: RwLock::new(None),
        }
    }

    /// Clear any turn-scoped config override at a user-turn boundary so last turn's
    /// temporary override does not leak into this one (Task 14). Injection-wall
    /// provenance is now tracked PER-TURN by the agent loop (settings-nl-control
    /// Task 3, fixes NEW-5) — it is no longer a global flag on the registry.
    pub fn clear_turn_override(&self) {
        *self
            .turn_override
            .write()
            .expect("tool registry turn_override lock poisoned") = None;
    }

    /// Install a turn-scoped [`crate::config::RequestOverride`] for the current turn
    /// (settings-config-revamp Task 14). Injected into every `ToolContext` produced
    /// this turn; cleared at the next turn boundary via `clear_turn_override`.
    pub fn set_turn_override(&self, ov: Arc<crate::config::RequestOverride>) {
        *self
            .turn_override
            .write()
            .expect("tool registry turn_override lock poisoned") = Some(ov);
    }

    /// Whether a tool ingests content from OUTSIDE the trusted user channel
    /// (web pages, files on disk, MCP servers, marketplace/knowledge stores,
    /// remote/sidecar output). Once such a tool runs in a turn, later config
    /// mutations in that turn are treated as untrusted (injection wall, Req 9).
    ///
    /// settings-nl-control Task 2 (fixes NEW-3): classification is driven by the
    /// LIVE registry's `category` metadata (every registered tool declares one) +
    /// a small structural rule (MCP prefix, file-read discrimination) — NOT an
    /// outdated hardcoded name list. A new external tool with an existing external
    /// category is covered automatically.
    pub fn is_external_content_tool(&self, name: &str) -> bool {
        // MCP tools are external by construction (remote server output).
        if name.starts_with("mcp_") {
            return true;
        }
        let category = self
            .defs
            .read()
            .ok()
            .and_then(|defs| defs.get(name).map(|d| d.category.clone()));
        match category.as_deref() {
            // Content pulled from the internet / news / knowledge+marketplace stores
            // / RAG can carry injected instructions.
            Some("internet") | Some("web") | Some("news") | Some("knowledge")
            | Some("marketplace") | Some("rag") => true,
            // Filesystem: only READS ingest external content; writes do not.
            Some("file_ops") => Self::is_file_read_op(name),
            // Unregistered/edge tools: fall back to a structural read-name heuristic.
            _ => Self::is_file_read_op(name),
        }
    }

    /// Filesystem READ operations that pull file contents/listings into context.
    /// (Writes/renames/deletes are not content ingestion.)
    fn is_file_read_op(name: &str) -> bool {
        matches!(
            name,
            "read_file"
                | "read_files"
                | "search_files"
                | "search_file_contents"
                | "list_directory"
                | "get_file_info"
                | "get_project_structure"
                | "find_files_by_pattern"
                | "calculate_dir_size"
                | "fetch_article"
                | "fetch_url"
                | "download_file"
                | "parse_document"
                | "read_document"
                | "extract_document"
        )
    }

    /// Inject the host-owned feature lifecycle backend into every tool context.
    pub fn set_feature_control_backend(
        &self,
        backend: Arc<dyn crate::tools::feature_control::FeatureControlBackend>,
    ) {
        *self
            .feature_control_backend
            .write()
            .expect("tool registry feature_control_backend lock poisoned") = Some(backend);
    }

    /// The live ConfigService, if injected (used by the settings NL pipeline +
    /// `config_patch`). `None` before runtime wiring.
    pub fn config_service(&self) -> Option<Arc<crate::config::ConfigService>> {
        self.config_service
            .read()
            .expect("tool registry config_service lock poisoned")
            .clone()
    }

    /// Inject the live ConfigService so tool contexts can carry it (used by
    /// the `config_patch` tool). Called once at runtime startup.
    pub fn set_config_service(&self, svc: Arc<crate::config::ConfigService>) {
        *self
            .config_service
            .write()
            .expect("tool registry config_service lock poisoned") = Some(svc);
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
        // Default provenance is User. The injection-wall taint is PER-TURN and owned
        // by the agent loop (Task 3, NEW-5); config-capable dispatch sites pass the
        // turn's provenance explicitly via `make_tool_context_with_provenance`.
        self.make_tool_context_with_provenance(cancellation, crate::tools::TriggerProvenance::User)
    }

    /// Build a `ToolContext` stamping an explicit trigger provenance. The agent loop
    /// passes `ExternalContent` when the current turn has already run an
    /// external-content tool, so `config_patch` refuses injected mutations (Req 9).
    pub fn make_tool_context_with_provenance(
        &self,
        cancellation: CancellationToken,
        provenance: crate::tools::TriggerProvenance,
    ) -> ToolContext {
        let mut ctx = ToolContext::new(
            self.environment_provider(),
            self.shell_state(),
            cancellation,
        )
        .with_provenance(provenance);
        if let Some(backend) = self
            .feature_control_backend
            .read()
            .expect("tool registry feature_control_backend lock poisoned")
            .clone()
        {
            ctx = ctx.with_feature_control_backend(backend);
        }
        if let Some(svc) = self
            .config_service
            .read()
            .expect("tool registry config_service lock poisoned")
            .clone()
        {
            ctx = ctx.with_config(svc);
        }
        if let Some(ov) = self
            .turn_override
            .read()
            .expect("tool registry turn_override lock poisoned")
            .clone()
        {
            ctx = ctx.with_request_override(ov);
        }
        if let Some(rt) = self.os_runtime() {
            ctx = ctx.with_os_runtime(rt);
        }
        ctx
    }

    /// Inject the governed OS-control runtime seam (Task 1.2). Live desktop/server
    /// composition roots call this with a provider-backed runtime; until then a
    /// detached runtime is present and OS handlers return `Unavailable`. Raw
    /// `HostOsControl` is never exposed through the registry or a tool context.
    pub fn set_os_runtime(&self, runtime: Arc<OsControlRuntime>) {
        *self
            .os_runtime
            .write()
            .expect("tool registry os_runtime lock poisoned") = Some(runtime);
    }

    /// The injected OS-control runtime seam, if any.
    pub fn os_runtime(&self) -> Option<Arc<OsControlRuntime>> {
        self.os_runtime
            .read()
            .expect("tool registry os_runtime lock poisoned")
            .clone()
    }

    /// The strict frozen contract for a canonical OS tool, if registered.
    pub fn os_contract(&self, name: &str) -> Option<Arc<ToolContractMetadata>> {
        self.os_contracts
            .read()
            .expect("tool registry os_contracts lock poisoned")
            .get(name)
            .cloned()
    }

    /// Sorted canonical names of every registered OS contract.
    pub fn os_contract_names(&self) -> Vec<String> {
        self.os_contracts
            .read()
            .expect("tool registry os_contracts lock poisoned")
            .keys()
            .cloned()
            .collect()
    }

    /// Number of registered strict OS contracts.
    pub fn os_contract_count(&self) -> usize {
        self.os_contracts
            .read()
            .expect("tool registry os_contracts lock poisoned")
            .len()
    }

    /// Install the frozen §§10.1–10.4 manifest as strict OS contract metadata so
    /// the live registry exactly implements the F0 manifest (Task 1.2). This is
    /// an **additive** metadata layer: it does not register or overwrite the
    /// existing working tool handlers (per-domain provider migration and the
    /// hard cutover of direct execution paths are owned by Tasks 2.x). It fails
    /// construction on any drift, duplicate, or incomplete/inconsistent entry.
    pub fn install_frozen_os_contracts(&self) -> Result<(), ToolRegistrationError> {
        for contract in crate::os_control::frozen_contracts() {
            self.install_os_contract_metadata(Arc::new(contract.clone()))?;
        }
        Ok(())
    }

    /// Validate a contract against the frozen manifest + internal completeness,
    /// then store it in the authoritative OS-contract map. Fails closed on
    /// unknown operation, drift, incompleteness, or duplicate — never overwrites.
    fn install_os_contract_metadata(
        &self,
        contract: Arc<ToolContractMetadata>,
    ) -> Result<(), ToolRegistrationError> {
        let tool = contract.tool_name.clone();

        // Internal completeness (no placeholder/unclassified/non-total entry).
        contract
            .check_complete()
            .map_err(|detail| ToolRegistrationError::IncompleteContract {
                tool: tool.clone(),
                detail,
            })?;

        // Manifest drift: must byte-match the single frozen source of truth.
        match crate::os_control::frozen_contract(&tool) {
            None => return Err(ToolRegistrationError::UnknownOsOperation(tool)),
            Some(frozen) if frozen != contract.as_ref() => {
                return Err(ToolRegistrationError::ManifestDrift {
                    tool,
                    detail: "supplied contract differs from the frozen manifest entry".to_string(),
                });
            }
            Some(_) => {}
        }

        let mut map = self
            .os_contracts
            .write()
            .expect("tool registry os_contracts lock poisoned");

        // Duplicate definition (must not overwrite).
        if map.contains_key(&tool) {
            return Err(ToolRegistrationError::DuplicateDefinition(tool));
        }
        // Alias collision: no two operations may share an operation/oracle id.
        for existing in map.values() {
            if existing.operation_id == contract.operation_id {
                return Err(ToolRegistrationError::DuplicateAlias {
                    id: contract.operation_id.clone(),
                    owner: existing.tool_name.clone(),
                });
            }
            if existing.oracle == contract.oracle {
                return Err(ToolRegistrationError::DuplicateAlias {
                    id: contract.oracle.clone(),
                    owner: existing.tool_name.clone(),
                });
            }
        }

        map.insert(tool, contract);
        Ok(())
    }

    /// Register a fully-migrated OS tool: its strict frozen contract, definition,
    /// and provider-backed handler, in one fail-closed step (design §15). Used by
    /// the per-domain migration tasks (2.x) and by Task 1.2 tests. Fails on
    /// duplicate definition/handler/alias, manifest drift, or incomplete metadata
    /// — the registration is rejected rather than overwriting anything.
    pub fn register_os_contract(
        &self,
        contract: ToolContractMetadata,
        handler: Arc<dyn ToolHandler>,
    ) -> Result<(), ToolRegistrationError> {
        let contract = Arc::new(contract);
        let tool = contract.tool_name.clone();

        // A handler for this name must not already exist (fail, don't overwrite).
        if self
            .handlers
            .read()
            .expect("tool registry handlers lock poisoned")
            .contains_key(&tool)
        {
            return Err(ToolRegistrationError::DuplicateHandler(tool));
        }
        if self
            .defs
            .read()
            .expect("tool registry defs lock poisoned")
            .contains_key(&tool)
        {
            return Err(ToolRegistrationError::DuplicateDefinition(tool));
        }

        // Validate + store the strict contract (drift/dup/alias/incomplete).
        self.install_os_contract_metadata(Arc::clone(&contract))?;

        let def = ToolDef {
            name: tool.clone(),
            description: format!("Canonical native OS control operation `{tool}` (host-only)."),
            category: "os_control".into(),
            parameters: Vec::new(),
            default_tier: contract.default_tier(),
            min_tier: "lite",
        };
        self.defs
            .write()
            .expect("tool registry defs lock poisoned")
            .insert(tool.clone(), def);
        self.handlers
            .write()
            .expect("tool registry handlers lock poisoned")
            .insert(tool, handler);
        Ok(())
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
        // linux-os-control-production Task 1.2: OS operations derive their resume
        // capability from the per-operation `ResumePolicy` in their strict
        // contract — not from the parallel resume map.
        if let Some(contract) = self.os_contract(name) {
            return contract.resume_capability();
        }

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

    fn is_control_tool(name: &str) -> bool {
        matches!(name, "feature_status" | "feature_control" | "config_patch")
    }

    fn definition_enabled(&self, def: &ToolDef) -> bool {
        if Self::is_control_tool(&def.name) {
            return true;
        }
        self.tools_enabled.load(Ordering::Acquire)
            && !self
                .disabled_categories
                .read()
                .expect("tool registry disabled_categories lock poisoned")
                .contains(&def.category)
            && !self
                .disabled_tools
                .read()
                .expect("tool registry disabled_tools lock poisoned")
                .contains(&def.name)
    }

    /// Atomically replace native-tool visibility/execution controls.
    pub fn set_availability(
        &self,
        enabled: bool,
        disabled_categories: impl IntoIterator<Item = String>,
        disabled_tools: impl IntoIterator<Item = String>,
    ) {
        self.tools_enabled.store(enabled, Ordering::Release);
        *self
            .disabled_categories
            .write()
            .expect("tool registry disabled_categories lock poisoned") =
            disabled_categories.into_iter().collect();
        *self
            .disabled_tools
            .write()
            .expect("tool registry disabled_tools lock poisoned") =
            disabled_tools.into_iter().collect();
    }

    pub fn is_available(&self, name: &str) -> bool {
        self.defs
            .read()
            .expect("tool registry defs lock poisoned")
            .get(name)
            .map(|def| self.definition_enabled(def))
            .unwrap_or(false)
    }

    /// All definitions, including currently disabled tools. Used by Settings.
    pub fn all_defs(&self) -> Vec<ToolDef> {
        self.defs
            .read()
            .expect("tool registry defs lock poisoned")
            .values()
            .cloned()
            .collect()
    }

    /// Get a tool definition by name.
    pub fn get_def(&self, name: &str) -> Option<ToolDef> {
        self.defs
            .read()
            .expect("tool registry defs lock poisoned")
            .get(name)
            .filter(|def| self.definition_enabled(def))
            .cloned()
    }

    /// Get a tool handler by name.
    pub fn get_handler(&self, name: &str) -> Option<Arc<dyn ToolHandler>> {
        if !self.is_available(name) {
            return None;
        }
        self.handlers
            .read()
            .expect("tool registry handlers lock poisoned")
            .get(name)
            .cloned()
    }

    /// List all enabled tool definitions (for LLM system prompt).
    pub fn list_defs(&self) -> Vec<ToolDef> {
        self.defs
            .read()
            .expect("tool registry defs lock poisoned")
            .values()
            .filter(|def| self.definition_enabled(def))
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
            .filter(|d| self.definition_enabled(d) && tier_rank(d.min_tier) <= rank)
            .cloned()
            .collect()
    }

    /// List tools by category.
    pub fn list_by_category(&self, category: &str) -> Vec<ToolDef> {
        self.defs
            .read()
            .expect("tool registry defs lock poisoned")
            .values()
            .filter(|d| d.category == category && self.definition_enabled(d))
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

    /// Generate the function schemas array for the LLM. OS tools that carry a
    /// strict frozen contract emit their closed nested input schema (unknown
    /// fields denied); all other tools use the flat parameter projection.
    pub fn function_schemas(&self, hw_tier: &str) -> Vec<serde_json::Value> {
        self.list_for_tier(hw_tier)
            .iter()
            .map(|d| d.to_function_schema_with_contract(self.os_contract(&d.name).as_deref()))
            .collect()
    }

    /// Total number of registered tools.
    pub fn len(&self) -> usize {
        self.list_defs().len()
    }

    pub fn is_empty(&self) -> bool {
        self.list_defs().is_empty()
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

/// Build with a memory runtime only.
pub fn build_registry_with_store(
    store: Option<std::sync::Arc<dyn crate::memory::MemoryRuntime>>,
) -> ToolRegistry {
    build_registry_full(store, None)
}

/// Build the full tool registry with a memory runtime and optional ProactiveEngine.
///
/// Pass `psdg` to enable persistent browser/IDE cognition. All browser and IDE tools
/// will write state to WorldModelStore after each operation when this is `Some`.
pub fn build_registry_full(
    store: Option<std::sync::Arc<dyn crate::memory::MemoryRuntime>>,
    proactive: Option<std::sync::Arc<crate::automation::proactive::ProactiveEngine>>,
) -> ToolRegistry {
    build_registry_full_with_psdg(store, proactive, None)
}

/// Build the full tool registry with all optional components including PSDG.
/// For WorkflowContinuationRuntime support, use `build_registry_full_with_psdg_wcr`.
pub fn build_registry_full_with_psdg(
    store: Option<std::sync::Arc<dyn crate::memory::MemoryRuntime>>,
    proactive: Option<std::sync::Arc<crate::automation::proactive::ProactiveEngine>>,
    psdg: Option<crate::agent::psdg::PsdgHandle>,
) -> ToolRegistry {
    build_registry_full_with_psdg_wcr(store, proactive, psdg, None)
}

/// As [`build_registry_full_with_psdg`] but also injects the unified
/// [`MemorySystem`](crate::memory::api::MemorySystem) so the knowledge tools
/// (`recall_fact`/`search_knowledge`/`remember_fact`/`list_remembered`) and the
/// RAG/library tools (`ingest_document_rag`/`rag_query`/`list_knowledge_base`/
/// `delete_knowledge_item`) route through the single retrieval pipeline.
pub fn build_registry_full_with_memory(
    store: Option<std::sync::Arc<dyn crate::memory::MemoryRuntime>>,
    proactive: Option<std::sync::Arc<crate::automation::proactive::ProactiveEngine>>,
    psdg: Option<crate::agent::psdg::PsdgHandle>,
    continuation_runtime: Option<
        std::sync::Arc<crate::agent::workflow_continuation::WorkflowContinuationRuntime>,
    >,
    memory: Option<std::sync::Arc<crate::memory::api::MemorySystem>>,
) -> ToolRegistry {
    build_registry_inner(store, proactive, psdg, continuation_runtime, memory)
}

/// Build the full tool registry with all optional components including PSDG and
/// `WorkflowContinuationRuntime` (enables the `resume_workflow` tool).
pub fn build_registry_full_with_psdg_wcr(
    store: Option<std::sync::Arc<dyn crate::memory::MemoryRuntime>>,
    proactive: Option<std::sync::Arc<crate::automation::proactive::ProactiveEngine>>,
    psdg: Option<crate::agent::psdg::PsdgHandle>,
    continuation_runtime: Option<
        std::sync::Arc<crate::agent::workflow_continuation::WorkflowContinuationRuntime>,
    >,
) -> ToolRegistry {
    build_registry_inner(store, proactive, psdg, continuation_runtime, None)
}

fn build_registry_inner(
    store: Option<std::sync::Arc<dyn crate::memory::MemoryRuntime>>,
    proactive: Option<std::sync::Arc<crate::automation::proactive::ProactiveEngine>>,
    psdg: Option<crate::agent::psdg::PsdgHandle>,
    continuation_runtime: Option<
        std::sync::Arc<crate::agent::workflow_continuation::WorkflowContinuationRuntime>,
    >,
    memory: Option<std::sync::Arc<crate::memory::api::MemorySystem>>,
) -> ToolRegistry {
    let reg = ToolRegistry::new();

    // linux-os-control-production Task 1.2: make the live registry exactly
    // implement the frozen §§10.1–10.4 manifest by installing all 149 strict OS
    // contracts as authoritative metadata. This is additive (it does not
    // register/overwrite the existing working handlers — per-domain provider
    // migration is Tasks 2.x) and fails construction on any drift/duplicate.
    reg.install_frozen_os_contracts()
        .expect("frozen OS-control contract manifest must install without drift");

    // settings-config-revamp: prompt-driven settings tool. NL settings control is
    // ON by default (disable with KRIA_NL_SETTINGS=0); no-ops if the disable flag
    // is set or no ConfigService is injected.
    reg.register(
        super::config_patch::ConfigPatchTool::def(),
        Arc::new(super::config_patch::ConfigPatchTool),
    );
    super::feature_control::register(&reg);

    super::system_info::register(&reg);
    super::file_ops::register(&reg);
    super::app_lifecycle::register(&reg);
    super::shell::register(&reg);
    super::internet::register(&reg);
    if let Some(s) = store {
        super::knowledge::register(&reg, s, memory.clone());
    } else {
        // No memory runtime available (headless degraded "core registry only"
        // fallback, or a minimal test registry): register the knowledge tool
        // surface with honest "no memory store" no-op handlers (DC2).
        super::knowledge::register_stubs(&reg);
    }
    super::system_config::register(&reg);
    super::power::register(&reg);
    super::bluetooth::register(&reg);
    // Clipboard history, notifications/DND, capability reporting, and the
    // canonical graceful_close_application name.
    super::audio::register(&reg);
    super::connectivity::register(&reg);
    super::file_control::register(&reg);
    super::power_session::register(&reg);
    super::secrets::register(&reg);
    super::automation_control::register(&reg);
    super::print_privacy_firewall::register(&reg);
    super::system_services::register(&reg);
    super::desktop_state::register(&reg);
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

    if let Some(ms) = &memory {
        super::rag::register(&reg, ms.clone());
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

#[cfg(test)]
mod provenance_tests {
    use super::*;
    use crate::tools::TriggerProvenance;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn default_context_is_user_provenance() {
        let reg = ToolRegistry::new();
        let ctx = reg.make_tool_context(CancellationToken::new());
        assert_eq!(ctx.provenance, TriggerProvenance::User);
    }

    #[test]
    fn explicit_external_provenance_is_stamped() {
        let reg = ToolRegistry::new();
        let ctx = reg.make_tool_context_with_provenance(
            CancellationToken::new(),
            TriggerProvenance::ExternalContent,
        );
        assert_eq!(ctx.provenance, TriggerProvenance::ExternalContent);
        // Default path stays User (no global taint bleed — NEW-5).
        let ctx2 = reg.make_tool_context(CancellationToken::new());
        assert_eq!(ctx2.provenance, TriggerProvenance::User);
    }

    #[test]
    fn external_content_tool_classification_is_category_driven() {
        let reg = ToolRegistry::new();
        // Register a tool with an external category → detected via metadata.
        reg.register(
            ToolDef {
                name: "some_web_tool".into(),
                description: "d".into(),
                category: "internet".into(),
                parameters: vec![],
                default_tier: crate::safety::RiskLevel::Green,
                min_tier: "lite",
            },
            std::sync::Arc::new(NoopHandler),
        );
        assert!(reg.is_external_content_tool("some_web_tool")); // category=internet
        assert!(reg.is_external_content_tool("mcp_fs_read_file")); // mcp_ prefix
        assert!(reg.is_external_content_tool("read_file")); // file read fallback
        assert!(!reg.is_external_content_tool("config_patch")); // not external
        assert!(!reg.is_external_content_tool("write_file")); // file WRITE, not ingestion
    }

    struct NoopHandler;
    #[async_trait::async_trait]
    impl ToolHandler for NoopHandler {
        async fn execute(&self, _p: serde_json::Value) -> crate::infra::ToolResult {
            crate::infra::ToolResult::ok(serde_json::json!({}))
        }
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod os_contract_task_1_2_tests {
    //! linux-os-control-production Task 1.2 code-level validation
    //! (OSC-001, OSC-003, OSC-009, OSC-033).
    use super::*;
    use crate::os_control::testing::FakeHostOsControl;
    use crate::os_control::{
        frozen_contract, frozen_contracts, sentinel_is_armed, sentinel_trip_count,
        OsControlRuntime, FROZEN_OPERATION_COUNT,
    };
    use tokio_util::sync::CancellationToken;

    fn noop_handler() -> Arc<dyn ToolHandler> {
        struct H;
        #[async_trait]
        impl ToolHandler for H {
            async fn execute(&self, _p: serde_json::Value) -> ToolResult {
                ToolResult::ok(serde_json::json!({}))
            }
        }
        Arc::new(H)
    }

    // ── Exact §§10.1–10.4 manifest snapshot: the live registry implements it ──
    #[test]
    fn live_registry_exactly_implements_frozen_manifest() {
        let reg = build_default_registry();
        assert_eq!(reg.os_contract_count(), FROZEN_OPERATION_COUNT);

        let mut frozen_names: Vec<String> = frozen_contracts()
            .iter()
            .map(|c| c.tool_name.clone())
            .collect();
        frozen_names.sort();
        assert_eq!(reg.os_contract_names(), frozen_names);

        // Every live contract byte-matches its frozen manifest entry.
        for name in reg.os_contract_names() {
            let live = reg.os_contract(&name).expect("live contract present");
            let frozen = frozen_contract(&name).expect("frozen contract present");
            assert_eq!(live.as_ref(), frozen, "contract drift for {name}");
        }
    }

    // ── Strict nested schema with additionalProperties:false for OS tools ────
    #[test]
    fn os_tool_function_schema_is_strict_and_closed() {
        let reg = build_default_registry();
        // `set_volume` is an existing working def AND a frozen OS contract, so the
        // registry serves its strict closed schema (unknown fields denied).
        let schemas = reg.function_schemas("high");
        let set_volume = schemas
            .iter()
            .find(|s| s["function"]["name"] == "set_volume")
            .expect("set_volume tool present in the live registry");
        let params = &set_volume["function"]["parameters"];
        assert_eq!(params["type"], "object");
        assert_eq!(
            params["additionalProperties"],
            serde_json::Value::Bool(false)
        );
        // It equals the frozen contract's input schema verbatim.
        let frozen = frozen_contract("set_volume").unwrap();
        assert_eq!(params, &frozen.input_schema);
    }

    // ── Runtime injection into every tool context ────────────────────────────
    #[test]
    fn tool_context_carries_injected_os_runtime() {
        let reg = build_default_registry();
        let ctx = reg.make_tool_context(CancellationToken::new());
        let rt = ctx.os_runtime().expect("runtime injected");
        // Default composition is detached: no provider, so Unavailable.
        assert!(!rt.provider_present());
        assert_eq!(
            rt.probe_provider("set_volume").unwrap_err().code(),
            "os_control.unavailable"
        );
    }

    // ── Clone/context: the runtime survives context cloning (Arc-shared) ─────
    #[test]
    fn cloned_context_preserves_runtime_seam() {
        let reg = build_default_registry();
        let ctx = reg.make_tool_context(CancellationToken::new());
        let cloned = ctx.clone();
        assert!(cloned.os_runtime().is_some());
    }

    // ── Unavailable registry: no provider ⇒ frozen Unavailable, no fallback ──
    #[tokio::test]
    async fn os_unavailable_handler_returns_frozen_envelope() {
        let reg = build_default_registry(); // detached runtime
        let ctx = reg.make_tool_context(CancellationToken::new());
        let handler = OsUnavailableHandler::new("set_volume");
        let result = handler
            .execute_with_context(serde_json::json!({"percent": 50}), ctx)
            .await;
        assert!(!result.success);
        assert_eq!(result.data["error"]["code"], "os_control.unavailable");
        assert!(result.data["os_control"]["provider"].is_null());
        assert_eq!(result.data["os_control"]["availability"], "unavailable");

        // With NO runtime injected at all it still fails closed (never falls back
        // to LocalEnvironment or a subprocess).
        let bare = handler.execute(serde_json::json!({})).await;
        assert!(!bare.success);
        assert_eq!(bare.data["error"]["code"], "os_control.unavailable");
    }

    // ── Fake call-log: OS handlers are fake-testable through the runtime ─────
    #[test]
    fn registry_routes_context_through_fake_provider() {
        let reg = ToolRegistry::new();
        let fake = Arc::new(FakeHostOsControl::new("logind"));
        let recorder = fake.recorder();
        reg.set_os_runtime(Arc::new(OsControlRuntime::with_host(fake)));

        let ctx = reg.make_tool_context(CancellationToken::new());
        let rt = ctx.os_runtime().expect("runtime injected");
        assert!(rt.provider_present());
        let id = rt
            .probe_provider("reboot_system")
            .expect("composed provider");
        assert_eq!(id.as_str(), "logind");
        assert_eq!(recorder.labels(), vec!["provider_id".to_string()]);
    }

    // ── Duplicate registration fails construction (never overwrites) ─────────
    #[test]
    fn duplicate_os_contract_registration_fails() {
        let reg = ToolRegistry::new();
        let contract = frozen_contract("set_volume").unwrap().clone();
        reg.register_os_contract(contract.clone(), noop_handler())
            .expect("first registration succeeds");
        let err = reg
            .register_os_contract(contract, noop_handler())
            .expect_err("second registration must fail");
        assert!(matches!(
            err,
            ToolRegistrationError::DuplicateHandler(_)
                | ToolRegistrationError::DuplicateDefinition(_)
        ));
    }

    #[test]
    fn reinstalling_frozen_contracts_fails_closed() {
        let reg = build_default_registry(); // already installed once
        let err = reg
            .install_frozen_os_contracts()
            .expect_err("second install must not overwrite");
        assert!(matches!(err, ToolRegistrationError::DuplicateDefinition(_)));
    }

    // ── Inconsistent registration (manifest drift) fails construction ────────
    #[test]
    fn manifest_drift_registration_fails() {
        let reg = ToolRegistry::new();
        let mut drifted = frozen_contract("set_volume").unwrap().clone();
        drifted.redaction = "redaction.tampered".to_string();
        let err = reg
            .register_os_contract(drifted, noop_handler())
            .expect_err("drift must fail");
        assert!(matches!(err, ToolRegistrationError::ManifestDrift { .. }));
    }

    #[test]
    fn unknown_operation_registration_fails() {
        let reg = ToolRegistry::new();
        let mut fake = frozen_contract("set_volume").unwrap().clone();
        fake.tool_name = "not_a_real_operation".to_string();
        fake.operation_id = "os.not_a_real_operation".to_string();
        fake.oracle = "oracle.not_a_real_operation".to_string();
        let err = reg
            .register_os_contract(fake, noop_handler())
            .expect_err("unknown op must fail");
        assert!(matches!(err, ToolRegistrationError::UnknownOsOperation(_)));
    }

    #[test]
    fn incomplete_contract_registration_fails() {
        let reg = ToolRegistry::new();
        let mut bad = frozen_contract("set_volume").unwrap().clone();
        // operation id no longer matches the tool name → incomplete/inconsistent.
        bad.operation_id = "os.something_else".to_string();
        let err = reg
            .register_os_contract(bad, noop_handler())
            .expect_err("incomplete contract must fail");
        assert!(matches!(
            err,
            ToolRegistrationError::IncompleteContract { .. }
        ));
    }

    // ── Resume policy is served from the contract, not a parallel map ────────
    #[test]
    fn os_resume_capability_derives_from_contract() {
        let reg = build_default_registry();
        // A read → deterministic-local.
        assert_eq!(
            reg.resume_capability("get_audio_state"),
            ToolResumeCapability::DeterministicLocal
        );
        // A session-ending accepted action → never resume after dispatch.
        assert_eq!(
            reg.resume_capability("reboot_system"),
            ToolResumeCapability::Unsupported
        );
    }

    // ── Panic-sentinel: building the manifest + contexts opens no live path ──
    #[test]
    fn building_registry_trips_no_live_transport() {
        assert!(
            sentinel_is_armed(),
            "deny-live sentinel must be armed under os-control-test"
        );
        // The trip counter is process-global and other suites legitimately
        // exercise the tripwire in parallel, so compare a delta rather than an
        // absolute zero (the convention used by broker::packaging and
        // linux::structured_command).
        let trips_before = sentinel_trip_count();
        // Constructing the full registry (installing all 149 contracts) and
        // producing tool contexts must not open any live transport; if it did,
        // the sentinel would have panicked before we reach this assertion.
        let reg = build_default_registry();
        let _ctx = reg.make_tool_context(CancellationToken::new());
        // The frozen contract install path performed zero raw-transport calls.
        assert_eq!(
            sentinel_trip_count(),
            trips_before,
            "building the registry must open no live transport"
        );
    }
}
