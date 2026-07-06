//! The single capability object (capability-contract INV-2).
//!
//! One `Capability{kind, mode, scope}` is declared in the bundle manifest, risk-classified by
//! KRIA (never the author), and later materialized by the runtime. A `Capability` is meaningless
//! without a `scope` — there is no "network: true", only "network egress to [domains]".
//!
//! A2 introduces this object and a projection to the legacy `SkillCapabilities` flags used by the
//! current `SkillDescriptor` (the descriptor is a *derived* view — package-contract §6).

use super::types::SkillCapabilities;
use crate::safety::RiskLevel;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    Filesystem,
    Network,
    Subprocess,
    Browser,
    Gpu,
    Clipboard,
    Device,
    Environment,
}

impl std::str::FromStr for CapabilityKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "filesystem" => Ok(Self::Filesystem),
            "network" => Ok(Self::Network),
            "subprocess" => Ok(Self::Subprocess),
            "browser" => Ok(Self::Browser),
            "gpu" => Ok(Self::Gpu),
            "clipboard" => Ok(Self::Clipboard),
            "device" => Ok(Self::Device),
            "environment" | "env" => Ok(Self::Environment),
            other => Err(format!("unknown capability kind: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityMode {
    ReadOnly,
    ReadWrite,
    Egress,
    Execute,
    Use,
}

impl std::str::FromStr for CapabilityMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "read_only" | "readonly" | "read" => Ok(Self::ReadOnly),
            "read_write" | "readwrite" | "write" => Ok(Self::ReadWrite),
            "egress" => Ok(Self::Egress),
            "execute" | "exec" => Ok(Self::Execute),
            "use" => Ok(Self::Use),
            other => Err(format!("unknown capability mode: {other}")),
        }
    }
}

/// A capability is always bounded to a scope. `None` is only valid for kinds that need no target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityScope {
    Workspace,
    InputMount(String),
    Domains(Vec<String>),
    Binaries(Vec<String>),
    /// Allowed environment variable names (kind = Environment).
    EnvVars(Vec<String>),
    None,
}

/// The one capability value object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    pub kind: CapabilityKind,
    pub mode: CapabilityMode,
    pub scope: CapabilityScope,
}

impl Capability {
    /// Whether this single capability is a *widening* relative to `other` of the same kind.
    fn widens(&self, other: &Capability) -> bool {
        // Mode escalation (write > read; execute/egress/use are elevated).
        let elevated = |m: CapabilityMode| {
            matches!(
                m,
                CapabilityMode::ReadWrite | CapabilityMode::Execute | CapabilityMode::Egress
            )
        };
        if elevated(self.mode) && !elevated(other.mode) {
            return true;
        }
        // Scope broadening.
        match (&self.scope, &other.scope) {
            (CapabilityScope::Domains(new), CapabilityScope::Domains(old)) => {
                new.iter().any(|d| d == "*") && !old.iter().any(|d| d == "*")
                    || new.iter().any(|d| !old.contains(d))
            }
            (CapabilityScope::Binaries(new), CapabilityScope::Binaries(old)) => {
                new.iter().any(|d| !old.contains(d))
            }
            _ => false,
        }
    }
}

/// Risk is a pure function of the *granted* capability set — KRIA-owned (capability-contract §4).
pub fn classify_risk(caps: &[Capability]) -> RiskLevel {
    let mut risk = RiskLevel::Green;
    for c in caps {
        let this = match (c.kind, c.mode, &c.scope) {
            // BLACK-adjacent: unrestricted network is forced to RED (blocked without explicit HITL).
            (CapabilityKind::Network, _, CapabilityScope::Domains(d))
                if d.iter().any(|x| x == "*") =>
            {
                RiskLevel::Red
            }
            (CapabilityKind::Filesystem, CapabilityMode::ReadWrite, _)
            | (CapabilityKind::Subprocess, _, _)
            | (CapabilityKind::Device, _, _) => RiskLevel::Red,
            (CapabilityKind::Network, _, _)
            | (CapabilityKind::Browser, _, _)
            | (CapabilityKind::Gpu, _, _) => RiskLevel::Yellow,
            _ => RiskLevel::Green,
        };
        risk = max_risk(risk, this);
    }
    risk
}

fn max_risk(a: RiskLevel, b: RiskLevel) -> RiskLevel {
    fn rank(r: RiskLevel) -> u8 {
        match r {
            RiskLevel::Green => 0,
            RiskLevel::Yellow => 1,
            RiskLevel::Red => 2,
            RiskLevel::Black => 3,
        }
    }
    if rank(a) >= rank(b) {
        a
    } else {
        b
    }
}

/// Returns true if `new_caps` widens any capability vs `old_caps` (⇒ requires re-approval).
pub fn requires_reapproval(old_caps: &[Capability], new_caps: &[Capability]) -> bool {
    for nc in new_caps {
        // A brand-new kind not present before is a widening.
        let matching_old: Vec<&Capability> =
            old_caps.iter().filter(|o| o.kind == nc.kind).collect();
        if matching_old.is_empty() {
            return true;
        }
        if matching_old.iter().all(|o| nc.widens(o)) {
            // widened against every prior grant of this kind
            if matching_old.iter().any(|o| nc.widens(o)) {
                return true;
            }
        } else if matching_old.iter().any(|o| nc.widens(o)) {
            return true;
        }
    }
    false
}

/// Project the capability set to the legacy `SkillCapabilities` bool flags used by
/// `SkillDescriptor`. The manifest capabilities remain the single source of truth; this is a
/// derived view for the descriptor + existing risk plumbing.
pub fn to_legacy(caps: &[Capability]) -> SkillCapabilities {
    let mut out = SkillCapabilities::default();
    for c in caps {
        match c.kind {
            CapabilityKind::Filesystem => {
                out.filesystem_read = true;
                if matches!(c.mode, CapabilityMode::ReadWrite) {
                    out.filesystem_write = true;
                }
            }
            CapabilityKind::Network => {
                out.network = true;
                if let CapabilityScope::Domains(d) = &c.scope {
                    out.network_domains = d.clone();
                }
            }
            CapabilityKind::Subprocess => out.subprocess = true,
            CapabilityKind::Browser => out.browser = true,
            CapabilityKind::Gpu => out.image_generation = true,
            CapabilityKind::Device => out.media = true,
            CapabilityKind::Clipboard => {}
            CapabilityKind::Environment => {}
        }
    }
    out
}

/// Inverse of `to_legacy`: derive a real `Capability` list from the legacy
/// `SkillCapabilities` bool-flag view (capability-grant wiring fix).
///
/// This is used ONLY where a skill's sole capability signal is the coarse
/// legacy flags — today that is the ClawHub marketplace transpile path
/// (`transpiler::transpile_skill`, YAML frontmatter `capabilities:` block has
/// no scope detail). The `.ocskill` bundle path (`bundle::to_descriptor`)
/// already carries the real, scoped `Vec<Capability>` from the manifest and
/// should always be preferred over this lossy projection when available.
///
/// Honest limitation, documented not hidden: legacy flags carry no
/// binary/domain allowlist detail beyond `network_domains`, so `Subprocess`
/// grants here have an empty `Binaries` scope (materializes to an empty
/// `SubprocessAllowlist` — deny-by-default-safe, but does not grant runnable
/// binaries). A skill declaring `subprocess: true` via the legacy flag path
/// is flagged RED (via `classify_risk`) and its grant is real but empty-scoped;
/// full subprocess execution requires the richer manifest capability format.
pub fn from_legacy(caps: &SkillCapabilities) -> Vec<Capability> {
    let mut out = Vec::new();
    if caps.filesystem_read || caps.filesystem_write {
        out.push(Capability {
            kind: CapabilityKind::Filesystem,
            mode: if caps.filesystem_write {
                CapabilityMode::ReadWrite
            } else {
                CapabilityMode::ReadOnly
            },
            scope: CapabilityScope::Workspace,
        });
    }
    if caps.network {
        let domains = if caps.network_domains.is_empty() {
            vec!["*".to_string()]
        } else {
            caps.network_domains.clone()
        };
        out.push(Capability {
            kind: CapabilityKind::Network,
            mode: CapabilityMode::Egress,
            scope: CapabilityScope::Domains(domains),
        });
    }
    if caps.subprocess {
        out.push(Capability {
            kind: CapabilityKind::Subprocess,
            mode: CapabilityMode::Execute,
            scope: CapabilityScope::Binaries(Vec::new()),
        });
    }
    if caps.browser {
        out.push(Capability {
            kind: CapabilityKind::Browser,
            mode: CapabilityMode::Use,
            scope: CapabilityScope::None,
        });
    }
    if caps.image_generation {
        out.push(Capability {
            kind: CapabilityKind::Gpu,
            mode: CapabilityMode::Use,
            scope: CapabilityScope::None,
        });
    }
    if caps.media {
        out.push(Capability {
            kind: CapabilityKind::Device,
            mode: CapabilityMode::Use,
            scope: CapabilityScope::Binaries(Vec::new()),
        });
    }
    out
}

// ─── CapabilityGrant + materialization (A3.1) ────────────────────────────────

/// Where a grant decision came from (capability-contract §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantSource {
    Manifest,
    UserApproval,
    PolicyDefault,
    Generated,
}

/// How the runtime realizes a granted capability (capability-contract §5). This is the bridge
/// between the abstract grant and the concrete container configuration (materialize.rs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Materialization {
    /// Ephemeral tmpfs workspace (always present).
    WorkspaceTmpfs,
    /// Read-only or read-write bind of an approved input path.
    InputMount { id: String, read_only: bool },
    /// Egress permitted only to these domains (via egress network/proxy).
    EgressAllowlist(Vec<String>),
    /// Subprocess execution limited to these binaries.
    SubprocessAllowlist(Vec<String>),
    /// Environment variables (allowlisted names) injected from a broker — never host env.
    EnvAllowlist(Vec<String>),
    /// Brokered browser endpoint (out of container).
    BrokeredBrowser,
    /// GPU device via HRA lease.
    GpuLease,
    /// A device mapping.
    Device(String),
    /// Nothing to materialize.
    None,
}

/// The single runtime permission object. Flows approval → materialize → runtime → audit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityGrant {
    pub capability: Capability,
    pub granted: bool,
    pub source: GrantSource,
    pub materialization: Materialization,
    #[serde(default)]
    pub approved_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl CapabilityGrant {
    /// Derive the materialization for a capability (deny-by-default base is applied separately).
    pub fn materialization_for(cap: &Capability) -> Materialization {
        match (&cap.kind, &cap.scope) {
            (CapabilityKind::Filesystem, CapabilityScope::Workspace) => {
                Materialization::WorkspaceTmpfs
            }
            (CapabilityKind::Filesystem, CapabilityScope::InputMount(id)) => {
                Materialization::InputMount {
                    id: id.clone(),
                    read_only: !matches!(cap.mode, CapabilityMode::ReadWrite),
                }
            }
            (CapabilityKind::Network, CapabilityScope::Domains(d)) => {
                Materialization::EgressAllowlist(d.clone())
            }
            (CapabilityKind::Subprocess, CapabilityScope::Binaries(b)) => {
                Materialization::SubprocessAllowlist(b.clone())
            }
            (CapabilityKind::Environment, CapabilityScope::EnvVars(v)) => {
                Materialization::EnvAllowlist(v.clone())
            }
            (CapabilityKind::Browser, _) => Materialization::BrokeredBrowser,
            (CapabilityKind::Gpu, _) => Materialization::GpuLease,
            (CapabilityKind::Device, CapabilityScope::Binaries(d)) => {
                Materialization::Device(d.first().cloned().unwrap_or_default())
            }
            _ => Materialization::None,
        }
    }
}

/// Grant a set of capabilities from a source (A3.1). This is the single place capabilities become
/// grants; the approval flow (approval.rs) decides `granted`/`approved_at`.
pub fn grant_all(caps: &[Capability], source: GrantSource, granted: bool) -> Vec<CapabilityGrant> {
    caps.iter()
        .map(|c| CapabilityGrant {
            capability: c.clone(),
            granted,
            source,
            materialization: CapabilityGrant::materialization_for(c),
            approved_at: if granted {
                Some(chrono::Utc::now())
            } else {
                None
            },
            expires_at: None,
        })
        .collect()
}

/// Extract the capabilities from a grant set (for risk/hashing/reapproval).
pub fn capabilities_of(grants: &[CapabilityGrant]) -> Vec<Capability> {
    grants.iter().map(|g| g.capability.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(kind: CapabilityKind, mode: CapabilityMode, scope: CapabilityScope) -> Capability {
        Capability { kind, mode, scope }
    }

    #[test]
    fn materialization_mapping() {
        let net = cap(
            CapabilityKind::Network,
            CapabilityMode::Egress,
            CapabilityScope::Domains(vec!["a.com".into()]),
        );
        assert_eq!(
            CapabilityGrant::materialization_for(&net),
            Materialization::EgressAllowlist(vec!["a.com".into()])
        );
        let fs = cap(
            CapabilityKind::Filesystem,
            CapabilityMode::ReadOnly,
            CapabilityScope::InputMount("docs".into()),
        );
        assert_eq!(
            CapabilityGrant::materialization_for(&fs),
            Materialization::InputMount {
                id: "docs".into(),
                read_only: true
            }
        );
        let env = cap(
            CapabilityKind::Environment,
            CapabilityMode::Use,
            CapabilityScope::EnvVars(vec!["API_KEY".into()]),
        );
        assert_eq!(
            CapabilityGrant::materialization_for(&env),
            Materialization::EnvAllowlist(vec!["API_KEY".into()])
        );
    }

    #[test]
    fn grant_all_sets_materialization() {
        let caps = vec![cap(
            CapabilityKind::Gpu,
            CapabilityMode::Use,
            CapabilityScope::None,
        )];
        let grants = grant_all(&caps, GrantSource::Manifest, true);
        assert_eq!(grants.len(), 1);
        assert!(grants[0].granted);
        assert_eq!(grants[0].materialization, Materialization::GpuLease);
    }

    #[test]
    fn risk_classification() {
        assert_eq!(
            classify_risk(&[cap(
                CapabilityKind::Filesystem,
                CapabilityMode::ReadOnly,
                CapabilityScope::Workspace
            )]),
            RiskLevel::Green
        );
        assert_eq!(
            classify_risk(&[cap(
                CapabilityKind::Network,
                CapabilityMode::Egress,
                CapabilityScope::Domains(vec!["api.example.com".into()])
            )]),
            RiskLevel::Yellow
        );
        assert_eq!(
            classify_risk(&[cap(
                CapabilityKind::Subprocess,
                CapabilityMode::Execute,
                CapabilityScope::Binaries(vec!["ffmpeg".into()])
            )]),
            RiskLevel::Red
        );
        assert_eq!(
            classify_risk(&[cap(
                CapabilityKind::Network,
                CapabilityMode::Egress,
                CapabilityScope::Domains(vec!["*".into()])
            )]),
            RiskLevel::Red
        );
    }

    #[test]
    fn reapproval_on_widening() {
        let old = vec![cap(
            CapabilityKind::Network,
            CapabilityMode::Egress,
            CapabilityScope::Domains(vec!["a.com".into()]),
        )];
        let widened = vec![cap(
            CapabilityKind::Network,
            CapabilityMode::Egress,
            CapabilityScope::Domains(vec!["a.com".into(), "b.com".into()]),
        )];
        assert!(requires_reapproval(&old, &widened));

        let narrowed = vec![cap(
            CapabilityKind::Network,
            CapabilityMode::Egress,
            CapabilityScope::Domains(vec!["a.com".into()]),
        )];
        assert!(!requires_reapproval(&old, &narrowed));

        let new_kind = vec![cap(
            CapabilityKind::Subprocess,
            CapabilityMode::Execute,
            CapabilityScope::Binaries(vec!["ls".into()]),
        )];
        assert!(requires_reapproval(&old, &new_kind));
    }

    #[test]
    fn legacy_projection() {
        let caps = vec![
            cap(
                CapabilityKind::Filesystem,
                CapabilityMode::ReadWrite,
                CapabilityScope::Workspace,
            ),
            cap(
                CapabilityKind::Network,
                CapabilityMode::Egress,
                CapabilityScope::Domains(vec!["x.com".into()]),
            ),
        ];
        let legacy = to_legacy(&caps);
        assert!(legacy.filesystem_read && legacy.filesystem_write);
        assert!(legacy.network);
        assert_eq!(legacy.network_domains, vec!["x.com".to_string()]);
    }
}
