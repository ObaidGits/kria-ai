use super::*;
use async_trait::async_trait;
use kria_core::config::ChangeSource;
use kria_core::infra::event_bus::KriaEvent;
use kria_core::tools::feature_control::{
    FeatureControl, FeatureControlBackend, FeatureControlState,
};
use std::collections::{HashMap, HashSet};
use tauri::{AppHandle, Manager};

#[derive(Clone, Copy)]
struct FeatureSpec {
    id: &'static str,
    label: &'static str,
    description: &'static str,
    section: &'static str,
}

const FEATURES: &[FeatureSpec] = &[
    FeatureSpec {
        id: "mcp",
        label: "MCP Services",
        description: "External MCP servers and their discovered tools.",
        section: "mcp",
    },
    FeatureSpec {
        id: "gui_cognition",
        label: "GUI Cognition",
        description: "Vision sidecar and safe desktop automation daemon.",
        section: "gui_cognition",
    },
    FeatureSpec {
        id: "memory",
        label: "Long-term Memory",
        description: "Persistent memory, retrieval, knowledge graph, and background cognition.",
        section: "memory",
    },
    FeatureSpec {
        id: "tools",
        label: "Native Tools",
        description: "Master switch for KRIA native tool execution.",
        section: "tools",
    },
    FeatureSpec {
        id: "n8n",
        label: "n8n",
        description: "Workflow catalog and optional KRIA-managed n8n container.",
        section: "n8n",
    },
    FeatureSpec {
        id: "openclaw",
        label: "OpenClaw",
        description: "Sandboxed skill substrate and warm Docker pool.",
        section: "openclaw",
    },
    FeatureSpec {
        id: "google_workspace",
        label: "Google Workspace",
        description: "Google Workspace MCP runtime. OAuth credentials are managed separately.",
        section: "mcp",
    },
    FeatureSpec {
        id: "voice",
        label: "Voice",
        description:
            "Speech input/output. Enabling makes voice available; microphone starts on demand.",
        section: "voice",
    },
    FeatureSpec {
        id: "image_generation",
        label: "Image Generation",
        description: "Local or cloud image generation runtime.",
        section: "image_generation",
    },
    FeatureSpec {
        id: "mobile",
        label: "Mobile Gateway",
        description: "Phone-facing authenticated gateway.",
        section: "mobile",
    },
    FeatureSpec {
        id: "telegram",
        label: "Telegram",
        description: "Telegram bot bridge or MCP connector.",
        section: "telegram",
    },
    FeatureSpec {
        id: "colab",
        label: "Google Colab",
        description: "Colab cloud-tier MCP connector.",
        section: "colab",
    },
    FeatureSpec {
        id: "executive",
        label: "Executive Controller",
        description: "Background task and executive routing controller.",
        section: "executive",
    },
    FeatureSpec {
        id: "classifier",
        label: "Intent Classifier",
        description: "Local TurnGate classifier fallback.",
        section: "classifier",
    },
    FeatureSpec {
        id: "orchestrator",
        label: "Model Orchestrator",
        description: "Managed local model server and GPU lifecycle.",
        section: "orchestrator",
    },
    FeatureSpec {
        id: "capability",
        label: "Capability Platform",
        description: "Provider-neutral capability discovery and execution.",
        section: "capability",
    },
    FeatureSpec {
        id: "ntfy",
        label: "ntfy Notifications",
        description: "Optional push notifications.",
        section: "ntfy",
    },
    FeatureSpec {
        id: "remote_desktop",
        label: "Remote Desktop",
        description: "Portal capture and remote takeover capability.",
        section: "remote_desktop",
    },
];

#[derive(Clone)]
struct Transition {
    state: FeatureControlState,
    error: Option<String>,
}

pub struct FeatureControlRuntime {
    transitions: tokio::sync::RwLock<HashMap<String, Transition>>,
    reconcile_lock: tokio::sync::Mutex<()>,
}

impl FeatureControlRuntime {
    pub fn new() -> Self {
        Self {
            transitions: tokio::sync::RwLock::new(HashMap::new()),
            reconcile_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub(super) async fn lifecycle_guard(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.reconcile_lock.lock().await
    }

    async fn set_transition(
        &self,
        id: impl Into<String>,
        state: FeatureControlState,
        error: Option<String>,
    ) {
        self.transitions
            .write()
            .await
            .insert(id.into(), Transition { state, error });
    }

    async fn clear_transition(&self, id: &str) {
        self.transitions.write().await.remove(id);
    }

    async fn transition(&self, id: &str) -> Option<Transition> {
        self.transitions.read().await.get(id).cloned()
    }
}

impl Default for FeatureControlRuntime {
    fn default() -> Self {
        Self::new()
    }
}

fn spec(id: &str) -> Option<&'static FeatureSpec> {
    FEATURES.iter().find(|feature| feature.id == id)
}

fn desired(cfg: &KriaConfig, id: &str) -> Option<bool> {
    Some(match id {
        "mcp" => cfg.mcp.enabled,
        "gui_cognition" => cfg.gui_cognition.enabled,
        "memory" => cfg.memory.enabled,
        "tools" => cfg.tools.enabled,
        "n8n" => cfg.n8n.enabled,
        "openclaw" => cfg.openclaw.enabled,
        "google_workspace" => cfg
            .mcp
            .servers
            .iter()
            .find(|server| server.name.eq_ignore_ascii_case("gworkspace"))
            .map(|server| server.enabled)
            .unwrap_or(false),
        "voice" => cfg.voice.enabled,
        "image_generation" => cfg.image_generation.enabled,
        "mobile" => cfg.mobile.enabled,
        "telegram" => cfg.telegram.enabled,
        "colab" => cfg.colab.enabled,
        "executive" => cfg.executive.enabled,
        "classifier" => cfg.classifier.enabled,
        "orchestrator" => cfg.orchestrator.enabled,
        "capability" => cfg.capability.enabled,
        "ntfy" => cfg.ntfy.enabled,
        "remote_desktop" => cfg.remote_desktop.enabled,
        _ => return None,
    })
}

fn tool_group_id(category: &str) -> String {
    format!("tool_group::{category}")
}

fn tool_id(name: &str) -> String {
    format!("tool::{name}")
}

fn parse_tool_target(id: &str) -> Option<(&str, &str)> {
    id.split_once("::").and_then(|(kind, value)| {
        if matches!(kind, "tool_group" | "tool") && !value.is_empty() {
            Some((kind, value))
        } else {
            None
        }
    })
}

fn parse_mcp_server_target(id: &str) -> Option<&str> {
    id.strip_prefix("mcp_server::")
        .filter(|name| !name.is_empty())
}

fn configured_tool_availability(cfg: &KriaConfig) -> (HashSet<String>, HashSet<String>) {
    (
        cfg.tools.disabled_groups.iter().cloned().collect(),
        cfg.tools.disabled_tools.iter().cloned().collect(),
    )
}

fn apply_tool_availability(state: &AppState, cfg: &KriaConfig) {
    let (mut groups, tools) = configured_tool_availability(cfg);
    if !cfg.memory.enabled {
        groups.insert("knowledge".into());
    }
    if !cfg.gui_cognition.enabled {
        groups.insert("gui_automation".into());
        groups.insert("vision_automation".into());
    }
    if !cfg.image_generation.enabled {
        groups.insert("image".into());
    }
    if !cfg.n8n.enabled {
        groups.insert("n8n".into());
    }
    if !cfg.openclaw.enabled {
        groups.insert("openclaw".into());
        groups.insert("marketplace".into());
    }
    if !cfg.browser_agent.enabled {
        groups.insert("browser_agent".into());
    }
    state
        .tool_registry
        .set_availability(cfg.tools.enabled, groups.into_iter(), tools.into_iter());
}

async fn static_control(state: &AppState, feature: &FeatureSpec) -> FeatureControl {
    let cfg = state.config.read().await.clone();
    let enabled = desired(&cfg, feature.id).unwrap_or(false);
    if let Some(transition) = state.feature_controls.transition(feature.id).await {
        return FeatureControl {
            id: feature.id.into(),
            label: feature.label.into(),
            description: feature.description.into(),
            desired_enabled: enabled,
            state: transition.state,
            detail: None,
            error: transition.error,
        };
    }

    let (runtime_state, detail, error) = actual_state(state, feature.id, enabled).await;
    FeatureControl {
        id: feature.id.into(),
        label: feature.label.into(),
        description: feature.description.into(),
        desired_enabled: enabled,
        state: runtime_state,
        detail,
        error,
    }
}

async fn actual_state(
    state: &AppState,
    id: &str,
    enabled: bool,
) -> (FeatureControlState, Option<String>, Option<String>) {
    if !enabled {
        return (FeatureControlState::Disabled, None, None);
    }
    match id {
        "mcp" => {
            let statuses = state.mcp_manager.lock().await.status().await;
            let enabled_count = statuses.iter().filter(|status| status.enabled).count();
            let running = statuses
                .iter()
                .filter(|status| status.enabled && status.state == McpServerState::Running)
                .count();
            let errors: Vec<String> = statuses
                .iter()
                .filter(|status| status.enabled && status.state == McpServerState::Error)
                .map(|status| {
                    format!(
                        "{}: {}",
                        status.name,
                        status
                            .error
                            .clone()
                            .unwrap_or_else(|| "startup failed".into())
                    )
                })
                .collect();
            if !errors.is_empty() {
                (
                    FeatureControlState::Error,
                    Some(format!("{running}/{enabled_count} servers running")),
                    Some(errors.join("; ")),
                )
            } else {
                (
                    FeatureControlState::Running,
                    Some(format!("{running}/{enabled_count} enabled servers running")),
                    None,
                )
            }
        }
        "gui_cognition" => match state.gui_orchestrator.as_ref() {
            Some(orchestrator) => {
                let status = orchestrator.status().await;
                if status.automation_enabled && status.all_healthy() {
                    (
                        FeatureControlState::Running,
                        Some(format!(
                            "vision={:?}, input={:?}",
                            status.vision_sidecar, status.uinput_daemon
                        )),
                        None,
                    )
                } else {
                    (
                        FeatureControlState::Error,
                        Some(format!(
                            "vision={:?}, input={:?}",
                            status.vision_sidecar, status.uinput_daemon
                        )),
                        Some("GUI services are not ready".into()),
                    )
                }
            }
            None => (
                FeatureControlState::Error,
                None,
                Some("GUI orchestrator unavailable".into()),
            ),
        },
        "memory" => {
            let cognition_running = state
                .memory_cognition_task
                .lock()
                .await
                .as_ref()
                .map(|task| !task.is_finished())
                .unwrap_or(false);
            let enrichment_running = state.memory_system.enrichment_worker_running();
            if state.memory_system.is_enabled() && cognition_running && enrichment_running {
                (
                    FeatureControlState::Running,
                    Some("Persistent writes, retrieval, enrichment, and cognition enabled".into()),
                    None,
                )
            } else {
                (
                    FeatureControlState::Error,
                    Some(format!(
                        "enrichment_worker={enrichment_running}, cognition_scheduler={cognition_running}"
                    )),
                    Some("Memory runtime is not fully running".into()),
                )
            }
        }
        "image_generation" => {
            if state.image_orchestrator.is_enabled() {
                (
                    FeatureControlState::Running,
                    Some("Image generation available; backend starts on demand".into()),
                    None,
                )
            } else {
                (
                    FeatureControlState::Error,
                    None,
                    Some("Image generation runtime gate is closed".into()),
                )
            }
        }
        "n8n" => {
            let maintenance_running = state
                .n8n_maintenance
                .lock()
                .await
                .as_ref()
                .map(|task| !task.is_finished())
                .unwrap_or(false);
            let catalog_ready = state.n8n_catalog.read().await.is_some();
            if maintenance_running && catalog_ready {
                (
                    FeatureControlState::Running,
                    Some("Workflow catalog and maintenance active".into()),
                    None,
                )
            } else {
                (
                    FeatureControlState::Error,
                    Some(format!(
                        "catalog={}, maintenance={}",
                        catalog_ready, maintenance_running
                    )),
                    Some("n8n runtime is not ready".into()),
                )
            }
        }
        "openclaw" => {
            let pool = state.container_pool.read().await.clone();
            match pool {
                Some(pool) => (
                    FeatureControlState::Running,
                    Some(format!(
                        "{} warm, {} active",
                        pool.warm_count_total().await,
                        pool.active_count().await
                    )),
                    None,
                ),
                None => (
                    FeatureControlState::Error,
                    None,
                    Some("Docker substrate unavailable".into()),
                ),
            }
        }
        "mobile" => {
            if super::mobile_gateway::gateway_running().await {
                (
                    FeatureControlState::Running,
                    Some("Mobile gateway listening".into()),
                    None,
                )
            } else {
                (
                    FeatureControlState::Error,
                    None,
                    Some("Mobile gateway is not listening".into()),
                )
            }
        }
        "voice" => {
            let active = state
                .voice_active
                .load(std::sync::atomic::Ordering::Relaxed);
            (
                FeatureControlState::Running,
                Some(
                    if active {
                        "Voice session active"
                    } else {
                        "Available; microphone starts on demand"
                    }
                    .into(),
                ),
                None,
            )
        }
        "executive" => {
            let alive = state
                .executive_sender
                .read()
                .await
                .as_ref()
                .map(|sender| sender.is_alive())
                .unwrap_or(false);
            if alive {
                (
                    FeatureControlState::Running,
                    Some("Executive dispatch loop active".into()),
                    None,
                )
            } else {
                (
                    FeatureControlState::Error,
                    None,
                    Some("Executive controller is not running".into()),
                )
            }
        }
        "orchestrator" => {
            if state.orchestrator.read().await.is_some() {
                (
                    FeatureControlState::Running,
                    Some("Local model lifecycle owner active".into()),
                    None,
                )
            } else {
                let uses_local_runtime = state
                    .config
                    .read()
                    .await
                    .providers
                    .active()
                    .map(|provider| {
                        provider.provider_type
                            == kria_core::llm::provider::config::ProviderType::LlamaCpp
                    })
                    .unwrap_or(false);
                if uses_local_runtime {
                    (
                        FeatureControlState::Error,
                        None,
                        Some("Model orchestrator is not running".into()),
                    )
                } else {
                    (
                        FeatureControlState::Running,
                        Some("Cloud provider active; local model process not needed".into()),
                        None,
                    )
                }
            }
        }
        _ => (
            FeatureControlState::Running,
            Some("Enabled and applied live".into()),
            None,
        ),
    }
}

async fn tool_controls(state: &AppState) -> Vec<FeatureControl> {
    let cfg = state.config.read().await.clone();
    let defs = state.tool_registry.all_defs();
    let mut categories: Vec<String> = defs
        .iter()
        .filter(|def| {
            !def.name.starts_with("mcp_")
                && !matches!(
                    def.name.as_str(),
                    "feature_status" | "feature_control" | "config_patch"
                )
        })
        .map(|def| def.category.clone())
        .collect();
    categories.sort();
    categories.dedup();

    let mut controls = Vec::new();
    for category in categories {
        let id = tool_group_id(&category);
        let enabled = cfg.tools.enabled && !cfg.tools.disabled_groups.contains(&category);
        let transition = state.feature_controls.transition(&id).await;
        controls.push(FeatureControl {
            id,
            label: format!("Tool group: {category}"),
            description: format!("All native tools in the '{category}' category."),
            desired_enabled: enabled,
            state: transition
                .as_ref()
                .map(|item| item.state)
                .unwrap_or(if enabled {
                    FeatureControlState::Running
                } else {
                    FeatureControlState::Disabled
                }),
            detail: Some(format!(
                "{} tools",
                defs.iter().filter(|def| def.category == category).count()
            )),
            error: transition.and_then(|item| item.error),
        });
    }

    let mut defs = defs;
    defs.sort_by(|a, b| a.name.cmp(&b.name));
    for def in defs {
        if def.name.starts_with("mcp_")
            || matches!(
                def.name.as_str(),
                "feature_status" | "feature_control" | "config_patch"
            )
        {
            continue;
        }
        let id = tool_id(&def.name);
        let enabled = cfg.tools.enabled
            && !cfg.tools.disabled_groups.contains(&def.category)
            && !cfg.tools.disabled_tools.contains(&def.name);
        let transition = state.feature_controls.transition(&id).await;
        controls.push(FeatureControl {
            id,
            label: format!("Tool: {}", def.name),
            description: def.description,
            desired_enabled: enabled,
            state: transition
                .as_ref()
                .map(|item| item.state)
                .unwrap_or(if enabled {
                    FeatureControlState::Running
                } else {
                    FeatureControlState::Disabled
                }),
            detail: Some(format!("Group: {}", def.category)),
            error: transition.and_then(|item| item.error),
        });
    }
    controls
}

async fn mcp_server_controls(state: &AppState) -> Vec<FeatureControl> {
    let cfg = state.config.read().await.clone();
    let statuses = state.mcp_manager.lock().await.status().await;
    let mut controls = Vec::new();
    for server in &cfg.mcp.servers {
        let id = format!("mcp_server::{}", server.name);
        let desired_enabled = server.enabled;
        let effective_enabled = cfg.mcp.enabled && server.enabled;
        let runtime = statuses.iter().find(|status| status.name == server.name);
        let transition = state.feature_controls.transition(&id).await;
        let (runtime_state, detail, error) = if let Some(transition) = transition {
            (transition.state, None, transition.error)
        } else if !effective_enabled {
            (
                FeatureControlState::Disabled,
                if desired_enabled && !cfg.mcp.enabled {
                    Some("Global MCP switch is off".into())
                } else {
                    None
                },
                None,
            )
        } else {
            match runtime.map(|status| status.state) {
                Some(McpServerState::Running) => (
                    FeatureControlState::Running,
                    runtime.map(|status| format!("{} tools", status.tool_count)),
                    None,
                ),
                Some(McpServerState::Starting) => (FeatureControlState::Starting, None, None),
                Some(McpServerState::Error) => (
                    FeatureControlState::Error,
                    None,
                    runtime.and_then(|status| status.error.clone()),
                ),
                _ => (
                    FeatureControlState::Error,
                    None,
                    Some("MCP server is stopped".into()),
                ),
            }
        };
        controls.push(FeatureControl {
            id,
            label: format!("MCP: {}", server.name),
            description: format!("Per-server MCP control for {}.", server.command),
            desired_enabled,
            state: runtime_state,
            detail,
            error,
        });
    }
    controls
}

pub async fn list_for_app(app: &AppHandle) -> Result<Vec<FeatureControl>, String> {
    let cell: tauri::State<'_, AppStateCell> = app.state();
    let state = cell
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    let mut controls = Vec::with_capacity(FEATURES.len() + state.tool_registry.len());
    for feature in FEATURES {
        controls.push(static_control(state, feature).await);
    }
    controls.extend(mcp_server_controls(state).await);
    controls.extend(tool_controls(state).await);
    Ok(controls)
}

async fn control_for_app(app: &AppHandle, id: &str) -> Result<FeatureControl, String> {
    list_for_app(app)
        .await?
        .into_iter()
        .find(|control| control.id == id)
        .ok_or_else(|| format!("Unknown feature ID: {id}"))
}

async fn persist_desired(
    state: &AppState,
    id: &str,
    enabled: bool,
    source: ChangeSource,
) -> Result<(), String> {
    if let Some(feature) = spec(id) {
        if id == "google_workspace" {
            let mut servers = state.config.read().await.mcp.servers.clone();
            let server = servers
                .iter_mut()
                .find(|server| server.name.eq_ignore_ascii_case("gworkspace"))
                .ok_or_else(|| "gworkspace MCP server is not configured".to_string())?;
            server.enabled = enabled;
            state
                .config_service
                .patch("mcp", "servers", serde_json::json!(servers), source, None)
                .await
                .map_err(|error| error.to_string())?;
        } else {
            state
                .config_service
                .patch(
                    feature.section,
                    "enabled",
                    serde_json::json!(enabled),
                    source,
                    None,
                )
                .await
                .map_err(|error| error.to_string())?;
        }
        return Ok(());
    }

    if let Some(server_name) = parse_mcp_server_target(id) {
        let mut servers = state.config.read().await.mcp.servers.clone();
        let server = servers
            .iter_mut()
            .find(|server| server.name == server_name)
            .ok_or_else(|| format!("Unknown MCP server: {server_name}"))?;
        server.enabled = enabled;
        state
            .config_service
            .patch("mcp", "servers", serde_json::json!(servers), source, None)
            .await
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    let Some((kind, target)) = parse_tool_target(id) else {
        return Err(format!("Unknown feature ID: {id}"));
    };
    let cfg = state.config.read().await.clone();
    let mut values = if kind == "tool_group" {
        cfg.tools.disabled_groups
    } else {
        cfg.tools.disabled_tools
    };
    values.retain(|value| value != target);
    if !enabled {
        values.push(target.to_string());
        values.sort();
        values.dedup();
    }
    let field = if kind == "tool_group" {
        "disabled_groups"
    } else {
        "disabled_tools"
    };
    state
        .config_service
        .patch("tools", field, serde_json::json!(values), source, None)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

async fn reconcile_openclaw(state: &AppState, enabled: bool) -> Result<(), String> {
    if !enabled {
        let pool = state.container_pool.write().await.take();
        if let Some(pool) = pool {
            pool.shutdown().await.map_err(|error| error.to_string())?;
        }
        return Ok(());
    }

    let cfg = state.config.read().await.openclaw.clone();
    if let Some(pool) = state.container_pool.read().await.clone() {
        if pool.image() == cfg.image.as_str() {
            return Ok(());
        }
        let old = state.container_pool.write().await.take();
        if let Some(old) = old {
            old.shutdown().await.map_err(|error| error.to_string())?;
        }
    }
    kria_core::openclaw::trust_runtime::set_live_trust_config(cfg.trust.clone());
    let pool = Arc::new(
        kria_core::openclaw::ContainerPool::new(cfg)
            .await
            .map_err(|error| error.to_string())?,
    );
    pool.verify_image_available()
        .await
        .map_err(|error| error.to_string())?;
    pool.initialize().await.map_err(|error| error.to_string())?;
    kria_core::openclaw::ContainerPool::spawn_prewarm_loop(pool.clone());
    *state.container_pool.write().await = Some(pool);
    Ok(())
}

async fn reconcile_mcp_heartbeat(state: &AppState, enabled: bool) {
    let mut heartbeat = state.mcp_heartbeat.lock().await;
    if enabled {
        if heartbeat
            .as_ref()
            .map(|handle| !handle.is_finished())
            .unwrap_or(false)
        {
            return;
        }
        *heartbeat = Some(McpServerManager::spawn_health_heartbeat(
            state.mcp_manager.clone(),
            state.tool_registry.clone(),
            30,
        ));
    } else if let Some(handle) = heartbeat.take() {
        handle.abort();
    }
}

async fn reconcile_executive(
    state: &AppState,
    enabled: bool,
    app: &AppHandle,
) -> Result<(), String> {
    if !enabled {
        if let Some(sender) = state.executive_sender.write().await.take() {
            sender.shutdown();
        }
        return Ok(());
    }
    if state
        .executive_sender
        .read()
        .await
        .as_ref()
        .map(|sender| sender.is_alive())
        .unwrap_or(false)
    {
        return Ok(());
    }
    let settings = state.config.read().await.executive.clone();
    let config = kria_core::agent::executive::ExecutiveConfig {
        max_background_tasks: settings.max_background_tasks,
        preemption_grace_ms: settings.preemption_grace_ms,
        ..Default::default()
    };
    let policy_gate: Arc<dyn kria_core::safety::policy_gate::PolicyGate> =
        Arc::new(kria_core::safety::policy_gate::CapabilityPolicyGate::new());
    let (mut controller, sender) = kria_core::agent::executive::ExecutiveController::new(
        config,
        state.gpu_lease.clone(),
        policy_gate,
    );
    super::runtime::spawn_executive_event_forwarding(app.clone(), controller.subscribe_events());
    tokio::spawn(async move { controller.run().await });
    *state.executive_sender.write().await = Some(sender);
    Ok(())
}

pub(super) async fn stop_orchestrator_tasks(state: &AppState) {
    let mut tasks = state.orchestrator_tasks.lock().await;
    for task in tasks.drain(..) {
        task.abort();
    }
    drop(tasks);

    state.model_router.detach_server_manager();
    if let Some(hra) = kria_core::resource::authority::global_hra() {
        hra.residency().unregister("l1-llm").await;
    }
}

pub(super) async fn start_orchestrator_tasks(
    state: &AppState,
    app: &AppHandle,
    orchestrator: Arc<Orchestrator>,
) {
    // Caller drains the previous owner before starting/rebinding the new one.
    // This function only installs tasks for the already-published orchestrator.
    let mut tasks = Vec::new();

    let hra = if let Some(hra) = kria_core::resource::authority::global_hra() {
        hra
    } else {
        use kria_core::resource::authority::{ConsumerId, HraService, PolicyProfile};

        let gpu_total_vram = state.hardware_info.vram_mb.unwrap_or(0);
        let gpus = if gpu_total_vram > 0 {
            vec![(0, gpu_total_vram)]
        } else {
            Vec::new()
        };
        let journal_path = state
            .config
            .read()
            .await
            .resolve_paths()
            .map(|paths| paths.data_dir.join("hra_journal.bin"))
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "HRA: using default data path after config path resolution failed");
                kria_core::platform::paths::KriaPaths::resolve()
                    .data_dir
                    .join("hra_journal.bin")
            });
        let created = HraService::new_persisted(
            &gpus,
            512,
            state.hardware_info.total_ram_mb,
            &[],
            PolicyProfile::Balanced,
            journal_path,
        );
        let enforce = std::env::var("KRIA_HRA_ENFORCE")
            .ok()
            .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
                "0" | "false" | "off" | "no" => Some(false),
                "1" | "true" | "on" | "yes" => Some(true),
                _ => None,
            })
            .unwrap_or(true);
        created.set_shadow_only(!enforce);
        created.set_bypass(ConsumerId::Llm, false);
        kria_core::resource::authority::set_global_hra(created.clone());
        kria_core::resource::authority::global_hra().unwrap_or(created)
    };

    {
        let model = Arc::new(
            kria_core::llm::orchestrator::ra_adapter::OrchestratorModel::new(
                orchestrator.clone(),
                "l1-llm",
                state.hardware_info.vram_mb.unwrap_or(0).min(4096),
                2048,
            ),
        );
        // ResidencyManager::register replaces by model ID, so every hot start
        // points HRA at the current orchestrator rather than a stopped owner.
        hra.residency().register(model).await;

        let sweep = hra.clone();
        tasks.push(tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                let _ = sweep.co_residency().reclaim_expired().await;
            }
        }));

        let app = app.clone();
        tasks.push(tokio::spawn(async move {
            let Some(hub) = kria_core::resource::global_telemetry_hub() else {
                return;
            };
            let mut telemetry = hub.subscribe();
            while telemetry.changed().await.is_ok() {
                let snapshot = telemetry.borrow().clone();
                hra.apply_snapshot(&snapshot);
                let _ = app.emit("resource:hra_status", hra.status_json());
            }
        }));
    }

    if orchestrator.config.idle_release_enabled {
        let idle_after =
            std::time::Duration::from_secs(orchestrator.config.idle_release_after_secs.max(30));
        let check_interval = std::time::Duration::from_secs(
            orchestrator.config.idle_release_check_interval_secs.max(1),
        );
        let active_turns = state.orchestrator_active_turns.clone();
        let last_activity = state.orchestrator_last_activity_at.clone();
        let voice_active = state.voice_active.clone();
        let app = app.clone();
        let idle_orchestrator = orchestrator.clone();
        tasks.push(tokio::spawn(async move {
            loop {
                tokio::time::sleep(check_interval).await;
                if voice_active.load(std::sync::atomic::Ordering::Relaxed)
                    || active_turns.load(std::sync::atomic::Ordering::SeqCst) > 0
                    || idle_orchestrator.server_manager.is_swapping()
                    || idle_orchestrator.server_manager.current_params().0 == 0
                {
                    continue;
                }
                let idle_for = last_activity.lock().await.elapsed();
                if idle_for < idle_after || !idle_orchestrator.server_manager.has_live_process().await
                {
                    continue;
                }
                match idle_orchestrator
                    .release_if_idle("desktop_idle_timeout")
                    .await
                {
                    Ok(true) => {
                        let _ = app.emit(
                            "orchestrator:idle_released",
                            serde_json::json!({ "idle_for_secs": idle_for.as_secs(), "mode": "unloaded" }),
                        );
                        touch_orchestrator_activity(&last_activity).await;
                    }
                    Ok(false) => {}
                    Err(error) => {
                        tracing::warn!(%error, "orchestrator: idle release attempt failed");
                        touch_orchestrator_activity(&last_activity).await;
                    }
                }
            }
        }));
    }

    let app = app.clone();
    let mut events = state.event_bus.subscribe();
    tasks.push(tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(KriaEvent::LlmSwapStarted {
                    from_ngl,
                    to_ngl,
                    emergency,
                }) => {
                    let _ = app.emit(
                        "orchestrator:swap_started",
                        serde_json::json!({ "from_ngl": from_ngl, "to_ngl": to_ngl, "emergency": emergency }),
                    );
                }
                Ok(KriaEvent::LlmSwapCompleted {
                    new_ngl,
                    new_context,
                    duration_ms,
                }) => {
                    let _ = app.emit(
                        "orchestrator:swap_completed",
                        serde_json::json!({ "new_ngl": new_ngl, "new_context": new_context, "duration_ms": duration_ms }),
                    );
                }
                Ok(KriaEvent::LlmSwapFailed { reason }) => {
                    let _ = app.emit(
                        "orchestrator:swap_failed",
                        serde_json::json!({ "reason": reason }),
                    );
                }
                Ok(KriaEvent::LlmStreamInterrupted) => {
                    let _ = app.emit("orchestrator:stream_interrupted", serde_json::json!({}));
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }));

    state.orchestrator_tasks.lock().await.extend(tasks);
}

async fn reconcile_one(app: &AppHandle, id: &str) -> Result<(), String> {
    let cell: tauri::State<'_, AppStateCell> = app.state();
    let state = cell
        .get()
        .ok_or_else(|| "KRIA runtime is not initialized".to_string())?;
    let _guard = state.feature_controls.reconcile_lock.lock().await;
    let cfg = state.config.read().await.clone();
    let enabled = if let Some(value) = desired(&cfg, id) {
        value
    } else if let Some(server_name) = parse_mcp_server_target(id) {
        cfg.mcp.enabled
            && cfg
                .mcp
                .servers
                .iter()
                .find(|server| server.name == server_name)
                .map(|server| server.enabled)
                .ok_or_else(|| format!("Unknown MCP server: {server_name}"))?
    } else if let Some((kind, target)) = parse_tool_target(id) {
        cfg.tools.enabled
            && if kind == "tool_group" {
                !cfg.tools
                    .disabled_groups
                    .iter()
                    .any(|value| value == target)
            } else {
                !cfg.tools.disabled_tools.iter().any(|value| value == target)
            }
    } else {
        return Err(format!("Unknown feature ID: {id}"));
    };

    apply_tool_availability(state, &cfg);
    match id {
        "mcp" | "google_workspace" | "colab" => {
            let result = super::mcp::apply_mcp_runtime_from_config(state).await;
            reconcile_mcp_heartbeat(state, cfg.mcp.enabled).await;
            let errors = result
                .get("report")
                .and_then(|report| report.get("errors"))
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            if enabled && !errors.is_empty() {
                return Err(errors
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join("; "));
            }
        }
        "gui_cognition" => {
            let orchestrator = state
                .gui_orchestrator
                .as_ref()
                .ok_or_else(|| "GUI orchestrator unavailable".to_string())?;
            orchestrator
                .set_automation_enabled(enabled)
                .await
                .map_err(|error| error.to_string())?;
        }
        "memory" => {
            state.memory_system.set_enabled(enabled);
            let mut task = state.memory_cognition_task.lock().await;
            if enabled {
                let needs_start = task
                    .as_ref()
                    .map(tokio::task::JoinHandle::is_finished)
                    .unwrap_or(true);
                if needs_start {
                    if let Some(stale) = task.take() {
                        stale.abort();
                    }
                    *task = Some(super::runtime::spawn_memory_cognition_task(
                        state.memory_system.clone(),
                        app.clone(),
                    ));
                }
            } else if let Some(running) = task.take() {
                running.abort();
            }
        }
        "classifier" => {
            let ready = state
                .agent_loop
                .set_classifier_enabled(enabled, Some(&cfg.classifier.model_path));
            if enabled && !ready {
                return Err(format!(
                    "Classifier model is unavailable: {}",
                    cfg.classifier.model_path
                ));
            }
        }
        "capability" => {
            if !enabled {
                super::capability::stop_discovery();
            }
        }
        "n8n" => super::n8n::reconcile_n8n_feature(state, enabled, app).await?,
        "openclaw" => reconcile_openclaw(state, enabled).await?,
        "mobile" => {
            if enabled {
                super::mobile_gateway::start_gateway(state).await?;
            } else {
                super::mobile_gateway::stop_gateway().await;
            }
        }
        "ntfy" | "remote_desktop" => {
            super::mobile_gateway::reconcile_auxiliary_managers(state).await?;
        }
        "voice" => {
            if !enabled {
                super::voice::stop_voice_runtime(state, app).await;
            }
        }
        "image_generation" => {
            state.image_orchestrator.set_enabled(enabled);
            if !enabled {
                state.image_orchestrator.shutdown().await;
            }
        }
        "telegram" => super::telegram::reconcile_telegram_feature(state, enabled).await?,
        "executive" => reconcile_executive(state, enabled, app).await?,
        "orchestrator" => {
            if !enabled {
                stop_orchestrator_tasks(state).await;
                if let Some(orchestrator) = state.orchestrator.write().await.take() {
                    orchestrator.shutdown().await;
                }
            } else if state.orchestrator.read().await.is_none() {
                let provider = cfg
                    .providers
                    .active()
                    .cloned()
                    .ok_or_else(|| "No active LLM provider configured".to_string())?;
                if provider.provider_type
                    == kria_core::llm::provider::config::ProviderType::LlamaCpp
                {
                    stop_orchestrator_tasks(state).await;
                    let orchestrator =
                        super::providers::start_local_orchestrator(state, &cfg, &provider).await?;
                    state
                        .model_router
                        .attach_server_manager(orchestrator.server_manager.clone());
                    *state.orchestrator.write().await = Some(orchestrator.clone());
                    start_orchestrator_tasks(state, app, orchestrator).await;
                }
            }
        }
        _ if parse_mcp_server_target(id).is_some() => {
            let result = super::mcp::apply_mcp_runtime_from_config(state).await;
            reconcile_mcp_heartbeat(state, cfg.mcp.enabled).await;
            let errors = result["report"]["errors"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if enabled && !errors.is_empty() {
                return Err(errors
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join("; "));
            }
        }
        _ => {}
    }
    apply_tool_availability(state, &state.config.read().await.clone());
    Ok(())
}

fn schedule_reconcile(app: AppHandle, id: String) {
    tauri::async_runtime::spawn(async move {
        let cell: tauri::State<'_, AppStateCell> = app.state();
        let Some(state) = cell.get() else { return };
        let enabled = {
            let cfg = state.config.read().await;
            desired(&cfg, &id).unwrap_or_else(|| {
                if let Some(server_name) = parse_mcp_server_target(&id) {
                    return cfg.mcp.enabled
                        && cfg
                            .mcp
                            .servers
                            .iter()
                            .find(|server| server.name == server_name)
                            .map(|server| server.enabled)
                            .unwrap_or(false);
                }
                parse_tool_target(&id)
                    .map(|(kind, target)| {
                        cfg.tools.enabled
                            && if kind == "tool_group" {
                                !cfg.tools
                                    .disabled_groups
                                    .iter()
                                    .any(|value| value == target)
                            } else {
                                !cfg.tools.disabled_tools.iter().any(|value| value == target)
                            }
                    })
                    .unwrap_or(false)
            })
        };
        state
            .feature_controls
            .set_transition(
                id.clone(),
                if enabled {
                    FeatureControlState::Starting
                } else {
                    FeatureControlState::Stopping
                },
                None,
            )
            .await;
        match reconcile_one(&app, &id).await {
            Ok(()) => state.feature_controls.clear_transition(&id).await,
            Err(error) => {
                tracing::warn!(feature = %id, %error, "feature reconciliation failed");
                state
                    .feature_controls
                    .set_transition(id, FeatureControlState::Error, Some(error))
                    .await;
            }
        }
    });
}

async fn set_for_app(
    app: &AppHandle,
    id: &str,
    enabled: bool,
    source: ChangeSource,
) -> Result<FeatureControl, String> {
    let cell: tauri::State<'_, AppStateCell> = app.state();
    let state = cell
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    if spec(id).is_none()
        && parse_tool_target(id).is_none()
        && parse_mcp_server_target(id).is_none()
    {
        return Err(format!("Unknown feature ID: {id}"));
    }
    state
        .feature_controls
        .set_transition(
            id.to_string(),
            if enabled {
                FeatureControlState::Starting
            } else {
                FeatureControlState::Stopping
            },
            None,
        )
        .await;
    if let Err(error) = persist_desired(state, id, enabled, source).await {
        state.feature_controls.clear_transition(id).await;
        return Err(error);
    }
    schedule_reconcile(app.clone(), id.to_string());
    control_for_app(app, id).await
}

struct DesktopFeatureControlBackend {
    app: AppHandle,
}

#[async_trait]
impl FeatureControlBackend for DesktopFeatureControlBackend {
    async fn list(&self) -> Result<Vec<FeatureControl>, String> {
        list_for_app(&self.app).await
    }

    async fn set_enabled(&self, id: &str, enabled: bool) -> Result<FeatureControl, String> {
        set_for_app(&self.app, id, enabled, ChangeSource::Prompt).await
    }
}

fn ids_for_section(section: &str) -> Vec<String> {
    if section == "*" {
        return FEATURES
            .iter()
            .map(|feature| feature.id.to_string())
            .collect();
    }
    let mut ids: Vec<String> = FEATURES
        .iter()
        .filter(|feature| feature.section == section)
        .map(|feature| feature.id.to_string())
        .collect();
    if section == "tools" {
        ids.push("tools".into());
    }
    ids
}

pub async fn initialize(app: &AppHandle) {
    let cell: tauri::State<'_, AppStateCell> = app.state();
    let Some(state) = cell.get() else {
        tracing::error!("cannot initialize feature controls before AppState");
        return;
    };
    state
        .tool_registry
        .set_feature_control_backend(Arc::new(DesktopFeatureControlBackend { app: app.clone() }));
    apply_tool_availability(state, &state.config.read().await.clone());

    let mut events = state.config_service.subscribe();
    let listener_app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let section = match events.recv().await {
                Ok(KriaEvent::ConfigChanged { section, .. }) => section,
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => "*".into(),
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            for id in ids_for_section(&section) {
                let cell: tauri::State<'_, AppStateCell> = listener_app.state();
                let transition_in_progress = match cell.get() {
                    Some(state) => state.feature_controls.transition(&id).await.is_some(),
                    None => false,
                };
                if !transition_in_progress {
                    schedule_reconcile(listener_app.clone(), id);
                }
            }
        }
    });

    for feature in FEATURES {
        // Model startup has an existing non-blocking owner below runtime init
        // that also wires HRA, idle release, and event forwarding. Starting it
        // here would race that owner and spawn a duplicate llama-server.
        if feature.id != "orchestrator" {
            schedule_reconcile(app.clone(), feature.id.to_string());
        }
    }
}

#[tauri::command]
pub async fn list_feature_controls(app: AppHandle) -> Result<Vec<FeatureControl>, String> {
    list_for_app(&app).await
}

#[tauri::command]
pub async fn set_feature_enabled(
    feature_id: String,
    enabled: bool,
    app: AppHandle,
) -> Result<FeatureControl, String> {
    let cell: tauri::State<'_, AppStateCell> = app.state();
    let state = cell
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;
    use kria_core::safety::hitl::ApprovalResponse;
    let risk = if matches!(
        feature_id.as_str(),
        "gui_cognition" | "mobile" | "remote_desktop"
    ) {
        RiskLevel::Red
    } else {
        RiskLevel::Yellow
    };
    match super::config_prompt::request_settings_approval(
        &app,
        &state.hitl,
        "features",
        &feature_id,
        &serde_json::json!(enabled),
        risk,
    )
    .await
    {
        ApprovalResponse::Approved => {}
        ApprovalResponse::Denied => return Err("Feature change was denied".into()),
        ApprovalResponse::Timeout => return Err("Feature change approval timed out".into()),
    }
    set_for_app(&app, &feature_id, enabled, ChangeSource::Ui).await
}

/// SkillRuntime adapter whose pool can be replaced while KRIA stays running.
pub(super) struct HotSwapDockerRuntime {
    pool: Arc<RwLock<Option<Arc<kria_core::openclaw::ContainerPool>>>>,
}

impl HotSwapDockerRuntime {
    pub fn new(pool: Arc<RwLock<Option<Arc<kria_core::openclaw::ContainerPool>>>>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl kria_core::openclaw::runtime::SkillRuntime for HotSwapDockerRuntime {
    fn kind(&self) -> kria_core::openclaw::runtime::RuntimeKind {
        kria_core::openclaw::runtime::RuntimeKind::Docker
    }

    async fn execute(
        &self,
        spec: kria_core::openclaw::runtime::LaunchSpec,
        ctx: kria_core::openclaw::runtime::RuntimeContext,
    ) -> kria_core::infra::ToolResult {
        let Some(pool) = self.pool.read().await.clone() else {
            return kria_core::infra::ToolResult::err("OpenClaw is disabled or unavailable.");
        };
        kria_core::openclaw::runtime::DockerRuntime::new(pool)
            .execute(spec, ctx)
            .await
    }
}
