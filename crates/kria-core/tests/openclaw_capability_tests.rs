//! Phase A3 — Capability Enforcement integration tests.
//!
//! Exercises the full declare → CapabilityGrant → materialization pipeline (manifest capabilities
//! becoming ACTUAL container restrictions), plus approval (escalation/reduction), revocation, and
//! capability events. Container *execution* is Docker/hardware; everything here validates that a
//! declared capability deterministically produces the correct enforced runtime config.

use kria_core::openclaw::approval::{ApprovalCache, ApprovalDecision};
use kria_core::openclaw::bundle::manifest::Manifest;
use kria_core::openclaw::capability::{
    self, Capability, CapabilityKind, CapabilityMode, CapabilityScope, GrantSource,
};
use kria_core::openclaw::event::{self, CapabilityAction};
use kria_core::openclaw::materialize::{self, EnvProvider, NullEnvProvider, ResourceLimits};
use kria_core::openclaw::revocation;
use kria_core::openclaw::types::ResourceClass;
use kria_core::safety::RiskLevel;
use std::collections::HashMap;
use std::path::Path;
use tokio_util::sync::CancellationToken;

fn manifest_with(caps_toml: &str) -> Manifest {
    let toml = format!(
        r#"
[skill]
slug = "oc_cap"
name = "Cap Test"
version = "1.0.0"
category = "productivity"
description = "capability test"
min_kria = "0.1.0"
[runtime]
kind = "docker"
entry = "handler/x.js"
[resource]
class = "light"
[trust]
declared_tier = "community"
publisher = "did:key:zTEST"
{caps_toml}
"#
    );
    Manifest::parse(&toml).unwrap()
}

fn grants_for(caps_toml: &str) -> Vec<kria_core::openclaw::capability::CapabilityGrant> {
    let m = manifest_with(caps_toml);
    let caps = m.validate().unwrap();
    capability::grant_all(&caps, GrantSource::Manifest, true)
}

fn limits() -> ResourceLimits {
    ResourceLimits::for_class(ResourceClass::Light)
}

struct MapEnv(HashMap<String, String>);
impl EnvProvider for MapEnv {
    fn get(&self, name: &str) -> Option<String> {
        self.0.get(name).cloned()
    }
}

// ── Filesystem ────────────────────────────────────────────────────────────────

#[test]
fn filesystem_readonly_enforced_and_no_blanket_mount() {
    let grants = grants_for(
        "[[capabilities]]\nkind = \"filesystem\"\nmode = \"read_only\"\nscope = \"input:docs\"\n",
    );
    let m = materialize::build(
        "img",
        vec!["node".into()],
        &grants,
        &limits(),
        &NullEnvProvider,
        Some(Path::new("/host")),
    );
    let binds = m.config.host_config.unwrap().binds.unwrap();
    assert_eq!(binds, vec!["/host/docs:/inputs/docs:ro".to_string()]);
}

#[test]
fn filesystem_write_mount_is_rw() {
    let grants = grants_for(
        "[[capabilities]]\nkind = \"filesystem\"\nmode = \"read_write\"\nscope = \"input:out\"\n",
    );
    let m = materialize::build(
        "img",
        vec!["node".into()],
        &grants,
        &limits(),
        &NullEnvProvider,
        Some(Path::new("/host")),
    );
    let binds = m.config.host_config.unwrap().binds.unwrap();
    assert!(binds[0].ends_with(":rw"));
}

#[test]
fn no_filesystem_grant_means_no_host_mounts() {
    let m = materialize::build(
        "img",
        vec!["node".into()],
        &[],
        &limits(),
        &NullEnvProvider,
        None,
    );
    assert!(
        m.config.host_config.unwrap().binds.is_none(),
        "host path blocked by default"
    );
}

// ── Network ─────────────────────────────────────────────────────────────────

#[test]
fn network_denied_by_default() {
    let m = materialize::build(
        "img",
        vec!["node".into()],
        &[],
        &limits(),
        &NullEnvProvider,
        None,
    );
    assert_eq!(m.network_mode, "none");
}

#[test]
fn network_allowlist_enables_and_records_domains() {
    let grants = grants_for(
        "[[capabilities]]\nkind = \"network\"\nmode = \"egress\"\nscope = [\"api.example.com\", \"cdn.example.com\"]\n",
    );
    let m = materialize::build(
        "img",
        vec!["node".into()],
        &grants,
        &limits(),
        &NullEnvProvider,
        None,
    );
    assert_eq!(m.network_mode, "bridge");
    assert_eq!(
        m.egress_allowlist,
        vec!["api.example.com".to_string(), "cdn.example.com".to_string()]
    );
}

// ── Environment / secrets ──────────────────────────────────────────────────────

#[test]
fn env_only_allowlisted_and_secret_injected_from_broker() {
    let grants = grants_for(
        "[[capabilities]]\nkind = \"environment\"\nmode = \"use\"\nscope = [\"API_KEY\", \"UNSET\"]\n",
    );
    let mut map = HashMap::new();
    map.insert("API_KEY".to_string(), "s3cr3t".to_string());
    // Also seed a host-like var that is NOT allowlisted → must never appear.
    map.insert("HOST_SECRET".to_string(), "leak".to_string());
    let m = materialize::build(
        "img",
        vec!["node".into()],
        &grants,
        &limits(),
        &MapEnv(map),
        None,
    );
    let env = m.config.env.unwrap();
    assert_eq!(env, vec!["API_KEY=s3cr3t".to_string()]);
    assert!(
        !env.iter().any(|e| e.contains("HOST_SECRET")),
        "non-allowlisted var blocked"
    );
}

#[test]
fn no_env_grant_means_empty_container_env() {
    let m = materialize::build(
        "img",
        vec!["node".into()],
        &[],
        &limits(),
        &NullEnvProvider,
        None,
    );
    assert!(m.config.env.is_none(), "no unrestricted host environment");
}

// ── Resources ──────────────────────────────────────────────────────────────

#[test]
fn resource_limits_enforced_per_class() {
    let m = materialize::build(
        "img",
        vec!["node".into()],
        &[],
        &limits(),
        &NullEnvProvider,
        None,
    );
    let hc = m.config.host_config.unwrap();
    assert_eq!(hc.memory, Some(256 * 1024 * 1024));
    assert_eq!(hc.nano_cpus, Some(500_000_000));
    assert_eq!(hc.pids_limit, Some(128));
    assert_eq!(hc.readonly_rootfs, Some(true));
    assert_eq!(hc.cap_drop, Some(vec!["ALL".to_string()]));
}

#[test]
fn gpu_denied_by_default_granted_when_declared() {
    let none = materialize::build(
        "img",
        vec!["node".into()],
        &[],
        &limits(),
        &NullEnvProvider,
        None,
    );
    assert!(!none.needs_gpu);
    let grants = grants_for("[[capabilities]]\nkind = \"gpu\"\nmode = \"use\"\n");
    let with = materialize::build(
        "img",
        vec!["node".into()],
        &grants,
        &limits(),
        &NullEnvProvider,
        None,
    );
    assert!(with.needs_gpu);
}

// ── Approval (escalation / reduction) ──────────────────────────────────────────

fn net(scope: Vec<&str>) -> Capability {
    Capability {
        kind: CapabilityKind::Network,
        mode: CapabilityMode::Egress,
        scope: CapabilityScope::Domains(scope.into_iter().map(String::from).collect()),
    }
}

#[test]
fn capability_escalation_rejected() {
    let cache = ApprovalCache::new();
    let old = vec![net(vec!["a.com"])];
    let widened = vec![net(vec!["a.com", "b.com"])];
    let d = cache.evaluate(
        "oc_cap",
        "1.1.0",
        &widened,
        Some(&old),
        "light",
        "1",
        RiskLevel::Yellow,
    );
    assert!(
        matches!(d, ApprovalDecision::NeedsHitl(_)),
        "widening requires re-approval"
    );
}

#[test]
fn capability_reduction_accepted() {
    let cache = ApprovalCache::new();
    let old = vec![net(vec!["a.com", "b.com"])];
    let reduced = vec![net(vec!["a.com"])];
    let d = cache.evaluate(
        "oc_cap",
        "1.1.0",
        &reduced,
        Some(&old),
        "light",
        "1",
        RiskLevel::Yellow,
    );
    assert!(
        matches!(d, ApprovalDecision::AutoApproved(_)),
        "narrowing never needs approval"
    );
}

// ── Revocation ─────────────────────────────────────────────────────────────

#[test]
fn grant_revoke_cancels_execution() {
    let token = CancellationToken::new();
    let _guard = revocation::register("oc_cap", "exec-x", token.clone());
    assert_eq!(revocation::revoke("oc_cap"), 1);
    assert!(
        token.is_cancelled(),
        "revocation cancels the in-flight execution token"
    );
}

// ── Events ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn capability_events_on_single_stream() {
    let mut rx = event::subscribe();
    event::emit_capability(
        "corr-1",
        "exec-1",
        "oc_cap",
        CapabilityAction::Granted,
        "hash123",
        Some("did:key:zTEST".into()),
        Some("yellow".into()),
        None,
    );
    let ev = rx.recv().await.unwrap();
    let cap = ev.capability.expect("capability payload present");
    assert_eq!(cap.action, CapabilityAction::Granted);
    assert_eq!(cap.capability_hash, "hash123");
    assert_eq!(cap.publisher.as_deref(), Some("did:key:zTEST"));
}

// ── Full pipeline: declaration == enforcement ──────────────────────────────────

#[test]
fn declared_capabilities_become_exact_enforcement() {
    let caps_toml = r#"
[[capabilities]]
kind = "filesystem"
mode = "read_only"
scope = "input:data"

[[capabilities]]
kind = "network"
mode = "egress"
scope = ["api.example.com"]

[[capabilities]]
kind = "environment"
mode = "use"
scope = ["TOKEN"]
"#;
    let grants = grants_for(caps_toml);
    // Risk classification is derived from the granted set (KRIA-owned).
    let caps = capability::capabilities_of(&grants);
    assert_eq!(capability::classify_risk(&caps), RiskLevel::Yellow);

    let mut env = HashMap::new();
    env.insert("TOKEN".to_string(), "abc".to_string());
    let m = materialize::build(
        "img",
        vec!["node".into()],
        &grants,
        &limits(),
        &MapEnv(env),
        Some(Path::new("/host")),
    );

    let hc = m.config.host_config.unwrap();
    // Filesystem: exactly one scoped read-only mount.
    assert_eq!(
        hc.binds.unwrap(),
        vec!["/host/data:/inputs/data:ro".to_string()]
    );
    // Network: enabled + allowlist recorded.
    assert_eq!(m.network_mode, "bridge");
    assert_eq!(m.egress_allowlist, vec!["api.example.com".to_string()]);
    // Env: only the allowlisted broker-supplied var.
    assert_eq!(m.config.env.unwrap(), vec!["TOKEN=abc".to_string()]);
    // Base lockdown still applied.
    assert_eq!(hc.readonly_rootfs, Some(true));
    assert_eq!(hc.pids_limit, Some(128));
}
