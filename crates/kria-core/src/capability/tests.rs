//! Milestone 1 validation for the Capability Provider Platform boundary.
//!
//! These tests assert the boundary's structural guarantees: descriptor
//! round-trip + forward-compat, conservative thin-provider defaults, negotiation
//! intersection + unknown-feature preservation, open-vocabulary extensibility
//! (Property 2 seed), state-machine legality, config defaults (flag OFF), and
//! honest error messages. No Docker/LLM/network required.

use super::config::{CapabilityPlatformConfig, ProviderConfig};
use super::descriptor::{
    CapabilityDescriptor, CapabilityTag, DescriptorVersion, Effects, Reversibility,
};
use super::error::CapError;
use super::fake::FakeProvider;
use super::protocol::{ClientCapabilities, Feature, FeatureSet, ProtocolVersion, ProviderHealth};
use super::provider::{CapabilityOutcome, CapabilityProvider, CapabilityRequest, RequestContext};
use super::state::{CapabilityState, ProviderState};

fn sample_descriptor(provider: &str, cap: &str) -> CapabilityDescriptor {
    let mut d = CapabilityDescriptor::minimal(
        provider,
        cap,
        "Sample",
        "A sample capability",
        serde_json::json!({"type": "object", "properties": {"x": {"type": "number"}}}),
    );
    d.tags = vec![CapabilityTag::new("math.arithmetic.add")];
    d.inputs = vec!["number".into()];
    d.outputs = vec!["number".into()];
    d
}

#[test]
fn descriptor_round_trips_and_preserves_unknown_extensions() {
    let mut d = sample_descriptor("openclaw", "calculator");
    // A field a "newer provider" advertised that this build does not model.
    d.extensions.insert(
        "future_capability_hint".to_string(),
        serde_json::json!({"beta": true, "score": 0.9}),
    );

    let json = serde_json::to_string(&d).expect("serialize");
    let back: CapabilityDescriptor = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(back, d, "descriptor must round-trip losslessly");
    assert_eq!(
        back.extensions.get("future_capability_hint"),
        Some(&serde_json::json!({"beta": true, "score": 0.9})),
        "unknown forward-compat fields must be preserved"
    );
    back.validate().expect("sample descriptor is valid");
}

#[test]
fn old_v1_descriptor_without_v11_blocks_still_deserializes() {
    // A minimal object lacking guidance/expectations/schema_version (an older
    // v1 provider). Additive fields must default; validation must pass.
    let json = r#"{
        "provider_id": "mcp:example",
        "capability_id": "echo",
        "name": "Echo",
        "description": "Echoes input"
    }"#;
    let d: CapabilityDescriptor = serde_json::from_str(json).expect("deserialize old v1");
    assert_eq!(d.schema_version, DescriptorVersion::CURRENT);
    assert!(d.guidance.is_none());
    assert!(d.expectations.is_none());
    d.validate().expect("old v1 descriptor validates");
}

#[test]
fn minimal_descriptor_is_conservatively_elevated() {
    let d = CapabilityDescriptor::minimal(
        "mcp:thin",
        "unknown_tool",
        "Unknown",
        "A thin provider tool",
        serde_json::json!({}),
    );
    // Thin provider ⇒ unknown reversibility ⇒ elevated (needs approval).
    assert_eq!(d.effects.reversible, Reversibility::Unknown);
    assert!(
        d.effects.is_elevated(),
        "a thin provider's capability must default to elevated (approval-required)"
    );
    assert_eq!(d.io_modality, vec!["text".to_string()]);
}

#[test]
fn descriptor_validation_rejects_empty_identity() {
    let mut d = sample_descriptor("openclaw", "calc");
    d.capability_id = "".into();
    let err = d.validate().expect_err("empty capability_id must fail");
    assert!(matches!(err, CapError::Descriptor(_)));
    assert!(!err.to_string().is_empty(), "error must be user-actionable");
}

#[test]
fn effects_low_risk_is_not_elevated() {
    let effects = Effects {
        classes: vec!["read".into()],
        reversible: Reversibility::Reversible,
        idempotent: true,
        resource_class: super::descriptor::ResourceClass::Light,
    };
    assert!(
        !effects.is_elevated(),
        "read-only, reversible, idempotent ⇒ not elevated"
    );
}

#[test]
fn novel_tag_and_effect_flow_through_unchanged() {
    // Property 2 seed: a never-before-seen capability tag + effect class must be
    // representable and round-trip with NO code change and NO enum rejecting it.
    let mut d = CapabilityDescriptor::minimal(
        "provider.from.the.future",
        "quantum.entangle",
        "Entangle",
        "A capability domain that does not exist yet",
        serde_json::json!({"type": "object"}),
    );
    d.tags = vec![CapabilityTag::new("physics.quantum.entangle")];
    d.effects.classes = vec!["quantum_side_effect".into()];

    d.validate()
        .expect("novel tag/effect must validate (open vocabulary)");
    let json = serde_json::to_string(&d).unwrap();
    let back: CapabilityDescriptor = serde_json::from_str(&json).unwrap();
    assert_eq!(back.tags[0].id, "physics.quantum.entangle");
    assert_eq!(
        back.effects.classes,
        vec!["quantum_side_effect".to_string()]
    );
}

#[test]
fn negotiation_is_version_min_and_feature_intersection() {
    let client = ClientCapabilities::default(); // supports everything, v1.0
                                                // Provider supports mandatory + Lifecycle, but NOT Streaming; at v1.0.
    let provider_features = FeatureSet::mandatory().with(Feature::Lifecycle);
    let session = client.negotiate(
        "openclaw",
        ProtocolVersion::new(1, 0),
        provider_features,
        serde_json::Map::new(),
    );

    assert!(session.has_mandatory(), "mandatory facets must be agreed");
    assert!(session.supports_lifecycle(), "agreed lifecycle facet");
    assert!(
        !session.supports_streaming(),
        "streaming absent from provider ⇒ absent from session (not an error)"
    );
    assert_eq!(session.version, ProtocolVersion::new(1, 0));
}

#[test]
fn negotiation_preserves_unknown_provider_features_in_extensions() {
    let client = ClientCapabilities::default();
    let mut ext = serde_json::Map::new();
    ext.insert("x_experimental_batch_v2".into(), serde_json::json!(true));

    let session = client.negotiate(
        "provider.future",
        ProtocolVersion::new(1, 0),
        FeatureSet::mandatory(),
        ext.clone(),
    );
    assert_eq!(
        session.extensions.get("x_experimental_batch_v2"),
        Some(&serde_json::json!(true)),
        "unknown negotiated features must be preserved for forward-compat"
    );
}

#[test]
fn featureset_serializes_as_names_and_ignores_unknown() {
    let set = FeatureSet::mandatory().with(Feature::Streaming);
    let json = serde_json::to_string(&set).unwrap();
    // Round-trips through names.
    let back: FeatureSet = serde_json::from_str(&json).unwrap();
    assert_eq!(back, set);

    // Unknown names on deserialization are ignored (they live in extensions).
    let with_unknown: FeatureSet =
        serde_json::from_str(r#"["describe","execute","discover","warp_drive"]"#).unwrap();
    assert!(with_unknown.contains(Feature::Describe));
    assert!(!with_unknown.contains(Feature::Streaming));
}

#[test]
fn lower_version_wins_in_negotiation() {
    let client = ClientCapabilities {
        version: ProtocolVersion::new(1, 3),
        features: FeatureSet::mandatory(),
    };
    let session = client.negotiate(
        "old.provider",
        ProtocolVersion::new(1, 0),
        FeatureSet::mandatory(),
        serde_json::Map::new(),
    );
    assert_eq!(
        session.version,
        ProtocolVersion::new(1, 0),
        "must operate at the lower mutually-supported version"
    );
}

#[test]
fn capability_state_machine_rejects_illegal_jumps() {
    assert!(CapabilityState::Discovered.can_transition_to(CapabilityState::Available));
    assert!(CapabilityState::Ready.can_transition_to(CapabilityState::Executing));
    assert!(CapabilityState::Executing.can_transition_to(CapabilityState::Failed));
    assert!(CapabilityState::Failed.can_transition_to(CapabilityState::Recovering));
    // Illegal jumps rejected.
    assert!(!CapabilityState::Discovered.can_transition_to(CapabilityState::Executing));
    assert!(!CapabilityState::Removed.can_transition_to(CapabilityState::Ready));
}

#[test]
fn provider_state_machine_paths() {
    assert!(ProviderState::Offline.can_transition_to(ProviderState::Connecting));
    assert!(ProviderState::Negotiating.can_transition_to(ProviderState::Ready));
    assert!(ProviderState::Healthy.can_transition_to(ProviderState::Degraded));
    assert!(ProviderState::Degraded.can_transition_to(ProviderState::Disconnected));
    assert!(!ProviderState::Offline.can_transition_to(ProviderState::Healthy));
}

#[test]
fn config_defaults_flag_off_and_no_providers() {
    let cfg = CapabilityPlatformConfig::default();
    assert!(
        !cfg.enabled,
        "capability_provider_platform_enabled MUST default to false"
    );
    assert!(cfg.providers.is_empty());
}

#[test]
fn config_loads_from_toml_additively() {
    let toml_src = r#"
        enabled = true

        [[providers]]
        id = "openclaw"
        enabled = true
        kind = "openclaw"

        [[providers]]
        id = "mcp:github"
        enabled = false
        kind = "mcp"
    "#;
    let cfg: CapabilityPlatformConfig = toml::from_str(toml_src).expect("deserialize [capability]");
    assert!(cfg.enabled);
    assert_eq!(cfg.providers.len(), 2);
    assert_eq!(cfg.enabled_provider_ids(), vec!["openclaw".to_string()]);
    assert_eq!(cfg.provider("mcp:github").map(|p| p.enabled), Some(false));
}

#[test]
fn provider_config_partial_uses_defaults() {
    let p: ProviderConfig = toml::from_str("id = \"x\"\nkind = \"mcp\"\n").expect("deserialize");
    assert_eq!(p.id, "x");
    assert!(!p.enabled, "provider disabled by default");
    assert!(p.settings.is_empty());
}

#[test]
fn cap_error_messages_are_actionable() {
    // Each variant must render a non-empty, human-readable message.
    for e in [
        CapError::Negotiation("v mismatch".into()),
        CapError::Unsupported("lifecycle".into()),
        CapError::Descriptor("empty id".into()),
        CapError::Discovery("index down".into()),
        CapError::Permission("needs approval".into()),
        CapError::Acquire("verify failed".into()),
        CapError::Execute("nonzero exit".into()),
        CapError::Degraded("embedder offline".into()),
        CapError::ProviderOffline("docker down".into()),
        CapError::Io("disk full".into()),
    ] {
        assert!(!e.to_string().is_empty());
    }
    assert!(CapError::Unsupported("x".into()).is_unsupported());
    assert!(CapError::Degraded("x".into()).is_transient());
    assert!(CapError::ProviderOffline("x".into()).is_transient());
}

#[tokio::test]
async fn fake_provider_negotiate_describe_execute() {
    let desc = sample_descriptor("fake", "calc");
    let provider = FakeProvider::new("fake", vec![desc]);
    let client = ClientCapabilities::default();

    let session = provider.negotiate(&client).await.expect("negotiate");
    assert!(session.has_mandatory());
    assert!(
        !session.supports_lifecycle(),
        "fake advertises no lifecycle"
    );

    let descs = provider.describe(&session).await.expect("describe");
    assert_eq!(descs.len(), 1);

    let out = provider
        .execute(CapabilityRequest {
            provider_id: "fake".into(),
            capability_id: "calc".into(),
            args: serde_json::json!({"x": 3}),
            context: RequestContext::new(),
            granted_effects: vec![],
        })
        .await
        .expect("execute");
    match out {
        CapabilityOutcome::Value(v) => assert_eq!(v["echo"]["x"], 3),
        other => panic!("expected Value, got {other:?}"),
    }
    assert_eq!(provider.health().await, ProviderHealth::Ready);
}

#[tokio::test]
async fn fake_provider_lifecycle_defaults_to_unsupported() {
    let provider = FakeProvider::new("fake", vec![]);
    let err = provider
        .acquire(&super::provider::AcquireRequest {
            capability_tag: "x".into(),
            hint: None,
            capability_id: None,
            proposed_graph: None,
            context: RequestContext::new(),
        })
        .await
        .expect_err("read-only provider must not support acquire");
    assert!(err.is_unsupported());
}

// ─── Milestone 3: federated discovery ────────────────────────────────────────

use super::index::{Embedder, FederatedIndex, InMemoryFederatedIndex, ScoredDescriptor};
use super::registry::ProviderRegistry;
use std::sync::Arc;

/// A deterministic fake embedder for index tests (no ONNX dependency): embeds a
/// fixed-dim vector from token hashing so cosine is stable and reproducible.
struct HashEmbedder {
    dim: usize,
}

impl Embedder for HashEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, CapError> {
        let mut v = vec![0.0f32; self.dim];
        for tok in text.to_lowercase().split_whitespace() {
            let mut h: u64 = 1469598103934665603;
            for b in tok.bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(1099511628211);
            }
            let idx = (h as usize) % self.dim;
            v[idx] += 1.0;
        }
        Ok(v)
    }
    fn dim(&self) -> usize {
        self.dim
    }
    fn model_id(&self) -> &str {
        "hash-test"
    }
}

fn desc_with(
    provider: &str,
    cap: &str,
    name: &str,
    description: &str,
    tags: &[&str],
) -> CapabilityDescriptor {
    let mut d = CapabilityDescriptor::minimal(
        provider,
        cap,
        name,
        description,
        serde_json::json!({"type": "object"}),
    );
    d.tags = tags.iter().map(|t| CapabilityTag::new(*t)).collect();
    d
}

fn test_index() -> Arc<InMemoryFederatedIndex> {
    Arc::new(InMemoryFederatedIndex::new(Arc::new(HashEmbedder {
        dim: 64,
    })))
}

#[test]
fn federated_index_ranks_relevant_descriptor_first() {
    let index = test_index();
    index
        .rebuild(&[
            desc_with(
                "openclaw",
                "oc_calculator",
                "Calculator",
                "Evaluate an arithmetic expression and return the numeric result",
                &["math.arithmetic"],
            ),
            desc_with(
                "openclaw",
                "oc_web_fetch",
                "Web Fetch",
                "Fetch the contents of a web page over http",
                &["net.http.fetch"],
            ),
            desc_with(
                "mcp:x",
                "hash_tool",
                "Hash",
                "Compute a cryptographic hash of some text",
                &["crypto.hash"],
            ),
        ])
        .unwrap();

    let hits = index.search("evaluate arithmetic expression", 3).unwrap();
    assert_eq!(
        hits[0].descriptor.capability_id, "oc_calculator",
        "calculator must rank first for an arithmetic query"
    );
    assert!(hits[0].score >= hits[1].score);
}

#[test]
fn federated_rebuild_is_idempotent() {
    // Property 5: rebuilding from the same descriptors yields identical results.
    let index = test_index();
    let descs = vec![
        desc_with(
            "openclaw",
            "a",
            "Alpha",
            "first capability about files",
            &["io.file"],
        ),
        desc_with(
            "mcp:y",
            "b",
            "Beta",
            "second capability about network",
            &["net"],
        ),
    ];
    index.rebuild(&descs).unwrap();
    let first = index.search("files", 5).unwrap();
    index.rebuild(&descs).unwrap();
    let second = index.search("files", 5).unwrap();
    let ids = |v: &[ScoredDescriptor]| {
        v.iter()
            .map(|s| s.descriptor.capability_id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ids(&first),
        ids(&second),
        "idempotent reindex must be stable"
    );
}

#[test]
fn federated_upsert_and_remove() {
    let index = test_index();
    index
        .rebuild(&[desc_with("openclaw", "a", "A", "alpha", &[])])
        .unwrap();
    assert_eq!(index.len(), 1);
    index
        .upsert(&desc_with("mcp:z", "b", "B", "beta", &[]))
        .unwrap();
    assert_eq!(index.len(), 2);
    // Upsert same key replaces, not duplicates.
    index
        .upsert(&desc_with("mcp:z", "b", "B2", "beta updated", &[]))
        .unwrap();
    assert_eq!(index.len(), 2);
    index.remove("mcp:z", "b");
    assert_eq!(index.len(), 1);
}

#[tokio::test]
async fn provider_registry_federates_two_providers() {
    // Property 2 + cross-provider: a novel provider with a novel tag flows
    // through register → refresh → search with no special-casing.
    let index = test_index();
    let registry = ProviderRegistry::new(index);

    let p1 = FakeProvider::new(
        "openclaw",
        vec![desc_with(
            "openclaw",
            "oc_calculator",
            "Calculator",
            "evaluate arithmetic expression",
            &["math.arithmetic"],
        )],
    );
    let p2 = FakeProvider::new(
        "provider.novel.v9",
        vec![desc_with(
            "provider.novel.v9",
            "quantum_solve",
            "Quantum Solver",
            "solve a quantum optimization problem",
            &["physics.quantum.solve"],
        )],
    );
    registry.register(Arc::new(p1));
    registry.register(Arc::new(p2));

    let report = registry.refresh().await;
    assert_eq!(report.total_descriptors, 2);
    assert_eq!(report.healthy_count(), 2);

    // Both providers' capabilities are discoverable across the federation.
    let arith = registry.search("arithmetic expression", 2).unwrap();
    assert_eq!(arith[0].descriptor.provider_id, "openclaw");

    let quantum = registry.search("quantum optimization", 2).unwrap();
    assert_eq!(
        quantum[0].descriptor.provider_id, "provider.novel.v9",
        "novel provider federates with no code change"
    );
}

// ─── Milestone 4: descriptor-effects permission + durable grants ──────────────

use super::descriptor::ResourceClass as RC;
use super::grants::{GrantDecision, GrantStore, ScopeKind};
use super::permission::{
    approval_grant, AuthorizeRequest, DefaultPermissionEngine, PermissionEngine, PermissionTier,
};

fn effects(classes: &[&str], reversible: Reversibility) -> Effects {
    Effects {
        classes: classes.iter().map(|s| s.to_string()).collect(),
        reversible,
        idempotent: false,
        resource_class: RC::Light,
    }
}

fn authz(provider: &str, cap: &str, effects: Effects, session: Option<&str>) -> AuthorizeRequest {
    AuthorizeRequest {
        provider_id: provider.into(),
        capability_id: cap.into(),
        effects,
        session_id: session.map(|s| s.into()),
        workspace_id: None,
    }
}

#[test]
fn never_ask_for_pure_reversible_capability() {
    // Property 7 (never-ask): read-only + reversible ⇒ NeverAsk, no prompt ever.
    let engine = DefaultPermissionEngine;
    let grants = GrantStore::in_memory().unwrap();
    let req = authz(
        "openclaw",
        "oc_calculator",
        effects(&["read"], Reversibility::Reversible),
        None,
    );
    let d = engine.authorize(&req, &grants);
    assert!(matches!(
        d,
        super::permission::PermissionDecision::Allow {
            tier: PermissionTier::NeverAsk,
            ..
        }
    ));
}

#[test]
fn always_ask_for_irreversible_regardless_of_trust() {
    // Property 7 (deny-by-default): irreversible ⇒ AlwaysAsk prompt, not remembered.
    let engine = DefaultPermissionEngine;
    let grants = GrantStore::in_memory().unwrap();
    let req = authz(
        "p",
        "delete_file",
        effects(&["write"], Reversibility::Irreversible),
        Some("s1"),
    );
    let d = engine.authorize(&req, &grants);
    assert!(matches!(
        d,
        super::permission::PermissionDecision::Prompt {
            tier: PermissionTier::AlwaysAsk,
            ..
        }
    ));
}

#[test]
fn host_subprocess_is_always_ask() {
    let engine = DefaultPermissionEngine;
    let grants = GrantStore::in_memory().unwrap();
    let req = authz(
        "p",
        "shell",
        effects(&["subprocess"], Reversibility::Reversible),
        Some("s1"),
    );
    assert!(engine.authorize(&req, &grants).is_prompt());
}

#[test]
fn silent_policy_grant_allows_system_modifying() {
    let engine = DefaultPermissionEngine;
    let grants = GrantStore::in_memory().unwrap();
    let req = authz(
        "p",
        "delete_file",
        effects(&["write"], Reversibility::Irreversible),
        None,
    );
    // Policy pre-authorizes via a Silent grant.
    grants
        .insert(&approval_grant(
            &req,
            ScopeKind::Silent,
            GrantDecision::Allow,
        ))
        .unwrap();
    let d = engine.authorize(&req, &grants);
    assert!(matches!(
        d,
        super::permission::PermissionDecision::Allow {
            tier: PermissionTier::Silent,
            ..
        }
    ));
}

#[test]
fn context_grant_reuse_then_revoke_reprompts() {
    let engine = DefaultPermissionEngine;
    let grants = GrantStore::in_memory().unwrap();
    // Network capability (elevated, reversible) in a session → prompt first.
    let req = authz(
        "p",
        "web_fetch",
        effects(&["network"], Reversibility::Reversible),
        Some("sess-1"),
    );
    assert!(
        engine.authorize(&req, &grants).is_prompt(),
        "first use prompts"
    );

    // User approves for the session → persist grant.
    let grant = approval_grant(&req, ScopeKind::Session, GrantDecision::Allow);
    let gid = grant.grant_id.clone();
    grants.insert(&grant).unwrap();

    // Reuse: same scope + same effects ⇒ Allow without prompt.
    assert!(
        engine.authorize(&req, &grants).is_allow(),
        "grant reused within session"
    );

    // Revoke ⇒ prompt again (fresh approval required).
    engine.revoke(&gid, &grants).unwrap();
    assert!(
        engine.authorize(&req, &grants).is_prompt(),
        "revoked grant forces re-approval"
    );
}

#[test]
fn permission_monotonicity_narrowing_allows_widening_prompts() {
    // Property 7 (monotonicity): grant {network, write}; a NARROWER request
    // {network} is still covered (Allow); a WIDER request {network, subprocess}
    // is NOT covered (Prompt).
    let engine = DefaultPermissionEngine;
    let grants = GrantStore::in_memory().unwrap();

    // Grant for a reversible network+read capability at session scope.
    let granted = authz(
        "p",
        "multi",
        effects(&["network", "read"], Reversibility::Reversible),
        Some("s"),
    );
    grants
        .insert(&approval_grant(
            &granted,
            ScopeKind::Session,
            GrantDecision::Allow,
        ))
        .unwrap();

    // Narrowing: request only {network} ⇒ covered ⇒ Allow.
    let narrower = authz(
        "p",
        "multi",
        effects(&["network"], Reversibility::Reversible),
        Some("s"),
    );
    assert!(
        engine.authorize(&narrower, &grants).is_allow(),
        "narrowing must not re-prompt"
    );

    // Widening: request {network, browser} — browser not granted ⇒ Prompt.
    let wider = authz(
        "p",
        "multi",
        effects(&["network", "browser"], Reversibility::Reversible),
        Some("s"),
    );
    assert!(
        engine.authorize(&wider, &grants).is_prompt(),
        "widening must re-prompt"
    );
}

#[test]
fn grant_store_durability_and_active_listing() {
    // Durable across store reopen (simulated via a temp file DB).
    let dir = std::env::temp_dir().join(format!("cpp_grants_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("grants.db");

    let req = authz(
        "p",
        "cap",
        effects(&["network"], Reversibility::Reversible),
        Some("s"),
    );
    let gid;
    {
        let store = GrantStore::open(&path).unwrap();
        let g = approval_grant(&req, ScopeKind::Persistent, GrantDecision::Allow);
        gid = g.grant_id.clone();
        store.insert(&g).unwrap();
        assert_eq!(store.active_grants(chrono::Utc::now()).unwrap().len(), 1);
    }
    // Reopen: grant persisted.
    {
        let store = GrantStore::open(&path).unwrap();
        let active = store.active_grants(chrono::Utc::now()).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].grant_id, gid);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ─── Milestone 7: provider conformance harness (SDK) ──────────────────────────

use super::conformance::run_conformance;
use super::protocol::{ProtocolSession, ProviderHealth as PH};
use super::provider::CapabilityOutcome as CO;

#[tokio::test]
async fn conformance_passes_for_fake_provider() {
    let provider = FakeProvider::new("fake", vec![sample_descriptor("fake", "cap1")]);
    let report = run_conformance(&provider).await;
    assert!(
        report.passed(),
        "FakeProvider must pass conformance; failures: {:?}",
        report.failures()
    );
    assert_eq!(report.descriptor_count, 1);
}

/// A brand-new, from-scratch provider defined ENTIRELY in this test (no
/// KRIA-core change) must pass conformance — Property 2 at the SDK level: adding
/// a provider is an adapter, not a core edit.
struct ExampleWeatherProvider {
    id: String,
}

#[async_trait::async_trait]
impl CapabilityProvider for ExampleWeatherProvider {
    fn provider_id(&self) -> &String {
        &self.id
    }
    async fn negotiate(
        &self,
        client: &super::protocol::ClientCapabilities,
    ) -> Result<ProtocolSession, CapError> {
        Ok(client.negotiate(
            self.id.clone(),
            super::protocol::ProtocolVersion::CURRENT,
            FeatureSet::mandatory(),
            serde_json::Map::new(),
        ))
    }
    async fn describe(&self, _s: &ProtocolSession) -> Result<Vec<CapabilityDescriptor>, CapError> {
        Ok(vec![CapabilityDescriptor::minimal(
            self.id.clone(),
            "get_forecast",
            "Get Forecast",
            "Return the weather forecast for a location",
            serde_json::json!({"type": "object", "properties": {"location": {"type": "string"}}}),
        )])
    }
    async fn execute(&self, req: super::provider::CapabilityRequest) -> Result<CO, CapError> {
        Ok(CO::Value(
            serde_json::json!({"forecast": "sunny", "for": req.args}),
        ))
    }
    async fn health(&self) -> PH {
        PH::Ready
    }
}

#[tokio::test]
async fn conformance_passes_for_brand_new_provider_with_no_core_change() {
    let provider = ExampleWeatherProvider {
        id: "example:weather".to_string(),
    };
    let report = run_conformance(&provider).await;
    assert!(
        report.passed(),
        "a brand-new provider must pass conformance with zero KRIA-core change; failures: {:?}",
        report.failures()
    );
    // And it federates + is discoverable through the same platform.
    let index = test_index();
    let registry = ProviderRegistry::new(index);
    registry.register(Arc::new(ExampleWeatherProvider {
        id: "example:weather".to_string(),
    }));
    let report = registry.refresh().await;
    assert_eq!(report.total_descriptors, 1);
    let hits = registry
        .search("weather forecast for a location", 3)
        .unwrap();
    assert_eq!(hits[0].descriptor.provider_id, "example:weather");
}

#[tokio::test]
async fn conformance_flags_a_broken_provider() {
    // A provider that emits an INVALID descriptor (empty capability_id) must FAIL
    // conformance — proving the harness actually guards the contract.
    struct BrokenProvider;
    #[async_trait::async_trait]
    impl CapabilityProvider for BrokenProvider {
        fn provider_id(&self) -> &String {
            static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
            ID.get_or_init(|| "broken".to_string())
        }
        async fn negotiate(
            &self,
            client: &super::protocol::ClientCapabilities,
        ) -> Result<ProtocolSession, CapError> {
            Ok(client.negotiate(
                "broken".to_string(),
                super::protocol::ProtocolVersion::CURRENT,
                FeatureSet::mandatory(),
                serde_json::Map::new(),
            ))
        }
        async fn describe(
            &self,
            _s: &ProtocolSession,
        ) -> Result<Vec<CapabilityDescriptor>, CapError> {
            // Invalid: empty capability_id.
            Ok(vec![CapabilityDescriptor::minimal(
                "broken",
                "",
                "Bad",
                "no id",
                serde_json::json!({}),
            )])
        }
        async fn execute(&self, _req: super::provider::CapabilityRequest) -> Result<CO, CapError> {
            Ok(CO::Declined {
                reason: "n/a".into(),
            })
        }
        async fn health(&self) -> PH {
            PH::Ready
        }
    }
    let report = run_conformance(&BrokenProvider).await;
    assert!(!report.passed(), "broken descriptor must fail conformance");
    assert!(report.failures().contains(&"descriptors_valid"));
}

// ─── Milestone 8: observability event bus + circuit breaker ───────────────────

use super::events::{CapabilityEventBus, Outcome as EvOutcome, Stage as EvStage};
use super::platform::CapabilityPlatform;

#[tokio::test]
async fn platform_recommends_installable_catalog_entries() {
    // M6 marketplace catalog federation: a provider advertises an installable
    // catalog (not-yet-installed); recommend ranks it against a goal.
    let index = test_index();
    let registry = ProviderRegistry::new(index);
    let provider = FakeProvider::new("market", vec![]).with_catalog(vec![
        desc_with(
            "market",
            "pdf_ocr",
            "PDF OCR",
            "extract text from scanned pdf documents",
            &["media.pdf.ocr"],
        ),
        desc_with(
            "market",
            "img_resize",
            "Image Resize",
            "resize and crop images",
            &["media.image.resize"],
        ),
    ]);
    registry.register(Arc::new(provider));
    let platform = CapabilityPlatform::new(Arc::new(registry));

    let recs = platform
        .recommend("extract text from a scanned pdf", 5)
        .await
        .unwrap();
    assert!(
        !recs.is_empty(),
        "recommendations must be returned from the catalog"
    );
    assert_eq!(recs[0].descriptor.capability_id, "pdf_ocr");
    // Catalog entries are flagged not-installed.
    assert_eq!(
        recs[0].descriptor.extensions.get("installed"),
        None,
        "fake catalog descriptors here carry no installed flag (real OpenClaw catalog sets installed=false)"
    );
}

#[tokio::test]
async fn platform_emits_execution_events() {
    let index = test_index();
    let registry = ProviderRegistry::new(index);
    registry.register(Arc::new(FakeProvider::new(
        "fake",
        vec![sample_descriptor("fake", "cap1")],
    )));
    let bus = std::sync::Arc::new(CapabilityEventBus::new(64));
    let platform = CapabilityPlatform::new(Arc::new(registry)).with_events(bus.clone());
    platform.refresh().await;

    let mut rx = bus.subscribe();
    let out = platform
        .execute(CapabilityRequest {
            provider_id: "fake".into(),
            capability_id: "cap1".into(),
            args: serde_json::json!({}),
            context: RequestContext::new(),
            granted_effects: vec![],
        })
        .await
        .unwrap();
    assert!(matches!(out, CapabilityOutcome::Value(_)));

    // Two execute-stage events: Started then Ok.
    let e1 = rx.try_recv().expect("started event");
    let e2 = rx.try_recv().expect("terminal event");
    assert_eq!(e1.stage, EvStage::Execute);
    assert_eq!(e1.outcome, EvOutcome::Started);
    assert_eq!(e2.outcome, EvOutcome::Ok);
    assert_eq!(e2.provider_id, "fake");
    assert_eq!(e2.capability_id.as_deref(), Some("cap1"));
}

/// A provider whose `execute` always errors — for circuit-breaker testing.
struct FailingProvider;

#[async_trait::async_trait]
impl CapabilityProvider for FailingProvider {
    fn provider_id(&self) -> &String {
        static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        ID.get_or_init(|| "flaky".to_string())
    }
    async fn negotiate(
        &self,
        client: &super::protocol::ClientCapabilities,
    ) -> Result<super::protocol::ProtocolSession, CapError> {
        Ok(client.negotiate(
            "flaky".to_string(),
            super::protocol::ProtocolVersion::CURRENT,
            FeatureSet::mandatory(),
            serde_json::Map::new(),
        ))
    }
    async fn describe(
        &self,
        _s: &super::protocol::ProtocolSession,
    ) -> Result<Vec<CapabilityDescriptor>, CapError> {
        Ok(vec![sample_descriptor("flaky", "x")])
    }
    async fn execute(&self, _req: CapabilityRequest) -> Result<CapabilityOutcome, CapError> {
        Err(CapError::Execute("simulated failure".into()))
    }
    async fn health(&self) -> super::protocol::ProviderHealth {
        super::protocol::ProviderHealth::Ready
    }
}

#[tokio::test]
async fn circuit_breaker_opens_after_consecutive_failures() {
    let index = test_index();
    let registry = Arc::new(ProviderRegistry::new(index));
    registry.register(Arc::new(FailingProvider));
    let platform = CapabilityPlatform::new(registry.clone());

    assert!(!registry.is_breaker_open("flaky"));
    for _ in 0..3 {
        let r = platform
            .execute(CapabilityRequest {
                provider_id: "flaky".into(),
                capability_id: "x".into(),
                args: serde_json::json!({}),
                context: RequestContext::new(),
                granted_effects: vec![],
            })
            .await;
        assert!(r.is_err(), "failing provider must error");
    }
    // 3 consecutive real execution failures ⇒ breaker open (threshold = 3).
    assert!(
        registry.is_breaker_open("flaky"),
        "breaker must open after 3 consecutive failures"
    );

    // A success resets it.
    registry.record_execution_outcome("flaky", "x", true);
    assert!(!registry.is_breaker_open("flaky"));
}

#[test]
fn learning_success_signal_shifts_ranking() {
    // M6 learning loop: two equally-matching capabilities; repeated SUCCESS on
    // one and FAILURE on the other must rank the successful one higher.
    let index = test_index();
    // Identical descriptor TEXT (same name/description/tags) so semantic +
    // lexical scores tie exactly and the learned success signal is the sole
    // differentiator (capability_id is not part of the embedded text).
    index
        .rebuild(&[
            desc_with("p", "alpha", "Tool", "process text data", &["text"]),
            desc_with("p", "beta", "Tool", "process text data", &["text"]),
        ])
        .unwrap();

    // Baseline: identical descriptors → tie (stable order).
    let before = index.search("process text data", 2).unwrap();
    assert_eq!(before.len(), 2);

    // Alpha succeeds repeatedly; beta fails repeatedly.
    for _ in 0..5 {
        index.record_outcome("p", "alpha", true);
        index.record_outcome("p", "beta", false);
    }
    let after = index.search("process text data", 2).unwrap();
    assert_eq!(
        after[0].descriptor.capability_id, "alpha",
        "the historically-successful capability must rank first after learning"
    );
}

// ─── OpenClaw Intelligence Enhancements: CKB learning loop (P1/P2 e2e) ───────

/// End-to-end: executing a capability through the platform records the outcome
/// to the wired CKB (learning layer), and the confidence selector then reuses
/// the learned signal. Real code paths (registry → platform.execute → CKB →
/// selector), no GUI/LLM/Docker needed (spec R1.4 / R3 / Property 10-adjacent).
#[tokio::test]
async fn ckb_learning_loop_records_and_reuses() {
    use super::intelligence::{
        CapabilityKnowledge, DefaultCapabilitySelector, SqliteCapabilityKnowledge,
    };

    let index = test_index();
    let registry = ProviderRegistry::new(index);
    registry.register(Arc::new(FakeProvider::new(
        "fake",
        vec![sample_descriptor("fake", "cap1")],
    )));

    let ckb: Arc<dyn CapabilityKnowledge> =
        Arc::new(SqliteCapabilityKnowledge::in_memory().unwrap());
    // Register the capability as installed so it is grounded + listable.
    ckb.record_install(&sample_descriptor("fake", "cap1"))
        .await
        .unwrap();

    let platform = CapabilityPlatform::new(Arc::new(registry)).with_knowledge(ckb.clone());
    platform.refresh().await;

    // Before execution: unobserved ⇒ neutral 0.5.
    assert_eq!(ckb.success_rate("fake", "cap1").await, 0.5);

    // Execute twice (both succeed via FakeProvider echo).
    for _ in 0..2 {
        let out = platform
            .execute(CapabilityRequest {
                provider_id: "fake".into(),
                capability_id: "cap1".into(),
                args: serde_json::json!({}),
                context: RequestContext::new(),
                granted_effects: vec![],
            })
            .await
            .unwrap();
        assert!(matches!(out, CapabilityOutcome::Value(_)));
    }

    // The CKB learned the successes (spec R1.4).
    assert_eq!(ckb.success_rate("fake", "cap1").await, 1.0);
    // And grounding lists exactly the installed capability (no hallucination).
    assert_eq!(ckb.list_installed().await.unwrap().len(), 1);

    // The selector, consulting the CKB, ranks the proven capability and (given a
    // strong lexical/semantic score) chooses to reuse it.
    let selector = DefaultCapabilitySelector::with_default_policy();
    let hits = platform.discover("cap1", 5).unwrap();
    if !hits.is_empty() {
        let selection = selector.select(&hits, platform.knowledge()).await;
        // The learned candidate is present and carries the learned success.
        assert!(selection
            .candidates
            .iter()
            .any(|c| c.descriptor.capability_id == "cap1" && c.learned_success == 1.0));
    }
}

/// A declined outcome must NOT count as a learned success (honest learning).
#[tokio::test]
async fn ckb_does_not_learn_success_on_decline() {
    use super::intelligence::{CapabilityKnowledge, SqliteCapabilityKnowledge};

    let index = test_index();
    let registry = ProviderRegistry::new(index);
    // FakeProvider declines any capability it does not expose.
    registry.register(Arc::new(FakeProvider::new("fake", vec![])));
    let ckb: Arc<dyn CapabilityKnowledge> =
        Arc::new(SqliteCapabilityKnowledge::in_memory().unwrap());
    let platform = CapabilityPlatform::new(Arc::new(registry)).with_knowledge(ckb.clone());
    platform.refresh().await;

    let out = platform
        .execute(CapabilityRequest {
            provider_id: "fake".into(),
            capability_id: "missing".into(),
            args: serde_json::json!({}),
            context: RequestContext::new(),
            granted_effects: vec![],
        })
        .await
        .unwrap();
    assert!(matches!(out, CapabilityOutcome::Declined { .. }));
    // Declined ⇒ recorded as a non-success (rate 0.0), never a fake success.
    assert_eq!(ckb.success_rate("fake", "missing").await, 0.0);
}

// ─── Wave 6: Brain-owned acquisition pipeline (ranking → trust gate → CKB) ────

use super::intelligence::{CatalogRanker, CatalogRankingPolicy, TrustPolicy};
use std::sync::Mutex as StdMutex;

/// A lifecycle-capable in-memory provider: exposes a catalog, and `acquire`
/// installs EXACTLY the Brain-selected `capability_id`, returning a descriptor
/// whose declared trust tier is taken from the catalog entry. Records the last
/// acquired `capability_id` so tests can prove the Brain (not the provider) chose.
struct LifecycleFake {
    id: super::ProviderId,
    catalog: Vec<CapabilityDescriptor>,
    installed: StdMutex<Vec<CapabilityDescriptor>>,
    last_acquired: StdMutex<Option<String>>,
}

impl LifecycleFake {
    fn new(id: &str, catalog: Vec<CapabilityDescriptor>) -> Self {
        Self {
            id: id.into(),
            catalog,
            installed: StdMutex::new(Vec::new()),
            last_acquired: StdMutex::new(None),
        }
    }
}

#[async_trait::async_trait]
impl CapabilityProvider for LifecycleFake {
    fn provider_id(&self) -> &super::ProviderId {
        &self.id
    }
    async fn negotiate(
        &self,
        client: &ClientCapabilities,
    ) -> Result<super::protocol::ProtocolSession, CapError> {
        // Advertise lifecycle so the platform will call acquire.
        Ok(client.negotiate(
            self.id.clone(),
            ProtocolVersion::CURRENT,
            FeatureSet::mandatory().with(Feature::Lifecycle),
            serde_json::Map::new(),
        ))
    }
    async fn describe(
        &self,
        _s: &super::protocol::ProtocolSession,
    ) -> Result<Vec<CapabilityDescriptor>, CapError> {
        Ok(self.installed.lock().unwrap().clone())
    }
    async fn catalog(&self) -> Result<Vec<CapabilityDescriptor>, CapError> {
        Ok(self.catalog.clone())
    }
    async fn acquire(
        &self,
        req: &super::provider::AcquireRequest,
    ) -> Result<CapabilityDescriptor, CapError> {
        let chosen = req
            .capability_id
            .clone()
            .ok_or_else(|| CapError::Acquire("Brain did not select a capability".into()))?;
        *self.last_acquired.lock().unwrap() = Some(chosen.clone());
        let d = self
            .catalog
            .iter()
            .find(|d| d.capability_id == chosen)
            .cloned()
            .ok_or_else(|| CapError::Acquire(format!("no catalog entry '{chosen}'")))?;
        self.installed.lock().unwrap().push(d.clone());
        Ok(d)
    }
    async fn execute(&self, req: CapabilityRequest) -> Result<CapabilityOutcome, CapError> {
        Ok(CapabilityOutcome::Value(
            serde_json::json!({ "ran": req.capability_id }),
        ))
    }
    async fn health(&self) -> ProviderHealth {
        ProviderHealth::Ready
    }
}

fn catalog_desc(provider: &str, cap: &str, desc: &str, tier: &str) -> CapabilityDescriptor {
    let mut d = desc_with(provider, cap, cap, desc, &[]);
    d.version = "1.0.0".to_string();
    d.trust.tier = Some(tier.to_string());
    d
}

fn platform_with_marketplace(provider: LifecycleFake) -> Arc<CapabilityPlatform> {
    let registry = ProviderRegistry::new(test_index());
    registry.register(Arc::new(provider));
    Arc::new(
        CapabilityPlatform::new(Arc::new(registry)).with_marketplace_v2(
            CatalogRanker::new(CatalogRankingPolicy::default()),
            std::time::Duration::from_secs(60),
        ),
    )
}

#[tokio::test]
async fn brain_selects_and_activates_trusted_capability() {
    let provider = LifecycleFake::new(
        "market",
        vec![catalog_desc(
            "market",
            "pdf_ocr",
            "extract text from scanned pdf documents",
            "community",
        )],
    );
    let platform = platform_with_marketplace(provider);
    platform.refresh().await;

    let d = platform
        .acquire_for_goal("extract text from a scanned pdf")
        .await
        .expect("trusted capability should install");
    assert_eq!(d.capability_id, "pdf_ocr");
    assert!(!platform.is_quarantined("market", "pdf_ocr"));
}

#[tokio::test]
async fn brain_quarantines_untrusted_and_blocks_execution() {
    let provider = LifecycleFake::new(
        "market",
        vec![catalog_desc(
            "market",
            "sketchy_tool",
            "do something with files",
            "untrusted",
        )],
    );
    let platform = platform_with_marketplace(provider);
    platform.refresh().await;

    let err = platform
        .acquire_for_goal("do something with files")
        .await
        .expect_err("untrusted capability must be quarantined, not activated");
    assert!(matches!(err, CapError::Permission(_)), "got {err:?}");
    assert!(platform.is_quarantined("market", "sketchy_tool"));

    // Even a direct execute of the quarantined capability must be refused.
    let exec = platform
        .execute(CapabilityRequest {
            provider_id: "market".into(),
            capability_id: "sketchy_tool".into(),
            args: serde_json::json!({}),
            context: RequestContext::new(),
            granted_effects: vec![],
        })
        .await;
    assert!(
        matches!(exec, Err(CapError::Permission(_))),
        "quarantined capability must not execute: {exec:?}"
    );
}

#[tokio::test]
async fn flag_off_acquisition_uses_legacy_path_no_quarantine() {
    // Without marketplace_v2, quarantine is inert and legacy acquisition runs.
    let provider = LifecycleFake::new(
        "market",
        vec![catalog_desc(
            "market",
            "any_tool",
            "do a thing",
            "untrusted",
        )],
    );
    let registry = ProviderRegistry::new(test_index());
    registry.register(Arc::new(provider));
    let platform = Arc::new(CapabilityPlatform::new(Arc::new(registry)));
    platform.refresh().await;

    // Legacy path passes capability_id: None; our LifecycleFake requires a
    // selection, so legacy acquire declines here — proving the reasoned pipeline
    // (which DOES select) is gated strictly behind the flag (parity).
    let res = platform.acquire_for_goal("do a thing").await;
    assert!(res.is_err(), "legacy path does not Brain-select");
    assert!(!platform.is_quarantined("market", "any_tool"));
}
