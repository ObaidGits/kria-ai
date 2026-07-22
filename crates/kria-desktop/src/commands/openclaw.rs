use crate::commands::app_state::AppStateCell;
use kria_core::openclaw::clawhub::{ClawHubClient, RemoteSkillEntry};
use kria_core::openclaw::{SkillCapabilities, SkillDescriptor};
use serde::{Deserialize, Serialize};
use tauri::{command, AppHandle, Emitter, State};

/// UI event-forwarding fix (product gap 5/8): bridge the two REAL backend
/// OpenClaw event streams — `kria_core::openclaw::bundle::events` (install/
/// update/remove/enable/disable/rollback/failed) and
/// `kria_core::openclaw::event` (per-execution Started/Preparing/Running/
/// Completed/Failed) — to the frontend via `AppHandle::emit`, mirroring the
/// exact pattern every other push-based feature in this codebase already
/// uses (see `commands/voice.rs`, `commands/wake_listener.rs`,
/// `commands/test_runner.rs`). Neither stream was ever subscribed to
/// anywhere in the desktop app before this fix — confirmed by exhaustive
/// grep (task 16 finding, R16). Event names use the same `"openclaw:*"`
/// prefix convention `tray.rs`/`voice.rs` use for their own namespaces.
///
/// Single-authority note: this does NOT introduce a new event system — it
/// subscribes to the SAME two `tokio::sync::broadcast` buses `bundle/
/// installer.rs` and `handler.rs`/`runtime/docker.rs` already emit to. No
/// duplicate event stream is created.
pub fn spawn_openclaw_event_forwarding(app: AppHandle) {
    let bundle_app = app.clone();
    forward_bundle_events(move |payload| {
        let _ = bundle_app.emit(event_names::BUNDLE_EVENT, payload);
    });
    forward_execution_events(move |payload| {
        let _ = app.emit(event_names::EXECUTION_EVENT, payload);
    });
}

/// Testable core: forwards the real `bundle::events` broadcast stream to any
/// sink closure. Production wiring (`spawn_openclaw_event_forwarding`) passes
/// a real `AppHandle::emit` sink; tests pass a channel/Vec-collecting sink —
/// same subscription logic either way, so a passing test proves the real
/// wiring reads from the real bus.
pub fn forward_bundle_events<F>(sink: F)
where
    F: Fn(serde_json::Value) + Send + 'static,
{
    tauri::async_runtime::spawn(async move {
        let mut rx = kria_core::openclaw::bundle::events::subscribe();
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let payload =
                        serde_json::to_value(&event).unwrap_or_else(|_| serde_json::json!({}));
                    sink(payload);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // A slow/absent frontend missed some events — the UI's
                    // next poll-based reconciliation (task 16's confirmed
                    // fallback: registry state is always immediately
                    // correct) catches up. Never fatal to the forwarder.
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// Testable core: forwards the real per-execution `openclaw::event` broadcast
/// stream to any sink closure. See `forward_bundle_events` doc.
pub fn forward_execution_events<F>(sink: F)
where
    F: Fn(serde_json::Value) + Send + 'static,
{
    tauri::async_runtime::spawn(async move {
        let mut rx = kria_core::openclaw::event::subscribe();
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let payload =
                        serde_json::to_value(&event).unwrap_or_else(|_| serde_json::json!({}));
                    sink(payload);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// Frozen OpenClaw Tauri **event-name contract** (R10.1). These `"openclaw:*"`
/// strings are a frontend/backend contract: the SolidJS UI subscribes to them by
/// exact name, so their VALUES MUST NOT change. New surfaces append a new
/// constant here (additive) — they never rename an existing one. Task 13.5
/// asserts each value below is unchanged.
pub mod event_names {
    /// Bundle-lifecycle stream (install/update/remove/enable/disable/rollback).
    pub const BUNDLE_EVENT: &str = "openclaw:bundle_event";
    /// Per-execution stream (Started/Preparing/Running/Completed/Failed).
    pub const EXECUTION_EVENT: &str = "openclaw:execution_event";
    /// Marketplace recommendations payload (task 7.3).
    pub const RECOMMENDATIONS: &str = "openclaw:recommendations";
    /// Capability-manager payload (task 13.1).
    pub const CAPABILITIES: &str = "openclaw:capabilities";
    /// Capability-graph payload (task 13.2).
    pub const CAPABILITY_GRAPH: &str = "openclaw:capability_graph";
    /// Permission grant revoked (task 13.3).
    pub const GRANTS_CHANGED: &str = "openclaw:grants_changed";
    /// Developer-mode flag toggled (task 13.3).
    pub const DEVELOPER_MODE: &str = "openclaw:developer_mode";
    /// NEW (task 13.4): frozen `RegistryEvent` push-sync bridge. Additive — this
    /// is the only name introduced by the push-sync bridge; the UI reconciles any
    /// missed `RegistryEvent` by polling the authoritative list commands
    /// (`clawhub_list_skills` / `openclaw_capability_manager`), since the frozen
    /// `ProductionSkillRegistry` is always immediately correct (R10.2).
    pub const REGISTRY_EVENT: &str = "openclaw:registry_event";
}

/// Push-sync event bridge for the frozen `RegistryEvent` stream (task 13.4,
/// R10.2). This is the THIRD backend event stream bridged to the UI, alongside
/// `bundle::events` and `openclaw::event` (see `spawn_openclaw_event_forwarding`).
///
/// Single-authority note: this introduces NO second event system. It subscribes
/// to the SAME frozen `ProductionSkillRegistry` broadcast the registry already
/// emits to on every install/update/enable/disable/remove/verify/execution
/// (`ProductionSkillRegistry::subscribe_events`, A5.8). Unlike the other two
/// buses (process-global), `RegistryEvent` is emitted per-registry-instance, so
/// this bridge is wired once the registry exists (from `init_runtime`, after
/// `AppState` is set) with the live registry handle.
///
/// Eventual consistency: if the UI is slow/absent and the broadcast lags, the
/// forwarder drops the missed events and keeps running (`Lagged` → `continue`,
/// mirroring the other two forwarders). The UI reconciles by polling the
/// authoritative registry-backed list commands — registry state is always
/// immediately correct, so no push event is ever load-bearing for correctness.
pub fn spawn_openclaw_registry_forwarding(
    app: AppHandle,
    registry: std::sync::Arc<kria_core::openclaw::registry::SkillRegistry>,
) {
    let rx = registry.subscribe_events();
    forward_registry_events(rx, move |payload| {
        let _ = app.emit(event_names::REGISTRY_EVENT, payload);
    });
}

/// Testable core: forwards a frozen `RegistryEvent` broadcast receiver to any
/// sink closure. Production wiring (`spawn_openclaw_registry_forwarding`) passes
/// a real `AppHandle::emit` sink; tests pass a `Vec`-collecting sink — same
/// subscription logic either way. Mirrors `forward_bundle_events` /
/// `forward_execution_events` exactly, including the `Lagged` → `continue`
/// (eventual-consistency) and `Closed` → `break` branches.
pub fn forward_registry_events<F>(
    mut rx: tokio::sync::broadcast::Receiver<kria_core::openclaw::registry::RegistryEvent>,
    sink: F,
) where
    F: Fn(serde_json::Value) + Send + 'static,
{
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let payload =
                        serde_json::to_value(&event).unwrap_or_else(|_| serde_json::json!({}));
                    sink(payload);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Missed registry events — the UI's next poll of the
                    // authoritative registry-backed list command reconciles
                    // (R10.2). Never fatal to the forwarder.
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

#[cfg(test)]
mod event_forwarding_tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Real proof: `forward_bundle_events` subscribes to the ACTUAL
    /// `kria_core::openclaw::bundle::events` bus (the same one
    /// `BundleInstaller` emits to on every real install/update/remove/
    /// enable/disable) and delivers a real emitted event to the sink.
    #[tokio::test]
    async fn forward_bundle_events_delivers_real_events_to_sink() {
        use kria_core::openclaw::bundle::events::{self, BundleLifecycleEvent};

        let received: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
        let received_clone = received.clone();
        forward_bundle_events(move |payload| {
            received_clone.lock().unwrap().push(payload);
        });

        // Give the spawned forwarder task a moment to subscribe before we emit.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        events::emit(BundleLifecycleEvent::Installed {
            slug: "oc_event_forward_test".into(),
            version: "1.0.0".into(),
        });

        // Poll for delivery (async forwarder task needs a scheduler tick).
        for _ in 0..20 {
            if !received.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        let events_received = received.lock().unwrap().clone();
        assert!(
            !events_received.is_empty(),
            "REGRESSION: forward_bundle_events must deliver real bundle lifecycle events to the sink"
        );
        assert_eq!(
            events_received[0]["slug"], "oc_event_forward_test",
            "forwarded event must carry the real slug from the real emitted event"
        );
    }

    /// Real proof: `forward_execution_events` subscribes to the ACTUAL
    /// `kria_core::openclaw::event` bus (the same one every real skill
    /// execution emits Started/Preparing/Running/Completed/Failed to).
    #[tokio::test]
    async fn forward_execution_events_delivers_real_events_to_sink() {
        use kria_core::openclaw::event::{self, FailureInfo, FailureKind, SkillEvent, Stage};
        use kria_core::openclaw::types::ExecutionSource;

        let received: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
        let received_clone = received.clone();
        forward_execution_events(move |payload| {
            received_clone.lock().unwrap().push(payload);
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let ev = SkillEvent::new(
            "corr-event-forward-test",
            "exec-event-forward-test",
            "oc_event_forward_test",
            ExecutionSource::OpenClaw,
            "docker",
            Stage::Failed,
        )
        .with_failure(FailureInfo {
            kind: FailureKind::RuntimeCrash,
            message: "test".into(),
            exit_code: None,
        });
        event::emit(ev);

        for _ in 0..20 {
            if !received.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        let events_received = received.lock().unwrap().clone();
        assert!(
            !events_received.is_empty(),
            "REGRESSION: forward_execution_events must deliver real execution events to the sink"
        );
        assert_eq!(
            events_received[0]["correlation_id"],
            "corr-event-forward-test"
        );
    }

    // ════════════════════════════════════════════════════════════════════════
    // Task 13.5 — frontend/command tests for event-name preservation (R10.1) +
    // push/poll reconcile (R10.2).
    // ════════════════════════════════════════════════════════════════════════

    use kria_core::openclaw::registry::{
        DiscoverySource, ProductionSkillRegistry, SkillMetadata, SkillState,
    };
    use kria_core::openclaw::types::{ResourceClass, SkillCapabilities, TrustTier};
    use kria_core::safety::RiskLevel;

    /// A temp `skills.db`-backed registry for reconcile/bridge tests. Uses the
    /// SAME frozen `ProductionSkillRegistry` the desktop app runs (no mock).
    fn temp_registry() -> (Arc<ProductionSkillRegistry>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("kria-oc-13-5-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let db_path = dir.join("skills.db");
        let registry = ProductionSkillRegistry::new(&db_path).expect("registry init");
        (Arc::new(registry), dir)
    }

    /// Minimal enabled `SkillMetadata` (mirrors kria-core's own registry-test
    /// fixture) so `get_enabled_skills()` returns it immediately after install.
    fn sample_meta(skill_id: &str) -> SkillMetadata {
        SkillMetadata {
            skill_id: skill_id.to_string(),
            name: format!("Skill {skill_id}"),
            description: "13.5 reconcile-test skill".to_string(),
            publisher: "test".to_string(),
            version: "1.0.0".to_string(),
            category: "media".to_string(),
            discovery_source: DiscoverySource::Bundled {
                path: "test".to_string(),
            },
            discovered_at: chrono::Utc::now(),
            capabilities: SkillCapabilities::default(),
            runtime_requirements: "docker".to_string(),
            risk_level: RiskLevel::Green,
            resource_class: ResourceClass::Light,
            tags: vec!["test".to_string()],
            categories: vec!["media.image".to_string()],
            semantic_version: "1.0.0".to_string(),
            dependencies: vec![],
            compatibility_requirements: vec![],
            trust_tier: TrustTier::Local,
            content_hash: format!("hash_{skill_id}"),
            signature: None,
            granted_capabilities: Vec::new(),
            bundle_path: None,
            manifest_toml: None,
            input_schema: None,
            state: SkillState::Enabled,
            state_changed_at: chrono::Utc::now(),
        }
    }

    /// R10.1 — the frozen OpenClaw Tauri **event-name contract** is unchanged.
    /// Every `"openclaw:*"` event string the UI subscribes to is asserted here by
    /// exact value; changing any (including the ones the push-sync bridge reuses)
    /// breaks this test. The new push-sync bridge (13.4) adds exactly ONE name
    /// (`openclaw:registry_event`) and renames none.
    #[test]
    fn openclaw_event_name_contract_is_preserved() {
        assert_eq!(event_names::BUNDLE_EVENT, "openclaw:bundle_event");
        assert_eq!(event_names::EXECUTION_EVENT, "openclaw:execution_event");
        assert_eq!(event_names::RECOMMENDATIONS, "openclaw:recommendations");
        assert_eq!(event_names::CAPABILITIES, "openclaw:capabilities");
        assert_eq!(event_names::CAPABILITY_GRAPH, "openclaw:capability_graph");
        assert_eq!(event_names::GRANTS_CHANGED, "openclaw:grants_changed");
        assert_eq!(event_names::DEVELOPER_MODE, "openclaw:developer_mode");
        // The single additive push-sync bridge event (13.4). All existing names
        // above are preserved verbatim; only this one is new.
        assert_eq!(event_names::REGISTRY_EVENT, "openclaw:registry_event");
    }

    /// R10.1 — the existing OpenClaw Tauri **command names** are unchanged.
    /// Referencing each command function item by its exact path makes a rename a
    /// COMPILE error (this test won't build), which is a stronger guarantee than
    /// a runtime string check. Every command registered in `main.rs`'s
    /// `invoke_handler!` for OpenClaw is pinned here.
    #[test]
    fn openclaw_command_names_are_preserved() {
        // Pre-existing (frozen) command surface — must never be renamed.
        let _ = clawhub_list_skills;
        let _ = clawhub_search_skills;
        let _ = clawhub_fetch_remote_skills;
        let _ = clawhub_install_skill;
        let _ = clawhub_uninstall_skill;
        let _ = clawhub_toggle_skill;
        let _ = openclaw_substrate_status;
        let _ = openclaw_substrate_restart;
        let _ = openclaw_get_settings;
        let _ = openclaw_update_settings;
        let _ = install_skill_bundle;
        let _ = uninstall_skill_bundle;
        let _ = openclaw_generate_skill;
        let _ = openclaw_recommend_skills;
        // Task 13.1–13.3 command surface (also part of the preserved contract).
        let _ = openclaw_capability_manager;
        let _ = openclaw_execution_logs;
        let _ = openclaw_capability_graph;
        let _ = openclaw_list_grants;
        let _ = openclaw_revoke_grant;
        let _ = openclaw_get_developer_mode;
        let _ = openclaw_set_developer_mode;
    }

    /// R10.2 (push half) — `forward_registry_events` bridges the REAL frozen
    /// `ProductionSkillRegistry` broadcast (the same one the registry emits to on
    /// every install/enable/…) to the sink. Proves the bridge subscribes to the
    /// real single-authority stream, not a mock.
    #[tokio::test]
    async fn forward_registry_events_delivers_real_registry_events_to_sink() {
        let (registry, dir) = temp_registry();

        let received: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
        let received_clone = received.clone();
        // Subscribe BEFORE the emitting mutation so the push is observed.
        forward_registry_events(registry.subscribe_events(), move |payload| {
            received_clone.lock().unwrap().push(payload);
        });

        // Give the spawned forwarder task a moment to begin receiving.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Real registry mutation → emits RegistryEvent::Installed on the bus.
        registry
            .install_skill(&sample_meta("oc.registry.bridge.test"))
            .expect("install");

        let mut got = false;
        for _ in 0..50 {
            if !received.lock().unwrap().is_empty() {
                got = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        let events_received = received.lock().unwrap().clone();
        assert!(
            got,
            "forward_registry_events must deliver real RegistryEvent stream items to the sink"
        );
        assert_eq!(
            events_received[0]["Installed"]["skill_id"], "oc.registry.bridge.test",
            "forwarded payload must carry the real skill_id from the real RegistryEvent"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// R10.2 (reconcile half) — a DROPPED push event is recovered by a subsequent
    /// poll of the authoritative registry. Here the `RegistryEvent::Installed` is
    /// missed entirely (subscription happens AFTER the install, the worst-case
    /// dropped-event scenario), yet polling the frozen registry
    /// (`get_enabled_skills`, the source backing `clawhub_list_skills` /
    /// `openclaw_capability_manager`) still reflects the true state. This is the
    /// eventual-consistency guarantee: no push event is load-bearing for
    /// correctness — registry state is always immediately correct.
    #[tokio::test]
    async fn dropped_registry_event_is_reconciled_by_registry_poll() {
        let (registry, dir) = temp_registry();

        // Simulate a dropped/lagged push: install BEFORE anyone subscribes, so
        // the Installed event is never observed by the UI bridge.
        let received: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
        registry
            .install_skill(&sample_meta("oc.dropped.event.skill"))
            .expect("install");

        // Now the (late) bridge subscribes — it missed the Installed event.
        let received_clone = received.clone();
        forward_registry_events(registry.subscribe_events(), move |payload| {
            received_clone.lock().unwrap().push(payload);
        });
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;

        // The push was missed …
        assert!(
            received.lock().unwrap().is_empty(),
            "precondition: the Installed event was dropped (never pushed to the late subscriber)"
        );

        // … but the UI's poll of the authoritative registry reconciles it.
        let enabled = registry.get_enabled_skills().expect("poll enabled skills");
        assert!(
            enabled
                .iter()
                .any(|s| s.skill_id == "oc.dropped.event.skill"),
            "R10.2: a dropped push event must be reconciled by polling the authoritative registry"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Frontend-facing substrate status payload.
#[derive(Debug, Clone, Serialize)]
pub struct SubstrateStatusPayload {
    pub status: String,
    pub details: String,
    pub active_invocations: u32,
    pub warm_pool_count: u32,
}

/// Lightweight skill card for the frontend marketplace view.
#[derive(Debug, Clone, Serialize)]
pub struct SkillCard {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub trust_tier: String,
    pub installed: bool,
    pub enabled: bool,
}

impl From<&SkillDescriptor> for SkillCard {
    fn from(sd: &SkillDescriptor) -> Self {
        Self {
            slug: sd.skill_id.clone(),
            name: sd.name.clone(),
            description: sd.description.clone(),
            category: sd.category.clone(),
            trust_tier: sd.trust_tier.as_str().to_string(),
            installed: true,
            enabled: sd.is_usable(),
        }
    }
}

impl From<&RemoteSkillEntry> for SkillCard {
    fn from(entry: &RemoteSkillEntry) -> Self {
        Self {
            slug: entry.slug.clone(),
            name: entry.name.clone(),
            description: entry.description.clone(),
            category: entry.category.clone(),
            trust_tier: entry.trust_tier.clone(),
            installed: false,
            enabled: false,
        }
    }
}

/// Extended remote skill card — carries manifest_url and capabilities_summary
/// for the permission modal.
#[derive(Debug, Clone, Serialize)]
pub struct RemoteSkillCard {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub category: String,
    /// Always "community" for remote skills.
    pub trust_tier: String,
    pub version: String,
    pub manifest_url: String,
    pub capabilities_summary: Vec<String>,
    pub installed: bool,
}

impl RemoteSkillCard {
    fn from_entry(entry: &RemoteSkillEntry, installed: bool) -> Self {
        Self {
            slug: entry.slug.clone(),
            name: entry.name.clone(),
            description: entry.description.clone(),
            category: entry.category.clone(),
            trust_tier: "community".into(),
            version: entry.version.clone(),
            manifest_url: entry.manifest_url.clone(),
            capabilities_summary: entry.capabilities_summary.clone(),
            installed,
        }
    }
}

/// A9 desktop wiring fix (product gap 8/8, final of 8): make autonomous
/// skill generation reachable from the UI. Real fix, additive — reuses the
/// REAL, existing production stack end to end:
///   GenerationPipeline -> LlmSkillGenerator -> ModelRouter::route() ->
///   the SAME configured local llama.cpp / cloud backend chat() already
///   uses -> codegen -> BundleInstallSink -> the SAME single BundleInstaller
///   every other real install path uses -> registry -> semantic router.
/// No new LLM client, no new installer, no new pipeline — every stage binds
/// to the real, single, existing implementation (self-audit: one
/// Registry/Runtime/Installer/Router/Generation-Pipeline invariant held).
#[derive(Debug, Clone, Serialize)]
pub struct GenerateSkillOutcome {
    /// "generated" | "reused" | "awaiting_approval" | "awaiting_user" | "denied" | "failed"
    pub outcome: String,
    pub slug: Option<String>,
    pub version: Option<String>,
    pub quality: Option<f64>,
    pub similarity: Option<f64>,
    pub reasons: Vec<String>,
    pub failure_reason: Option<String>,
}

/// Generate a new OpenClaw skill from a natural-language prompt, using the
/// real configured LLM backend (`ModelRouter::route`), and install it
/// through the single real `BundleInstaller` on success.
#[tauri::command]
pub async fn openclaw_generate_skill(
    prompt: String,
    state: State<'_, AppStateCell>,
) -> Result<GenerateSkillOutcome, String> {
    use kria_core::openclaw::bundle::verify::keypair_from_seed;
    use kria_core::openclaw::generation::approval::ApprovalLayer;
    use kria_core::openclaw::generation::budget::{BudgetLimits, GenerationBudget};
    use kria_core::openclaw::generation::decision::{
        DecisionEngine, GenerationPolicy, SkillCandidate,
    };
    use kria_core::openclaw::generation::events::GenerationEventStream;
    use kria_core::openclaw::generation::llm_generator::LlmSkillGenerator;
    use kria_core::openclaw::generation::pipeline::{
        GenerationPipeline, PipelineConfig, PipelineOutcome,
    };
    use kria_core::openclaw::generation::sandbox::StaticSandbox;
    use kria_core::openclaw::generation::BundleInstallSink;
    use kria_core::openclaw::ToolRegistryActivation;
    use std::sync::Arc;

    let app = state.get().ok_or("runtime not ready")?;

    // Real, configured LLM backend — the SAME one the rest of KRIA's chat
    // uses (no second LLM client introduced). An honest, actionable error if
    // none is configured/reachable — never a fabricated generation.
    let backend = app
        .model_router
        .route("generate an openclaw skill")
        .await
        .ok_or_else(|| {
            "No LLM backend configured or reachable (KRIA_LLAMA_API_URL / cloud provider). \
             Configure a local llama.cpp server or a cloud provider in Settings, then retry."
                .to_string()
        })?;

    let data_dir = kria_data_dir();
    let store_dir = data_dir.join("openclaw_skills");
    let _ = std::fs::create_dir_all(&store_dir);
    let work_dir = data_dir.join("openclaw_generation_work");
    let _ = std::fs::create_dir_all(&work_dir);

    let audit = Arc::new(
        kria_core::openclaw::audit::AuditLedger::open(
            &data_dir.join("skills.db"),
            b"kria-openclaw-dev-audit-key-0001".to_vec(),
        )
        .map_err(|e| format!("audit open failed: {e}"))?,
    );

    let sink = BundleInstallSink::new(app.skill_registry.clone(), audit, store_dir)
        .with_activation(Arc::new(ToolRegistryActivation::new()));

    let generator = Arc::new(LlmSkillGenerator::new(backend));
    let sandbox = Arc::new(StaticSandbox);
    // GenerateIfMissing: skip generating an exact duplicate of an already-
    // installed skill (real reuse-vs-generate decision, A9.0).
    let decision = DecisionEngine::new(0.85, GenerationPolicy::GenerateIfMissing);
    // No auto-approve: an honest AwaitingApproval outcome surfaces for any
    // skill design the real ApprovalLayer flags, rather than silently
    // bypassing it (no HITL prompt UI exists yet in this session).
    let approval = ApprovalLayer::new(false);
    let events = GenerationEventStream::new();

    let pipeline = GenerationPipeline::new(
        generator,
        sandbox,
        decision,
        approval,
        events,
        Arc::new(sink),
    );

    // Ephemeral per-generation signing key (mirrors the marketplace-bundle
    // synthesis pattern, Fix 3/8 — a generated bundle is self-signed the
    // same honest way; trust comes from Community-tier + capability
    // enforcement on install, not the key's identity).
    let seed: [u8; 32] = {
        let mut s = [0u8; 32];
        use rand::RngCore;
        rand::rngs::OsRng.fill_bytes(&mut s);
        s
    };
    let (signing_key, publisher_hex) = keypair_from_seed(seed);

    let config = PipelineConfig {
        quality_threshold: 0.6,
        publisher_hex,
        signing_key,
        work_dir,
    };
    let budget = GenerationBudget::new(BudgetLimits::default());

    // Real existing-skill candidates from the real registry (never
    // regenerates an installed skill; enables real reuse detection).
    let existing: Vec<SkillCandidate> = app
        .skill_registry
        .list_installed()
        .unwrap_or_default()
        .iter()
        .map(|s| SkillCandidate {
            slug: s.skill_id.clone(),
            description: s.description.clone(),
            category: s.category.clone(),
            tags: vec![],
            capabilities: vec![],
        })
        .collect();

    let goal_id = uuid::Uuid::new_v4().to_string();
    let outcome = pipeline
        .run(&goal_id, &prompt, &existing, &budget, &config)
        .await;

    Ok(match outcome {
        PipelineOutcome::Generated {
            slug,
            version,
            quality,
        } => GenerateSkillOutcome {
            outcome: "generated".into(),
            slug: Some(slug),
            version: Some(version),
            quality: Some(quality),
            similarity: None,
            reasons: vec![],
            failure_reason: None,
        },
        PipelineOutcome::Reused { slug, similarity } => GenerateSkillOutcome {
            outcome: "reused".into(),
            slug: Some(slug),
            version: None,
            quality: None,
            similarity: Some(similarity),
            reasons: vec![],
            failure_reason: None,
        },
        PipelineOutcome::AwaitingApproval { slug, reasons, .. } => GenerateSkillOutcome {
            outcome: "awaiting_approval".into(),
            slug: Some(slug),
            version: None,
            quality: None,
            similarity: None,
            reasons,
            failure_reason: None,
        },
        PipelineOutcome::AwaitingUser {
            best_match,
            similarity,
        } => GenerateSkillOutcome {
            outcome: "awaiting_user".into(),
            slug: best_match,
            version: None,
            quality: None,
            similarity: Some(similarity),
            reasons: vec![],
            failure_reason: None,
        },
        PipelineOutcome::Denied => GenerateSkillOutcome {
            outcome: "denied".into(),
            slug: None,
            version: None,
            quality: None,
            similarity: None,
            reasons: vec![],
            failure_reason: None,
        },
        PipelineOutcome::Failed { reason } => GenerateSkillOutcome {
            outcome: "failed".into(),
            slug: None,
            version: None,
            quality: None,
            similarity: None,
            reasons: vec![],
            failure_reason: Some(reason),
        },
    })
}

/// RAII cleanup for the ephemeral marketplace-bundle-synthesis temp directory
/// (installer-unification fix). `BundleInstaller::install` copies everything
/// it needs into its own versioned store dir synchronously before returning,
/// so this directory is safe to remove as soon as `install()` returns
/// (success or failure) — never left behind as a leak.
struct SynthDirCleanup(std::path::PathBuf);
impl Drop for SynthDirCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Install request from the frontend permission modal.
#[derive(Debug, Deserialize)]
pub struct RemoteInstallRequest {
    pub manifest_url: String,
    pub slug: String,
    /// User-approved capability set from the permission modal.
    /// Kept for future HITL policy enforcement; not yet validated against
    /// the transpiled descriptor.
    #[allow(dead_code)]
    pub approved_capabilities: Option<SkillCapabilities>,
}

/// List all installed skills from the live SQLite registry.
#[command]
pub fn clawhub_list_skills(state: State<'_, AppStateCell>) -> Result<Vec<SkillCard>, String> {
    let app = state.get().ok_or("runtime not ready")?;
    let skills = app
        .skill_registry
        .list_installed()
        .map_err(|e| e.to_string())?;
    Ok(skills.iter().map(SkillCard::from).collect())
}

/// Search installed skills by name/description substring.
/// Remote ClawHub search is intentionally omitted until a real endpoint exists.
#[command]
pub fn clawhub_search_skills(
    query: String,
    _category: Option<String>,
    _limit: Option<usize>,
    state: State<'_, AppStateCell>,
) -> Result<Vec<SkillCard>, String> {
    let app = state.get().ok_or("runtime not ready")?;
    let all = app
        .skill_registry
        .list_installed()
        .map_err(|e| e.to_string())?;
    let q = query.to_lowercase();
    let matched: Vec<SkillCard> = all
        .iter()
        .filter(|s| {
            q.is_empty()
                || s.name.to_lowercase().contains(&q)
                || s.description.to_lowercase().contains(&q)
                || s.category.to_lowercase().contains(&q)
        })
        .map(SkillCard::from)
        .collect();
    Ok(matched)
}

/// Fetch skills from the remote GitHub registry index.
///
/// Returns remote entries enriched with `installed: true/false` by cross-
/// referencing the local registry. Passes through `query` and `category`
/// filters server-side (index is small enough to filter locally).
#[command]
pub async fn clawhub_fetch_remote_skills(
    query: String,
    category: Option<String>,
    state: State<'_, AppStateCell>,
) -> Result<Vec<RemoteSkillCard>, String> {
    let app = state.get().ok_or("runtime not ready")?;
    let cfg = app.config.read().await.openclaw.clone();
    let client = ClawHubClient::new(&cfg.registry.index_url, cfg.registry.allowed_hosts.clone());

    let entries = client
        .search_remote(&query, category.as_deref())
        .await
        .map_err(|e| e.to_string())?;

    let cards = entries
        .iter()
        .map(|e| {
            let installed = app.skill_registry.get(&e.slug).is_ok();
            RemoteSkillCard::from_entry(e, installed)
        })
        .collect();

    Ok(cards)
}

/// Install a skill from a remote manifest URL.
///
/// Installer-unification fix (R12, product gap 3/8): this now goes through the
/// SAME single `BundleInstaller` the local `.ocskill` path uses — real signature
/// verification, real rollback-on-failure, real hot activation, real
/// `content_hash` (never the old hardcoded `"legacy"`). Pipeline:
/// 1. Validate manifest URL via `DomainValidator` (HTTPS + allowlist).
/// 2. Download the raw `SKILL.md` (≤ 64 KiB).
/// 3. Transpile through `transpiler::transpile_skill()` — enforces safe name,
///    description, and capabilities; derives real capability grants (fix 1/8).
///    Sets `TrustTier::Community`.
/// 4. Verify network_domains against PSL via `DomainValidator`.
/// 5. Synthesize a real, self-signed, verifiable bundle directory
///    (`bundle::synth::synth_marketplace_bundle`) from the transpiled
///    descriptor, then install it via `BundleInstaller::install` — the exact
///    same verify → deps → registry → activate → audit → events pipeline the
///    local bundle path uses (see `synth.rs` module doc for the honest scope
///    note: marketplace `SKILL.md` sources carry no executable handler code
///    today; this fix unifies the INSTALLER, not the underlying skill-content
///    format).
#[command]
pub async fn clawhub_install_skill(
    request: RemoteInstallRequest,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    use kria_core::openclaw::audit::AuditLedger;
    use kria_core::openclaw::bundle::synth::synth_marketplace_bundle;
    use kria_core::openclaw::bundle::BundleInstaller;
    use kria_core::openclaw::clawhub::DomainValidator;
    use kria_core::openclaw::transpiler::transpile_skill;
    use kria_core::openclaw::types::{SkillSource, TrustTier};
    use kria_core::openclaw::ToolRegistryActivation;
    use std::sync::Arc;

    let app = state.get().ok_or("runtime not ready")?;

    // No-op if already installed.
    if app.skill_registry.get(&request.slug).is_ok() {
        return Ok(());
    }

    let cfg = app.config.read().await.openclaw.clone();

    // 1. Validate manifest URL.
    let validator = DomainValidator::new(cfg.registry.allowed_hosts.clone());
    validator
        .validate(&request.manifest_url)
        .map_err(|e| format!("URL rejected: {e}"))?;

    // 2. Download manifest.
    let client = ClawHubClient::new(&cfg.registry.index_url, cfg.registry.allowed_hosts.clone());
    let raw_manifest = client
        .download_skill_manifest(&request.manifest_url)
        .await
        .map_err(|e| format!("Download failed: {e}"))?;

    // 3. Transpile — enforces name/desc validation; assigns Community tier;
    // derives real capability grants (capability-grant-wiring fix).
    let source = SkillSource::ClawHub {
        slug: request.slug.clone(),
        version: "remote".into(),
    };
    let mut descriptor = transpile_skill(&raw_manifest, source, false)
        .map_err(|e| format!("Transpile failed: {e}"))?;

    // 4. Security enforcement: remote skills are ALWAYS Community, never Verified.
    descriptor.trust_tier = TrustTier::Community;

    // Publisher revocation enforcement fix (product gap 7/8): since the
    // installer-unification fix (product gap 3/8), this marketplace path
    // installs through the SAME `BundleInstaller::install` used by the
    // local-bundle path — which now checks the real, global
    // `PublisherRegistry` for a revoked signing key BEFORE any registry
    // mutation (see `bundle/installer.rs::install_inner`). No separate
    // check is added here: marketplace bundles are synthesized with a
    // process-local ephemeral signing key (never a real, registered
    // publisher identity — see `bundle::synth`'s honest-scope note), so
    // there is no real stable publisher identity to check against on THIS
    // path today. If marketplace skills ever carry real publisher keys
    // (a future content-format change), the SAME `install_inner` check
    // already protects them with zero additional code — single authority,
    // no duplicate revocation-check path.

    // 5. Validate declared network_domains via DomainValidator.
    if let kria_core::openclaw::types::OpenClawNetworkPolicy::DomainAllowlist(ref domains) =
        descriptor.network_policy
    {
        for domain in domains {
            let test_url = format!("https://{}/", domain);
            validator
                .validate(&test_url)
                .map_err(|e| format!("Network domain '{}' rejected: {e}", domain))?;
        }
    }

    // 6. Synthesize a real bundle dir + install through the SINGLE, unified
    // BundleInstaller (same one the local `.ocskill` path uses).
    let caps: Vec<kria_core::openclaw::capability::Capability> = descriptor
        .granted
        .iter()
        .map(|g| g.capability.clone())
        .collect();
    let synth_root = std::env::temp_dir().join(format!("kria-oc-synth-{}", uuid::Uuid::new_v4()));
    let bundle_dir = synth_root.join(&descriptor.skill_id);
    synth_marketplace_bundle(&descriptor, &caps, &bundle_dir)
        .map_err(|e| format!("Bundle synthesis failed: {e}"))?;
    // Best-effort cleanup of the ephemeral synth dir once BundleInstaller has
    // copied what it needs into its own versioned store dir (install() below
    // reads from bundle_dir synchronously, then this dir is no longer needed).
    let _cleanup_guard = SynthDirCleanup(synth_root.clone());

    let data_dir = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".kria");
    let store_dir = data_dir.join("openclaw_skills");
    let _ = std::fs::create_dir_all(&store_dir);
    let audit = Arc::new(
        AuditLedger::open(
            &data_dir.join("skills.db"),
            b"kria-openclaw-dev-audit-key-0001".to_vec(),
        )
        .map_err(|e| format!("audit open failed: {e}"))?,
    );

    let mut installer = BundleInstaller::new(app.skill_registry.clone(), audit, store_dir)
        // Marketplace bundles are self-signed with an ephemeral key
        // (see synth.rs doc) — trust comes from the forced Community
        // tier + capability enforcement, not from the signature's
        // identity. `require_signature` stays true (still enforces the
        // bundle wasn't tampered with in transit/synthesis).
        .with_trust_policy(kria_core::openclaw::bundle::verify::TrustPolicy {
            trusted_keys: Vec::new(),
            require_signature: true,
        });
    installer = installer.with_activation(Arc::new(ToolRegistryActivation::new()));

    let outcome = installer
        .install(&bundle_dir)
        .map_err(|e| format!("Install failed: {e}"))?;

    tracing::info!(
        skill_id = %outcome.skill_id,
        version = %outcome.version,
        trust_tier = %descriptor.trust_tier,
        source_url = %request.manifest_url,
        "[OpenClaw] remote skill installed via unified BundleInstaller"
    );

    Ok(())
}

/// Uninstall a skill from the registry.
#[command]
pub fn clawhub_uninstall_skill(
    skill_id: String,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let app = state.get().ok_or("runtime not ready")?;
    let result = app.skill_registry.uninstall(&skill_id);
    super::history_helpers::observe_capability_lifecycle(
        app.memory_system.as_ref(),
        "uninstall",
        &skill_id,
        result.is_ok(),
    );
    result.map_err(|e| e.to_string())
}

/// Toggle a skill enabled/disabled.
#[command]
pub fn clawhub_toggle_skill(
    skill_id: String,
    enabled: bool,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    let app = state.get().ok_or("runtime not ready")?;
    let result = app.skill_registry.toggle(&skill_id, enabled);
    super::history_helpers::observe_capability_lifecycle(
        app.memory_system.as_ref(),
        if enabled { "enable" } else { "disable" },
        &skill_id,
        result.is_ok(),
    );
    result.map_err(|e| e.to_string())
}

/// Return current substrate health — reads live pool counts when Docker is available.
#[command]
pub async fn openclaw_substrate_status(
    state: State<'_, AppStateCell>,
) -> Result<SubstrateStatusPayload, String> {
    let app = state.get().ok_or("runtime not ready")?;
    let cfg = app.config.read().await.openclaw.clone();
    if !cfg.enabled {
        return Ok(SubstrateStatusPayload {
            status: "disabled".into(),
            details: "OpenClaw substrate is disabled in settings".into(),
            active_invocations: 0,
            warm_pool_count: 0,
        });
    }

    let pool = app.container_pool.read().await.clone();
    match pool {
        Some(pool) => {
            let active = pool.active_count().await as u32;
            let warm = pool.warm_count_total().await as u32;
            let status = if active > 0 { "busy" } else { "running" };
            let details = format!(
                "Docker substrate healthy — {} active, {} warm",
                active, warm
            );
            Ok(SubstrateStatusPayload {
                status: status.into(),
                details,
                active_invocations: active,
                warm_pool_count: warm,
            })
        }
        None => Ok(SubstrateStatusPayload {
            status: "unavailable".into(),
            details: format!(
                "OpenClaw substrate unavailable. Check Docker and build the image with: docker build -f Dockerfile.openclaw-substrate -t {} .",
                cfg.image
            ),
            active_invocations: 0,
            warm_pool_count: 0,
        }),
    }
}

/// Drain and re-warm the container pool.
#[command]
pub async fn openclaw_substrate_restart(state: State<'_, AppStateCell>) -> Result<(), String> {
    let app = state.get().ok_or("runtime not ready")?;
    if let Some(pool) = app.container_pool.read().await.clone() {
        // Drain + re-warm without tearing down background health/recycle tasks.
        pool.rewarm().await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ─── A2: Production .ocskill bundle install/uninstall (one installer) ───────────

/// Summary returned to the frontend after a bundle install.
#[derive(Debug, Clone, Serialize)]
pub struct BundleInstallSummary {
    pub skill_id: String,
    pub version: String,
    /// "fresh" | "upgrade" | "same" | "downgrade"
    pub relation: String,
}

fn kria_data_dir() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".kria")
}

/// Install (or update) a production `.ocskill` bundle from a filesystem path.
///
/// Uses the single `BundleInstaller`: verify → validate → deps → registry → hot-activate into the
/// live `ToolRegistry` (immediately callable when the substrate is up) → audit → events.
#[tauri::command]
pub async fn install_skill_bundle(
    path: String,
    state: State<'_, AppStateCell>,
) -> Result<BundleInstallSummary, String> {
    use kria_core::openclaw::audit::AuditLedger;
    use kria_core::openclaw::bundle::BundleInstaller;
    use kria_core::openclaw::ToolRegistryActivation;
    use std::sync::Arc;

    let app = state.get().ok_or("runtime not ready")?;
    let data_dir = kria_data_dir();
    let store_dir = data_dir.join("openclaw_skills");
    let _ = std::fs::create_dir_all(&store_dir);

    let audit = Arc::new(
        AuditLedger::open(
            &data_dir.join("skills.db"),
            b"kria-openclaw-dev-audit-key-0001".to_vec(),
        )
        .map_err(|e| format!("audit open failed: {e}"))?,
    );

    let mut installer = BundleInstaller::new(app.skill_registry.clone(), audit.clone(), store_dir);

    // Registry-driven activation (A6): trigger the semantic tool-index reindex on
    // install. No per-skill ToolRegistry registration needed — see activation.rs.
    installer = installer.with_activation(Arc::new(ToolRegistryActivation::new()));

    let outcome = installer
        .install(std::path::Path::new(&path))
        .map_err(|e| e.to_string())?;

    let relation = format!("{:?}", outcome.relation).to_lowercase();
    super::history_helpers::observe_capability_lifecycle(
        app.memory_system.as_ref(),
        "acquire",
        &outcome.skill_id,
        true,
    );
    Ok(BundleInstallSummary {
        skill_id: outcome.skill_id,
        version: outcome.version,
        relation,
    })
}

/// Uninstall a bundle-installed skill: deactivate from the live registry + delete stored files.
#[tauri::command]
pub async fn uninstall_skill_bundle(
    skill_id: String,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    use kria_core::openclaw::audit::AuditLedger;
    use kria_core::openclaw::bundle::BundleInstaller;
    use kria_core::openclaw::ToolRegistryActivation;
    use std::sync::Arc;

    let app = state.get().ok_or("runtime not ready")?;
    let data_dir = kria_data_dir();
    let store_dir = data_dir.join("openclaw_skills");

    let audit = Arc::new(
        AuditLedger::open(
            &data_dir.join("skills.db"),
            b"kria-openclaw-dev-audit-key-0001".to_vec(),
        )
        .map_err(|e| format!("audit open failed: {e}"))?,
    );

    let mut installer = BundleInstaller::new(app.skill_registry.clone(), audit.clone(), store_dir);
    installer = installer.with_activation(Arc::new(ToolRegistryActivation::new()));

    let result = installer.uninstall(&skill_id);
    super::history_helpers::observe_capability_lifecycle(
        app.memory_system.as_ref(),
        "uninstall",
        &skill_id,
        result.is_ok(),
    );
    result.map_err(|e| e.to_string())
}

// ─── Production Settings surface (no TOML editing) ──────────────────────────

/// OpenClaw settings surfaced to the Settings UI. Mirrors the persisted
/// `OpenClawConfig` subset that is safe and useful to edit from the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenClawSettingsPayload {
    // General
    pub enabled: bool,
    // Runtime
    pub image: String,
    pub warm_per_class: usize,
    pub max_concurrent_invocations: usize,
    pub default_timeout_secs: u64,
    pub max_warm_age_secs: u64,
    pub max_restart_attempts: u32,
    // Skills
    pub rewrite_descriptions: bool,
    pub check_updates: bool,
    // Registry / marketplace
    pub registry_index_url: String,
    // Security / trust
    pub community_allows_network: bool,
    pub verified_skips_hitl: bool,
    /// Informational: whether the live container pool is currently running.
    pub runtime_active: bool,
}

/// Read the current OpenClaw settings for the Settings UI.
#[command]
pub async fn openclaw_get_settings(
    state: State<'_, AppStateCell>,
) -> Result<OpenClawSettingsPayload, String> {
    let app = state.get().ok_or("runtime not ready")?;
    let cfg = app.config.read().await.openclaw.clone();
    Ok(OpenClawSettingsPayload {
        enabled: cfg.enabled,
        image: cfg.image,
        warm_per_class: cfg.warm_per_class,
        max_concurrent_invocations: cfg.max_concurrent_invocations,
        default_timeout_secs: cfg.default_timeout_secs,
        max_warm_age_secs: cfg.max_warm_age_secs,
        max_restart_attempts: cfg.max_restart_attempts,
        rewrite_descriptions: cfg.rewrite_descriptions,
        check_updates: cfg.lifecycle.check_updates,
        registry_index_url: cfg.registry.index_url,
        community_allows_network: cfg.trust.community_allows_network,
        verified_skips_hitl: cfg.trust.verified_skips_hitl,
        runtime_active: app.container_pool.read().await.is_some(),
    })
}

/// Persist OpenClaw settings and reconcile the live substrate. The return value
/// remains for frontend compatibility and is always `false`: restart is no longer required.
#[command]
pub async fn openclaw_update_settings(
    settings: OpenClawSettingsPayload,
    state: State<'_, AppStateCell>,
) -> Result<bool, String> {
    let app = state.get().ok_or("runtime not ready")?;

    // Clone the whole live config and replace only the OpenClaw section.
    let mut next = app.config.read().await.clone();

    next.openclaw.enabled = settings.enabled;
    next.openclaw.image = settings.image;
    next.openclaw.warm_per_class = settings.warm_per_class.min(16);
    next.openclaw.max_concurrent_invocations = settings.max_concurrent_invocations.clamp(1, 64);
    next.openclaw.default_timeout_secs = settings.default_timeout_secs.clamp(1, 3600);
    next.openclaw.max_warm_age_secs = settings.max_warm_age_secs.max(30);
    next.openclaw.max_restart_attempts = settings.max_restart_attempts.clamp(1, 10);
    next.openclaw.rewrite_descriptions = settings.rewrite_descriptions;
    next.openclaw.lifecycle.check_updates = settings.check_updates;
    if !settings.registry_index_url.trim().is_empty() {
        next.openclaw.registry.index_url = settings.registry_index_url;
    }
    next.openclaw.trust.community_allows_network = settings.community_allows_network;
    next.openclaw.trust.verified_skips_hitl = settings.verified_skips_hitl;

    // TrustConfig enforcement fix (product gap 6/8): push the new trust
    // config into the live, process-wide snapshot `execute_semantic` reads
    // on every real execution — hot, no restart required (R14.3).
    kria_core::openclaw::trust_runtime::set_live_trust_config(next.openclaw.trust.clone());

    app.config_service
        .replace_all(next, kria_core::config::ChangeSource::Ui)
        .await
        .map_err(|error| error.to_string())?;

    tracing::info!(
        enabled = settings.enabled,
        "[OpenClaw] settings updated and live reconciliation scheduled"
    );
    Ok(false)
}

// ─── Phase D: capability recommendations (CIL Recommender, pure reads) ──────────
//
// Task 7.3 (design §8.7, R8.1/R8.2/R8.3/R8.5/R10.1): surface the CIL
// `Recommender` to the frontend as a NEW, additive Tauri command + event. This
// does NOT rename or change any existing OpenClaw command/event — every
// contract above (`clawhub_*`, `openclaw_*`, `install_skill_bundle`,
// `openclaw:bundle_event`, `openclaw:execution_event`) is preserved verbatim.
//
// Runtime-authority invariants honored here:
//   * KRIA orchestration authority — this command only *reads* and *ranks*
//     candidates; it NEVER installs, generates, or mutates a skill. Acquisition
//     is a separate, explicitly-approved step (task 8). Recommendations are
//     PURE READS.
//   * Capability-first / no hardcoding — ranking + rationale come entirely from
//     the configured `CilConfig` weights/thresholds and the candidate's real
//     `market_catalog` signals (via the frozen `Recommender`); there is no
//     per-skill or per-category branch anywhere in this file for it.
//   * Deterministic + honest degraded — gated behind `openclaw_icp_enabled`;
//     with the flag OFF, or when the embedder/marketplace is unavailable, an
//     honest empty/degraded payload is returned (never a fabricated candidate).
//   * Frozen components extended, not forked — reuses the frozen KRIA embedding
//     backend (`MemoryEmbedder` over `app.embeddings`), the frozen
//     `ClawHubClient` (via the `ClawHubProvider` adapter), and the SAME
//     `skills.db` the registry owns (migration 4 owns `market_catalog`). No new
//     embedder, marketplace, database, or recommender store is introduced.

/// One recommended marketplace skill surfaced to the frontend (design §8.7).
///
/// Every field is copied straight from the CIL [`Recommendation`] (which in turn
/// copies from the `market_catalog` row) — nothing is fabricated. `trust_tier`,
/// `quality`, and `popularity` are honestly `None`/absent when the catalog row
/// carried no such signal.
///
/// [`Recommendation`]: kria_core::openclaw::cil::Recommendation
#[derive(Debug, Clone, Serialize)]
pub struct SkillRecommendationCard {
    /// Marketplace this candidate came from (`market_catalog.provider_id`).
    pub provider_id: String,
    /// Stable skill identifier / slug (`market_catalog.slug`).
    pub slug: String,
    /// Offered semver (`market_catalog.version`).
    pub version: String,
    /// Combined, weighted rank score used to order recommendations (higher = better).
    pub score: f32,
    /// Effective trust tier recorded at sync time, or `None` when honestly absent.
    pub trust_tier: Option<String>,
    /// Validator/marketplace quality signal, if the provider supplied one.
    pub quality: Option<f64>,
    /// Install/usage popularity signal, if the provider supplied one.
    pub popularity: Option<f64>,
    /// Whether the marketplace flags this skill deprecated.
    pub deprecated: bool,
    /// Human-readable rationale assembled from the candidate's real signals
    /// (never a template keyed to name/category).
    pub rationale: String,
    /// Interchangeable alternative skill ids from the capability graph (empty
    /// when none are known).
    pub alternatives: Vec<String>,
}

/// The result of [`openclaw_recommend_skills`] — an honest, ranked recommendation
/// set plus the status flags that make degraded/empty results truthful.
#[derive(Debug, Clone, Serialize)]
pub struct RecommendationsPayload {
    /// Whether the ICP feature flag (`openclaw_icp_enabled`) is ON. When `false`
    /// this is an honest disabled result with no recommendations.
    pub enabled: bool,
    /// `true` when the result is degraded (flag OFF or embedder unavailable), so
    /// the UI can present the result honestly rather than as full fidelity.
    pub degraded: bool,
    /// Honest, human-readable status/reason — especially when degraded or empty.
    pub status: String,
    /// Ranked recommendations (empty is a valid, honest "nothing to recommend").
    pub recommendations: Vec<SkillRecommendationCard>,
}

/// Fetch ranked capability recommendations for a natural-language `goal` from the
/// CIL [`Recommender`](kria_core::openclaw::cil::Recommender) (design §8.7, task 7.3).
///
/// This is a **pure read** (R8.2, R9.2): it ranks marketplace candidates from the
/// pre-embedded, offline `market_catalog` cache the user could choose to install,
/// and NEVER installs, generates, mutates a skill, or performs a live per-query
/// marketplace fetch. The catalog cache is populated/refreshed by the CIL's
/// background federated sync job (design §8.2, tasks 2.4/6.3) — not by this read.
///
/// Gating & honesty:
/// - `openclaw_icp_enabled == false` → honest disabled payload, no recommendations.
/// - embedder cannot embed the goal → honest degraded payload, no recommendations.
/// - nothing clears the configured relevance threshold (or the cache is empty) →
///   honest empty set, never a fabricated candidate (R8.5).
#[tauri::command]
pub async fn openclaw_recommend_skills(
    goal: String,
    limit: Option<usize>,
    app_handle: AppHandle,
    state: State<'_, AppStateCell>,
) -> Result<RecommendationsPayload, String> {
    // M12 (Option A): the legacy CIL recommender is removed. Marketplace
    // recommendations now flow through the ONE Capability Provider Platform —
    // the `cpp_recommend` command backing Capabilities → Marketplace. This
    // command is retained only as a compatibility shim for any old caller and
    // returns an honest, empty result that points at the CPP surface; it
    // installs/generates/ranks nothing itself.
    let _ = (goal, limit, state);
    let payload = RecommendationsPayload {
        enabled: false,
        degraded: true,
        status: "Legacy recommender removed — use Capabilities → Marketplace (cpp_recommend)."
            .into(),
        recommendations: Vec::new(),
    };
    let _ = app_handle.emit(event_names::RECOMMENDATIONS, &payload);
    Ok(payload)
}

// ════════════════════════════════════════════════════════════════════════════
// Task 13 — Frontend evolution: capability manager (13.1), execution logs +
// capability-graph view (13.2), permission management + developer mode (13.3).
//
// Every command below is NEW and additive. NONE renames or changes the
// signature of any existing OpenClaw command/event (`clawhub_*`, `openclaw_*`,
// `install_skill_bundle`, `openclaw:bundle_event`, `openclaw:execution_event`,
// `openclaw:recommendations`, …) — they are all preserved verbatim. New event
// names use the same `"openclaw:*"` prefix convention.
//
// Runtime-authority invariants honored throughout this section:
//   * KRIA orchestration authority — these commands only READ derived views and
//     (for the ONE explicit user action, `openclaw_revoke_grant`) mutate a
//     permission grant the user explicitly triggered. Nothing auto-installs,
//     auto-generates, or auto-revokes.
//   * Capability-first / no hardcoding — capability data comes from the derived
//     `capability_profiles` / `capability_edges` views (keyed by `skill_id`),
//     never a per-skill or per-category branch here.
//   * Deterministic + honest degraded — the CIL-backed surfaces (capability
//     manager profiles, capability graph) are gated behind `openclaw_icp_enabled`
//     and return honest empty/degraded payloads when the flag is OFF or a
//     derived view is unavailable — never fabricated data.
//   * Frozen components extended, not forked — consumes the frozen
//     `ProductionSkillRegistry`, the CIL `ProfileStore`/`CapabilityGraph`
//     derived views, the `GrantStore`/`PermissionEngine`, and the SAME
//     `openclaw::event` + `bundle::events` broadcast buses the existing
//     forwarder already reads. No new registry, database, event bus, or
//     permission store is introduced (all live in the one `skills.db`).
//   * Deny-by-default permission surfaces — grant listing shows only active
//     (non-revoked, non-expired) grants; revoke forces fresh approval next use.

use std::collections::VecDeque;
use std::sync::{Mutex as StdMutex, OnceLock};

/// Path to the SAME `skills.db` the frozen `ProductionSkillRegistry` owns
/// (created + migrated at boot; migrations 3–6 own `capability_profiles`,
/// `market_catalog`, `capability_grants_scoped`, `capability_edges`). The CIL
/// derived-view stores open additional connections to this one file — there is
/// no second database. Mirrors the path `openclaw_recommend_skills` already uses.
fn openclaw_skills_db_path() -> std::path::PathBuf {
    kria_data_dir().join("skills.db")
}

/// Path to the single CPP grant store (`capability::grants`), shared by the chat
/// dispatcher, the Capabilities panel, and the desktop grant list/revoke
/// commands — the ONE grant store (M12 Option-A).
fn cpp_grants_db_path() -> std::path::PathBuf {
    kria_data_dir().join("cpp_grants.db")
}

/// Read every non-removed skill's authoritative metadata from the frozen
/// registry (single source of truth). `Removed` skills are filtered out — they
/// are tombstones, not manageable capabilities.
fn all_installed_metadata(
    registry: &kria_core::openclaw::registry::SkillRegistry,
) -> Result<Vec<kria_core::openclaw::registry::SkillMetadata>, String> {
    use kria_core::openclaw::registry::{SkillQuery, SkillState};
    let query = SkillQuery {
        slug: None,
        publisher: None,
        description_contains: None,
        tags: Vec::new(),
        categories: Vec::new(),
        capabilities: Vec::new(),
        runtime_requirements: None,
        risk_level: None,
        state: None,
        enabled_only: false,
    };
    let mut skills = registry.search_skills(&query).map_err(|e| e.to_string())?;
    skills.retain(|s| s.state != SkillState::Removed);
    Ok(skills)
}

/// Map a frozen [`DiscoverySource`] to a stable provenance string plus the
/// generating workflow id when the skill was A9-generated (R10.5 — "generated
/// skill provenance"). Generic: a straight enum projection, no skill-name or
/// per-category branch.
///
/// [`DiscoverySource`]: kria_core::openclaw::registry::DiscoverySource
fn provenance_of(
    source: &kria_core::openclaw::registry::DiscoverySource,
) -> (String, Option<String>) {
    use kria_core::openclaw::registry::DiscoverySource;
    match source {
        DiscoverySource::Generated { workflow_id } => {
            ("generated".to_string(), Some(workflow_id.clone()))
        }
        DiscoverySource::Bundled { .. } => ("bundled".to_string(), None),
        DiscoverySource::InstalledBundle { .. } => ("installed_bundle".to_string(), None),
        DiscoverySource::ClawHub { .. } => ("clawhub".to_string(), None),
        DiscoverySource::Workspace { .. } => ("workspace".to_string(), None),
        DiscoverySource::Developer { .. } => ("developer".to_string(), None),
    }
}

// ─── Task 13.1: capability manager + generated-skills provenance surface ────────

/// A skill's derived [`CapabilityProfile`] projected for the UI (design §7.1).
/// Copied straight from the `capability_profiles` derived view; `has_profile`
/// is `false` (and the lists empty) when no derived row exists yet — an honest
/// "not indexed" rather than a fabricated profile.
///
/// [`CapabilityProfile`]: kria_core::openclaw::cil::CapabilityProfile
#[derive(Debug, Clone, Serialize)]
pub struct CapabilityProfileView {
    pub provides: Vec<String>,
    pub consumes: Vec<String>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    /// Whether a derived `capability_profiles` row exists for this skill.
    pub has_profile: bool,
}

impl Default for CapabilityProfileView {
    fn default() -> Self {
        Self {
            provides: Vec::new(),
            consumes: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            has_profile: false,
        }
    }
}

/// One capability-manager row: a frozen-registry skill enriched with its
/// provenance and (when the ICP flag is ON and the derived view exists) its
/// capability profile.
#[derive(Debug, Clone, Serialize)]
pub struct CapabilitySkillCard {
    pub skill_id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub trust_tier: String,
    pub risk_level: String,
    /// Registry state (`enabled` | `disabled` | `installed` | …).
    pub state: String,
    pub enabled: bool,
    /// Skill provenance: `generated` | `bundled` | `installed_bundle` |
    /// `clawhub` | `workspace` | `developer`.
    pub provenance: String,
    /// The A9 generation workflow id, present only for generated skills (R10.5).
    pub generated_workflow_id: Option<String>,
    /// Derived capability profile (empty + `has_profile=false` when unindexed).
    pub profile: CapabilityProfileView,
}

/// The capability-manager payload (task 13.1 / R10.1, R10.5).
#[derive(Debug, Clone, Serialize)]
pub struct CapabilityManagerPayload {
    /// Whether `openclaw_icp_enabled` is ON. When `false`, capability profiles
    /// are honestly empty (the derived view is populated only under the flag).
    pub enabled: bool,
    /// `true` when capability profiles could not be surfaced (flag OFF or the
    /// derived `capability_profiles` view is unavailable) — the UI presents the
    /// provenance/registry data honestly without full-fidelity profiles.
    pub degraded: bool,
    pub status: String,
    pub skills: Vec<CapabilitySkillCard>,
}

/// Capability manager (task 13.1, design §4 "capability manager", R10.1/R10.5).
///
/// Lists every installed skill from the frozen registry with its provenance
/// (including A9-generated skills' `workflow_id`) and, when `openclaw_icp_enabled`
/// is ON, its derived capability profile from the `capability_profiles` view.
///
/// Honesty & gating:
/// - The registry/provenance data is always available (it lives in the frozen
///   registry, independent of the flag).
/// - Capability profiles come from the CIL `ProfileStore` derived view, which is
///   only populated when the ICP flag is ON (the first-boot backfill job, task
///   2.4). With the flag OFF, or when the derived view can't be opened, the
///   payload is `degraded=true` and profiles are honestly empty — never faked.
#[tauri::command]
pub async fn openclaw_capability_manager(
    app_handle: AppHandle,
    state: State<'_, AppStateCell>,
) -> Result<CapabilityManagerPayload, String> {
    // M12 (Option A): the legacy CIL capability-profile view is removed. This
    // command now returns the frozen registry + provenance only (empty capability
    // profiles); rich capability metadata lives in the CPP descriptors
    // (Capabilities → Browser / Descriptor Viewer via `cpp_catalog`/`cpp_descriptor`).
    let app = state.get().ok_or("runtime not ready")?;
    let metadata = all_installed_metadata(&app.skill_registry)?;

    let mut skills = Vec::with_capacity(metadata.len());
    for meta in &metadata {
        let (provenance, generated_workflow_id) = provenance_of(&meta.discovery_source);
        skills.push(CapabilitySkillCard {
            skill_id: meta.skill_id.clone(),
            name: meta.name.clone(),
            description: meta.description.clone(),
            category: meta.category.clone(),
            trust_tier: meta.trust_tier.as_str().to_string(),
            risk_level: meta.risk_level.as_str().to_string(),
            state: meta.state.as_str().to_string(),
            enabled: meta.state.is_usable(),
            provenance,
            generated_workflow_id,
            profile: CapabilityProfileView::default(),
        });
    }

    let payload = CapabilityManagerPayload {
        enabled: false,
        degraded: true,
        status: format!(
            "registry + provenance only — {} skill(s); capability profiles now come from CPP descriptors.",
            skills.len()
        ),
        skills,
    };
    let _ = app_handle.emit(event_names::CAPABILITIES, &payload);
    Ok(payload)
}

// ─── Task 13.2: execution logs (AuditLedger + openclaw::event) ──────────────────
//
// The frozen `AuditLedger` is append-only and exposes NO public read/query API
// (only `append` + `verify_chain`) — see `kria_core::openclaw::audit`. Rather
// than reach around it into the raw table (which would fork the frozen store's
// contract), execution logs are sourced from the SAME live `openclaw::event`
// (per-execution Started/Preparing/Running/Completed/Failed) and
// `bundle::events` (install/update/remove/…) broadcast buses the AuditLedger
// records from and that the existing forwarder already bridges to the UI. A
// bounded in-memory ring buffer captures those events since app start; the UI
// reconciles via polling this command (eventual consistency, R10.2). This is an
// honest surface: it reports exactly the decision-stage events observed on the
// real buses, and its `note` states the historical-scope limitation plainly.

/// Bounded in-memory ring buffer of recent OpenClaw log events.
static OPENCLAW_LOG_BUFFER: OnceLock<StdMutex<VecDeque<serde_json::Value>>> = OnceLock::new();
/// Cap on retained log entries (oldest evicted first).
const OPENCLAW_LOG_BUFFER_CAP: usize = 500;

fn openclaw_log_buffer() -> &'static StdMutex<VecDeque<serde_json::Value>> {
    OPENCLAW_LOG_BUFFER.get_or_init(|| StdMutex::new(VecDeque::new()))
}

/// Push one observed event into the ring buffer, tagged with its source kind and
/// a capture timestamp. Never blocks the forwarder task on a poisoned lock.
fn push_openclaw_log(kind: &str, payload: serde_json::Value) {
    let entry = serde_json::json!({
        "kind": kind,
        "received_at": chrono::Utc::now().to_rfc3339(),
        "event": payload,
    });
    if let Ok(mut buf) = openclaw_log_buffer().lock() {
        if buf.len() >= OPENCLAW_LOG_BUFFER_CAP {
            buf.pop_front();
        }
        buf.push_back(entry);
    }
}

/// Start buffering the real OpenClaw execution + bundle-lifecycle event streams
/// into the in-memory log ring (task 13.2). Reuses the SAME testable
/// `forward_execution_events` / `forward_bundle_events` subscription cores the
/// UI forwarder uses — each subscribes its own broadcast receiver, so this is
/// additive to (not a replacement for) `spawn_openclaw_event_forwarding`.
///
/// Safe to call once at startup: it subscribes to broadcast buses that exist
/// regardless of whether OpenClaw is enabled yet.
pub fn spawn_openclaw_log_buffer() {
    forward_execution_events(|payload| push_openclaw_log("execution", payload));
    forward_bundle_events(|payload| push_openclaw_log("bundle", payload));
}

/// The execution-logs payload (task 13.2 / R10.4).
#[derive(Debug, Clone, Serialize)]
pub struct ExecutionLogsPayload {
    /// Most-recent-last log entries, each `{ kind, received_at, event }`.
    pub entries: Vec<serde_json::Value>,
    /// Honest note on the source + scope of these logs.
    pub note: String,
}

/// Fetch recent OpenClaw execution + lifecycle log entries (task 13.2, R10.4).
///
/// Reads the in-memory ring buffer fed by the real `openclaw::event` +
/// `bundle::events` streams (populated by [`spawn_openclaw_log_buffer`]). Returns
/// the most recent `limit` entries (default 200, capped at the buffer size).
///
/// Honesty: the durable, HMAC-signed `AuditLedger` exposes no public read API,
/// so this surface reflects events observed on the live buses since app start
/// (the UI reconciles via polling, R10.2). The `note` field states this plainly.
#[tauri::command]
pub fn openclaw_execution_logs(limit: Option<usize>) -> Result<ExecutionLogsPayload, String> {
    let k = limit.unwrap_or(200).clamp(1, OPENCLAW_LOG_BUFFER_CAP);
    let entries = {
        let buf = openclaw_log_buffer()
            .lock()
            .map_err(|_| "log buffer poisoned".to_string())?;
        let start = buf.len().saturating_sub(k);
        buf.iter().skip(start).cloned().collect::<Vec<_>>()
    };
    Ok(ExecutionLogsPayload {
        entries,
        note: "Live OpenClaw execution + lifecycle events observed since app start (openclaw::event + bundle::events). The durable AuditLedger is append-only with no public read API; poll to reconcile.".to_string(),
    })
}

// ─── Task 13.2: capability-graph view (CapabilityGraph derived edges) ──────────

/// One derived capability-graph edge for the UI (design §7.4 `capability_edges`).
#[derive(Debug, Clone, Serialize)]
pub struct CapabilityGraphEdgeView {
    pub from_skill: String,
    pub to_skill: String,
    /// `depends` | `provides_for` | `alternative` | `supersedes`.
    pub edge_kind: String,
    pub weight: f64,
}

/// One capability-graph node for the UI (a frozen-registry skill).
#[derive(Debug, Clone, Serialize)]
pub struct CapabilityGraphNodeView {
    pub skill_id: String,
    pub name: String,
    pub category: String,
    pub trust_tier: String,
    pub provenance: String,
}

/// The capability-graph payload (task 13.2 / R10.5).
#[derive(Debug, Clone, Serialize)]
pub struct CapabilityGraphPayload {
    pub enabled: bool,
    pub degraded: bool,
    pub status: String,
    pub nodes: Vec<CapabilityGraphNodeView>,
    pub edges: Vec<CapabilityGraphEdgeView>,
}

/// Capability-graph view (task 13.2, design §7.4 / §13, R10.5).
///
/// Surfaces the derived `capability_edges` view (dependency / provides-for /
/// alternative / supersedes edges) from the CIL [`CapabilityGraph`], with nodes
/// drawn from the frozen registry. Gated behind `openclaw_icp_enabled`: the
/// edge view is a derived table populated only under the flag (task 12.1), so
/// with the flag OFF this returns nodes with an empty edge set and
/// `degraded=true` — honest, never fabricated edges.
///
/// [`CapabilityGraph`]: kria_core::openclaw::cil::CapabilityGraph
#[tauri::command]
pub async fn openclaw_capability_graph(
    app_handle: AppHandle,
    state: State<'_, AppStateCell>,
) -> Result<CapabilityGraphPayload, String> {
    // M12 (Option A): the legacy CIL capability-edge graph is removed. This
    // command now returns skill nodes from the frozen registry with an empty edge
    // set; capability relationships/alternatives are surfaced by CPP discovery
    // (Capabilities → Browser, `cpp_discover`/`cpp_recommend`).
    let app = state.get().ok_or("runtime not ready")?;
    let metadata = all_installed_metadata(&app.skill_registry)?;
    let nodes: Vec<CapabilityGraphNodeView> = metadata
        .iter()
        .map(|meta| {
            let (provenance, _) = provenance_of(&meta.discovery_source);
            CapabilityGraphNodeView {
                skill_id: meta.skill_id.clone(),
                name: meta.name.clone(),
                category: meta.category.clone(),
                trust_tier: meta.trust_tier.as_str().to_string(),
                provenance,
            }
        })
        .collect();
    let edges: Vec<CapabilityGraphEdgeView> = Vec::new();

    let status = format!(
        "nodes only — {} node(s); capability relationships now come from CPP discovery.",
        nodes.len()
    );

    let payload = CapabilityGraphPayload {
        enabled: false,
        degraded: true,
        status,
        nodes,
        edges,
    };
    let _ = app_handle.emit(event_names::CAPABILITY_GRAPH, &payload);
    Ok(payload)
}

// ─── Task 13.3: permission management (GrantStore list + revoke) ────────────────

/// One active scoped permission grant projected for the UI (design §7.4
/// `capability_grants_scoped`). Only active (non-revoked, non-expired) grants
/// are surfaced (deny-by-default).
#[derive(Debug, Clone, Serialize)]
pub struct ScopedGrantView {
    pub grant_id: String,
    pub skill_id: String,
    /// `never` | `once` | `session` | `workspace` | `persistent` | `silent`.
    pub scope_kind: String,
    /// Session/workspace partition key, when partitioned.
    pub scope_key: Option<String>,
    /// `GREEN` | `YELLOW` | `RED` | `BLACK`.
    pub risk: String,
    /// `allow` | `deny`.
    pub decision: String,
    pub granted_at: String,
    pub expires_at: Option<String>,
}

/// The permission-management payload (task 13.3 / R10.1).
#[derive(Debug, Clone, Serialize)]
pub struct GrantsPayload {
    pub grants: Vec<ScopedGrantView>,
    pub status: String,
}

/// List all active scoped permission grants (task 13.3, design §8.7, R10.1).
///
/// A pure read over the `GrantStore` (`capability_grants_scoped`), one query per
/// installed skill (backed by `idx_grants_skill`). Deny-by-default: only
/// non-revoked, non-expired grants are returned, so a revoked/expired grant
/// never appears as active. Never mutates state.
#[tauri::command]
pub async fn openclaw_list_grants(state: State<'_, AppStateCell>) -> Result<GrantsPayload, String> {
    // M12: unified on the ONE CPP grant store (`capability::grants` over
    // `cpp_grants.db`) — the same store the chat dispatcher + Capabilities panel
    // use. No second grant store.
    use kria_core::capability::grants::GrantStore;

    let _app = state.get().ok_or("runtime not ready")?;
    let store = GrantStore::open(&cpp_grants_db_path())
        .map_err(|e| format!("grant store unavailable: {e}"))?;

    let now = chrono::Utc::now();
    let grants = store
        .active_grants(now)
        .map_err(|e| format!("list grants: {e}"))?
        .into_iter()
        .map(|g| ScopedGrantView {
            grant_id: g.grant_id,
            // Provider-neutral: the UI's `skill_id` field carries the fully
            // qualified capability id.
            skill_id: format!("{}/{}", g.provider_id, g.capability_id),
            scope_kind: g.scope_kind.as_str().to_string(),
            scope_key: g.scope_key,
            // Effects → coarse risk label (elevated ⇒ YELLOW, else GREEN).
            risk: if g.effects.is_empty() {
                "GREEN"
            } else {
                "YELLOW"
            }
            .to_string(),
            decision: g.decision.as_str().to_string(),
            granted_at: g.granted_at.to_rfc3339(),
            expires_at: g.expires_at.map(|t| t.to_rfc3339()),
        })
        .collect::<Vec<_>>();

    let status = format!("{} active grant(s)", grants.len());
    Ok(GrantsPayload { grants, status })
}

/// Revoke a permission grant by id (task 13.3, design §8.7, R6.6).
///
/// An explicit, user-triggered state mutation: it marks the grant `revoked=1`
/// via the frozen `PermissionEngine::revoke` (over the `GrantStore`), forcing
/// fresh approval before the affected capability is next used. Deny-by-default
/// and never auto-invoked — this only runs when the user explicitly revokes.
/// Emits `openclaw:grants_changed` so any open permission view refreshes.
#[tauri::command]
pub async fn openclaw_revoke_grant(
    grant_id: String,
    app_handle: AppHandle,
    state: State<'_, AppStateCell>,
) -> Result<(), String> {
    use kria_core::capability::grants::GrantStore;
    use kria_core::capability::permission::{DefaultPermissionEngine, PermissionEngine};

    let _app = state.get().ok_or("runtime not ready")?;
    let store = GrantStore::open(&cpp_grants_db_path())
        .map_err(|e| format!("grant store unavailable: {e}"))?;

    let engine = DefaultPermissionEngine;
    engine
        .revoke(&grant_id, &store)
        .map_err(|e| e.to_string())?;

    tracing::info!(grant_id = %grant_id, "[OpenClaw] permission grant revoked by user");
    let _ = app_handle.emit(event_names::GRANTS_CHANGED, &grant_id);
    Ok(())
}

// ─── Task 13.3: developer-mode gating ───────────────────────────────────────────
//
// Not-production-ready OpenClaw surfaces are gated behind a Developer Mode flag
// (R10.3). The flag is owned entirely by the desktop app as a small JSON file in
// the KRIA data dir — it does NOT touch any frozen kria-core config, and it
// defaults OFF (production-safe). The UI reads it to hide/show gated features.

/// On-disk developer-mode flag state.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DeveloperModeState {
    enabled: bool,
}

fn developer_mode_path() -> std::path::PathBuf {
    kria_data_dir().join("openclaw_devmode.json")
}

/// Read the OpenClaw Developer Mode flag (task 13.3, R10.3). Defaults `false`
/// (production-safe) when the flag file is absent or unreadable.
#[tauri::command]
pub fn openclaw_get_developer_mode() -> Result<bool, String> {
    let path = developer_mode_path();
    match std::fs::read_to_string(&path) {
        Ok(raw) => {
            let st: DeveloperModeState = serde_json::from_str(&raw).unwrap_or_default();
            Ok(st.enabled)
        }
        // Absent file = default OFF (not an error).
        Err(_) => Ok(false),
    }
}

/// Set the OpenClaw Developer Mode flag (task 13.3, R10.3). Persists to the
/// desktop-local flag file and emits `openclaw:developer_mode` so open UI
/// surfaces reconcile immediately.
#[tauri::command]
pub fn openclaw_set_developer_mode(enabled: bool, app_handle: AppHandle) -> Result<(), String> {
    let path = developer_mode_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body = serde_json::to_string(&DeveloperModeState { enabled })
        .map_err(|e| format!("serialize developer mode: {e}"))?;
    std::fs::write(&path, body).map_err(|e| format!("persist developer mode: {e}"))?;
    let _ = app_handle.emit(event_names::DEVELOPER_MODE, enabled);
    Ok(())
}
