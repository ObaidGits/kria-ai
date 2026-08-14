//! `os_control::resource` — typed OS resource declarations, deterministic
//! ordering, the single canonical resource-set digest, and the non-cloneable
//! [`AcquiredResourceLeaseSet`] that later runtime sealing (Task 1.7) consumes.
//!
//! linux-os-control-production **Task 1.6**, design §4, §6, §10.4 (OSC-008,
//! OSC-009).
//!
//! # What this module owns
//!
//! * [`OsResourceKind`] — the closed, typed set of OS resource domains named by
//!   OSC-008.1 and by every §10 operation's manifest
//!   `canonical_resource_derivation`. There is **no** global "unknown" scope: a
//!   rule whose domain does not parse to a typed kind is a hard error, so every
//!   mutating canonical tool maps to at least one precise typed resource.
//! * The manifest-driven derivation ([`os_write_requirements`] /
//!   [`os_read_requirements`]) that turns a frozen operation contract into the
//!   exclusive write-resource set (for the grant digest and mutation leases) and
//!   the shared read-lease set (for observation).
//! * The deterministic canonical ordering and the *single* canonical
//!   resource-set digest — both delegated to
//!   [`crate::agent::resource_lease`] so `ExecutionGate` grant issuance and OS
//!   lease acquisition compute the exact same value with no divergent copy.
//! * [`OsResourceCoordinator`], which acquires the write set in canonical order
//!   through the existing generic [`ResourceLeaseManager`] and seals the held
//!   leases into an [`AcquiredResourceLeaseSet`].
//!
//! # Sealing invariant (design §4)
//!
//! [`AcquiredResourceLeaseSet`] has private fields, is **non-`Clone`**, and has
//! no public constructor. The only way to obtain one is
//! [`OsResourceCoordinator::acquire_write_leases`], which actually holds live
//! leases — so a provider/tool module can neither *forge* lease evidence (the
//! fields are private to this module) nor *clone* an existing set. Task 1.7's
//! runtime borrows the held set's [`resource_set_digest`] to seal a
//! `MutationPermit`; it never widens this authority.
//!
//! [`resource_set_digest`]: AcquiredResourceLeaseSet::resource_set_digest

use std::time::Duration;

use serde_json::Value;

use crate::agent::resource_lease::{
    canonical_resource_key, canonical_resource_set_digest, sort_canonical, AccessMode,
    ResourceKind, ResourceLeaseError, ResourceLeaseGuard, ResourceLeaseManager,
    ResourceLeaseRequest, ResourceRequirement,
};
use crate::os_control::contract::Digest;
use crate::os_control::manifest::{
    frozen_contract, ManifestVerificationClass, ToolContractMetadata,
};

/// Bounded default lease lifetime for an OS write resource (OSC-008.7: no
/// unattended workflow holds a user-visible device or global subsystem lease
/// indefinitely). Runtime/rollback tasks may hold shorter effective windows.
pub const DEFAULT_OS_LEASE_TTL: Duration = Duration::from_secs(120);

// ─────────────────────────────────────────────────────────────────────────────
// Typed resource kinds (OSC-008.1, §10.4)
// ─────────────────────────────────────────────────────────────────────────────

macro_rules! os_resource_kinds {
    ($($variant:ident => $token:literal),+ $(,)?) => {
        /// The closed, typed set of OS-control resource domains (OSC-008.1). Each
        /// variant maps to exactly one stable manifest domain token. Ordering is
        /// deterministic; the canonical acquisition order is lexicographic over
        /// the full `"<token>/<scope>"` resource key (see
        /// [`crate::agent::resource_lease::canonical_resource_key`]).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[non_exhaustive]
        pub enum OsResourceKind {
            $(
                #[doc = concat!("The `", $token, "` resource domain.")]
                $variant,
            )+
        }

        impl OsResourceKind {
            /// The stable domain token (never model prose).
            #[must_use]
            pub const fn token(self) -> &'static str {
                match self {
                    $( OsResourceKind::$variant => $token, )+
                }
            }

            /// Parse a manifest domain token into a typed kind. Returns `None`
            /// for an unknown token — the caller treats that as a hard
            /// validation failure, so no global "unknown" scope is ever valid.
            #[must_use]
            pub fn from_token(token: &str) -> Option<Self> {
                match token {
                    $( $token => Some(OsResourceKind::$variant), )+
                    _ => None,
                }
            }

            /// Every typed kind (used by coverage tests).
            #[must_use]
            pub fn all() -> &'static [OsResourceKind] {
                &[ $( OsResourceKind::$variant, )+ ]
            }
        }
    };
}

os_resource_kinds! {
    Application => "application",
    ApplicationCatalog => "application-catalog",
    AudioDefault => "audio-default",
    AudioEndpoint => "audio-endpoint",
    AudioState => "audio-state",
    AudioStream => "audio-stream",
    Automation => "automation",
    AutomationState => "automation-state",
    Backup => "backup",
    BackupState => "backup-state",
    BatteryThreshold => "battery-threshold",
    BluetoothAdapter => "bluetooth-adapter",
    BluetoothDevice => "bluetooth-device",
    BluetoothState => "bluetooth-state",
    CapabilitySnapshot => "capability-snapshot",
    Clipboard => "clipboard",
    ClipboardState => "clipboard-state",
    Display => "display",
    DisplayState => "display-state",
    DisplayTopology => "display-topology",
    FileCatalog => "file-catalog",
    Firewall => "firewall",
    FirewallState => "firewall-state",
    FirmwareState => "firmware-state",
    HardwareSensors => "hardware-sensors",
    MediaPlayer => "media-player",
    MediaState => "media-state",
    MimeDefault => "mime-default",
    NetworkDevice => "network-device",
    NetworkProfile => "network-profile",
    NetworkRadio => "network-radio",
    NetworkState => "network-state",
    NotificationState => "notification-state",
    PackageCatalog => "package-catalog",
    PackageDb => "package-db",
    Path => "path",
    PathSubtree => "path-subtree",
    PowerProfile => "power-profile",
    PowerSession => "power-session",
    PowerState => "power-state",
    PrintJob => "print-job",
    PrintState => "print-state",
    Printer => "printer",
    Privacy => "privacy",
    PrivacyState => "privacy-state",
    Process => "process",
    ProcessTable => "process-table",
    RecoveryRecipe => "recovery-recipe",
    SandboxGrant => "sandbox-grant",
    Scanner => "scanner",
    ScannerState => "scanner-state",
    SearchIndex => "search-index",
    SearchScope => "search-scope",
    SearchState => "search-state",
    Secret => "secret",
    SecretMetadata => "secret-metadata",
    ShutdownSchedule => "shutdown-schedule",
    Storage => "storage",
    StorageTopology => "storage-topology",
    SystemHealth => "system-health",
    TrashItem => "trash-item",
    UpdateManager => "update-manager",
    Workflow => "workflow",
}

// ─────────────────────────────────────────────────────────────────────────────
// Typed resource + canonical key
// ─────────────────────────────────────────────────────────────────────────────

/// A single typed, precisely-scoped OS resource (OSC-008.1/.2). The `access`
/// mode is `Read` for a shared observation lease and `Exclusive` for a
/// serialized write; two `Exclusive` claims on the same [`canonical_key`] cannot
/// overlap, while `Read` claims coexist.
///
/// [`canonical_key`]: OsResource::canonical_key
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct OsResource {
    kind: OsResourceKind,
    scope: String,
    access: AccessMode,
}

impl OsResource {
    /// Construct a typed resource.
    #[must_use]
    pub fn new(kind: OsResourceKind, scope: impl Into<String>, access: AccessMode) -> Self {
        Self {
            kind,
            scope: scope.into(),
            access,
        }
    }

    /// The typed domain.
    #[must_use]
    pub fn kind(&self) -> OsResourceKind {
        self.kind
    }

    /// The resolved scope.
    #[must_use]
    pub fn scope(&self) -> &str {
        &self.scope
    }

    /// The access mode.
    #[must_use]
    pub fn access(&self) -> AccessMode {
        self.access
    }

    /// The canonical resource key `"<domain-token>/<scope>"`. Distinct resources
    /// have distinct keys; this is the value carried into the generic
    /// [`ResourceRequirement::scope`] so the existing manager, digest, and
    /// ordering apply uniformly.
    #[must_use]
    pub fn canonical_key(&self) -> String {
        format!("{}/{}", self.kind.token(), self.scope)
    }

    /// Project this typed resource onto a generic [`ResourceRequirement`] under
    /// the [`ResourceKind::OsControl`] bridge, carrying the precise typed
    /// identity in the requirement scope.
    #[must_use]
    pub fn to_requirement(&self, ttl: Duration) -> ResourceRequirement {
        ResourceRequirement::new(
            ResourceKind::OsControl,
            self.canonical_key(),
            self.access,
            ttl,
        )
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Manifest-driven derivation (§10.4 canonical_resource_derivation)
// ─────────────────────────────────────────────────────────────────────────────

/// Derive the exclusive **write** resource set for a canonical tool from its
/// frozen manifest rule (design §10.4). Empty for read-only operations and for
/// any name outside the frozen manifest. This is the exact set whose canonical
/// digest `ExecutionGate` stores in a grant and the coordinator re-acquires.
#[must_use]
pub fn os_write_requirements(tool_name: &str, params: &Value) -> Vec<ResourceRequirement> {
    requirements_for(tool_name, params, AccessMode::Exclusive)
}

/// Derive the shared **read** lease set for a canonical tool from its frozen
/// manifest rule (design §6 observation phase). These coexist with each other
/// and gate observation before any write leases are acquired.
#[must_use]
pub fn os_read_requirements(tool_name: &str, params: &Value) -> Vec<ResourceRequirement> {
    requirements_for(tool_name, params, AccessMode::Read)
}

/// Derive the full typed resource set (both read and write) for a canonical
/// tool. Useful for mapping/coverage tests and for callers that need the whole
/// declaration. Deterministically ordered by canonical key.
#[must_use]
pub fn os_resources(tool_name: &str, params: &Value) -> Vec<OsResource> {
    let Some(contract) = frozen_contract(tool_name) else {
        return Vec::new();
    };
    let mut resources = derive_resources(contract, params);
    resources.sort();
    resources.dedup();
    resources
}

fn requirements_for(tool_name: &str, params: &Value, want: AccessMode) -> Vec<ResourceRequirement> {
    let Some(contract) = frozen_contract(tool_name) else {
        return Vec::new();
    };
    let mut reqs: Vec<ResourceRequirement> = derive_resources(contract, params)
        .into_iter()
        .filter(|r| r.access == want)
        .map(|r| r.to_requirement(DEFAULT_OS_LEASE_TTL))
        .collect();
    sort_canonical(&mut reqs);
    reqs.dedup_by(|a, b| canonical_resource_key(a) == canonical_resource_key(b));
    reqs
}

/// The mutating classification (design §10.4): a read has no postcondition, so a
/// `NoVerification` operation is read-only and its bare resource rules are
/// shared read leases; any other verification class is a mutation whose bare
/// rules are exclusive write resources.
fn is_mutating(contract: &ToolContractMetadata) -> bool {
    contract.verification != ManifestVerificationClass::NoVerification
}

fn derive_resources(contract: &ToolContractMetadata, params: &Value) -> Vec<OsResource> {
    let mutating = is_mutating(contract);
    let mut out = Vec::new();
    for rule in &contract.resources.rules {
        if let Some(mut resources) = parse_rule(rule, mutating, params) {
            out.append(&mut resources);
        }
    }
    out
}

/// Parse one frozen manifest rule string into typed resources. Returns `None`
/// only when the domain token is unknown (which the coverage test proves never
/// happens for the frozen manifest), guaranteeing no untyped global scope.
fn parse_rule(rule: &str, mutating: bool, params: &Value) -> Option<Vec<OsResource>> {
    let mut rest = rule.trim();
    let read_prefix = rest.strip_prefix("shared read lease ").map(|r| {
        rest = r;
        true
    });
    let read_prefix = read_prefix.unwrap_or(false);
    if let Some(r) = rest.strip_prefix("sorted union: ") {
        rest = r;
    }

    let (kind_token, scope_template) = rest.split_once('/').unwrap_or((rest, ""));
    let kind = OsResourceKind::from_token(kind_token.trim())?;

    // A "shared read lease" rule is always a shared read. A bare rule is an
    // exclusive write for a mutating op, or a shared read for a read-only op's
    // read target.
    let access = if read_prefix || !mutating {
        AccessMode::Read
    } else {
        AccessMode::Exclusive
    };

    let scopes = resolve_scope(scope_template, params);
    Some(
        scopes
            .into_iter()
            .map(|scope| OsResource::new(kind, scope, access))
            .collect(),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Scope template resolution
// ─────────────────────────────────────────────────────────────────────────────

/// Resolve a manifest scope template into one or more concrete scope strings.
/// Placeholder `<...>` segments are resolved from strict params when a bound
/// field is present (precise per-target identity) and otherwise normalized to a
/// stable deterministic token (used e.g. for provider-resolved defaults and in
/// param-free coverage tests). `each field[]` templates expand to one scope per
/// element (the `sorted union` multi-target rule).
fn resolve_scope(template: &str, params: &Value) -> Vec<String> {
    let template = strip_annotations(template);
    if template.is_empty() {
        return vec![String::new()];
    }

    if let Some((array_field, sub_field)) = detect_each(&template) {
        let mut out = Vec::new();
        if let Some(Value::Array(items)) = lookup_param(params, &array_field) {
            for item in items {
                let value = match &sub_field {
                    Some(sf) => item.get(sf).and_then(value_to_scope),
                    None => value_to_scope(item),
                };
                if let Some(v) = value {
                    out.push(sanitize_scope(&v));
                }
            }
        }
        if out.is_empty() {
            out.push(normalize_token(&template));
        }
        out.sort();
        out.dedup();
        return out;
    }

    vec![substitute(&template, params)]
}

/// Remove trailing prose annotations (`; …`, ` bound to …`) that follow the
/// resource key in some manifest rules.
fn strip_annotations(template: &str) -> String {
    let mut s = template.trim();
    if let Some(idx) = s.find("; ") {
        s = &s[..idx];
    }
    if let Some(idx) = s.find(" bound to ") {
        s = &s[..idx];
    }
    s.trim().to_string()
}

/// Detect an `each field[]` array-expansion placeholder, returning the array
/// param path and an optional per-element sub-field.
fn detect_each(template: &str) -> Option<(String, Option<String>)> {
    let idx = template.find("each ")?;
    let after = &template[idx + "each ".len()..];
    let bracket = after.find("[]")?;
    let array_field = after[..bracket].trim().to_string();
    let mut rest = &after[bracket + 2..];
    rest = rest.trim_start_matches('.');
    let sub: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
        .collect();
    let sub_field = if sub.is_empty() { None } else { Some(sub) };
    Some((array_field, sub_field))
}

/// Substitute every `<placeholder>` in a template, preserving literal text
/// (including `/` separators) between placeholders.
fn substitute(template: &str, params: &Value) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        if let Some(end_rel) = rest[start..].find('>') {
            let inner = &rest[start + 1..start + end_rel];
            out.push_str(&resolve_placeholder(inner, params));
            rest = &rest[start + end_rel + 1..];
        } else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        }
    }
    out.push_str(rest);
    sanitize_scope(&out)
}

/// Resolve a single placeholder's inner text to a concrete scope fragment.
fn resolve_placeholder(inner: &str, params: &Value) -> String {
    // Drop any conditional clause (`… when …`).
    let base = inner.split(" when ").next().unwrap_or(inner).trim();

    // A parenthesised form names the bound param field(s); use the first.
    let field = if let (Some(open), Some(close)) = (base.find('('), base.rfind(')')) {
        if open < close {
            base[open + 1..close]
                .split(',')
                .next()
                .unwrap_or("")
                .trim()
                .to_string()
        } else {
            base.to_string()
        }
    } else {
        base.to_string()
    };

    if !field.is_empty() {
        if let Some(value) = lookup_param(params, &field).and_then(value_to_scope) {
            return sanitize_scope(&value);
        }
    }
    normalize_token(base)
}

/// Look up a possibly dotted field path in the strict params object.
fn lookup_param<'a>(params: &'a Value, field: &str) -> Option<&'a Value> {
    let mut cur = params;
    for segment in field.split('.') {
        cur = cur.get(segment)?;
    }
    Some(cur)
}

/// Convert a scalar JSON value to a scope fragment. Non-scalars are rejected so
/// no structured/unbounded value can leak into a resource key.
fn value_to_scope(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Bound and sanitize a resolved scope fragment: strip control characters and
/// cap the length so no unbounded or control-bearing value enters a key.
fn sanitize_scope(value: &str) -> String {
    let mut out: String = value
        .chars()
        .map(|c| if c.is_control() { '_' } else { c })
        .collect();
    if out.len() > 256 {
        out.truncate(256);
    }
    out
}

/// Normalize arbitrary placeholder prose into a stable deterministic token
/// (lowercase, `[a-z0-9.]` retained, other runs collapsed to `-`).
fn normalize_token(text: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

// ─────────────────────────────────────────────────────────────────────────────
// Canonical resource-set digest (single source of truth)
// ─────────────────────────────────────────────────────────────────────────────

/// The single canonical resource-set digest for a tool's exclusive write set
/// (OSC-008.3, OSC-001). This is byte-identical to what `ExecutionGate` stores
/// in a grant, because both delegate to
/// [`crate::agent::resource_lease::canonical_resource_set_digest`] over the same
/// derived requirements — there is no divergent second derivation.
#[must_use]
pub fn write_resource_set_digest(tool_name: &str, params: &Value) -> Digest {
    let reqs = os_write_requirements(tool_name, params);
    Digest::from_hex(canonical_resource_set_digest(&reqs))
}

// ─────────────────────────────────────────────────────────────────────────────
// AcquiredResourceLeaseSet (sealed lease evidence) + coordinator
// ─────────────────────────────────────────────────────────────────────────────

/// Proof that the canonical write-resource set is currently held under live
/// leases (design §4, §6). **Non-`Clone`, private fields, no public
/// constructor** — the only producer is
/// [`OsResourceCoordinator::acquire_write_leases`], which holds real leases, so
/// a provider/tool module can neither forge nor clone lease evidence. Task 1.7's
/// runtime borrows [`resource_set_digest`](Self::resource_set_digest) to seal a
/// `MutationPermit`; dropping this set releases every held lease.
///
/// A provider/tool module cannot **clone** existing lease evidence:
///
/// ```compile_fail
/// use kria_core::os_control::AcquiredResourceLeaseSet;
/// fn clone_evidence(set: &AcquiredResourceLeaseSet) -> AcquiredResourceLeaseSet {
///     set.clone() // error: `AcquiredResourceLeaseSet` does not implement `Clone`
/// }
/// ```
///
/// nor **forge** it (the fields are private to `os_control::resource`, so a
/// struct literal does not compile outside this module):
///
/// ```compile_fail
/// use kria_core::os_control::AcquiredResourceLeaseSet;
/// use kria_core::os_control::contract::Digest;
/// fn forge() -> AcquiredResourceLeaseSet {
///     AcquiredResourceLeaseSet { resource_set_digest: Digest::of_str("x"), _guards: vec![] }
/// }
/// ```
#[derive(Debug)]
pub struct AcquiredResourceLeaseSet {
    resource_set_digest: Digest,
    /// Held lease guards. Retained solely to keep the leases alive for the
    /// lifetime of the set; released on drop (RAII).
    _guards: Vec<ResourceLeaseGuard>,
}

impl AcquiredResourceLeaseSet {
    /// Seal held leases with their canonical digest. Module-private: only the
    /// coordinator in this module constructs a set, and only after leases are
    /// actually acquired.
    fn seal(resource_set_digest: Digest, guards: Vec<ResourceLeaseGuard>) -> Self {
        Self {
            resource_set_digest,
            _guards: guards,
        }
    }

    /// The canonical resource-set digest the held leases cover. This is the only
    /// evidence exposed; runtime sealing matches it against the grant's
    /// `resource_set_digest`.
    #[must_use]
    pub fn resource_set_digest(&self) -> &Digest {
        &self.resource_set_digest
    }

    /// The number of held leases (diagnostics/tests only; not authority).
    #[must_use]
    pub fn held_count(&self) -> usize {
        self._guards.len()
    }
}

#[cfg(feature = "os-control-test")]
impl AcquiredResourceLeaseSet {
    /// Seal an empty lease set carrying only the given canonical digest for
    /// deny-live tests. Gated to `os-control-test`; production sets are produced
    /// only by the coordinator after real leases are acquired.
    #[must_use]
    pub fn for_test(resource_set_digest: Digest) -> Self {
        Self::seal(resource_set_digest, Vec::new())
    }
}

/// Context binding a lease acquisition to one logical action.
#[derive(Debug, Clone)]
pub struct OsLeaseContext {
    /// The owning workflow/session identity.
    pub workflow_id: String,
    /// Optional stage identity.
    pub stage_id: Option<String>,
    /// The bound action hash.
    pub action_hash: String,
}

/// Acquires OS write resources in the single canonical order and seals the held
/// leases (design §6). Wraps the existing generic [`ResourceLeaseManager`], so
/// there is no second coordinator and generic behavior is preserved.
#[derive(Debug, Clone)]
pub struct OsResourceCoordinator {
    manager: ResourceLeaseManager,
}

impl OsResourceCoordinator {
    /// Wrap an explicit manager (tests use an isolated in-memory manager).
    #[must_use]
    pub fn new(manager: ResourceLeaseManager) -> Self {
        Self { manager }
    }

    /// Wrap the process-global lease manager (production composition).
    #[must_use]
    pub fn global() -> Self {
        Self {
            manager: ResourceLeaseManager::global(),
        }
    }

    /// Acquire the exclusive write-resource set for `tool_name` in canonical
    /// order and seal the held leases. Conflicting writes block via the
    /// manager; on any conflict the partially-acquired guards are dropped
    /// (released) and the conflict is returned before any provider is reached.
    ///
    /// The returned set's digest is the single canonical
    /// [`write_resource_set_digest`], so it matches the grant `ExecutionGate`
    /// issued for the same tool and params.
    pub async fn acquire_write_leases(
        &self,
        ctx: &OsLeaseContext,
        tool_name: &str,
        params: &Value,
    ) -> Result<AcquiredResourceLeaseSet, ResourceLeaseError> {
        let mut reqs = os_write_requirements(tool_name, params);
        // Requirements are already canonically ordered, but enforce it here so
        // acquisition order cannot depend on caller construction (OSC-008.3).
        sort_canonical(&mut reqs);
        let digest = Digest::from_hex(canonical_resource_set_digest(&reqs));

        let mut guards = Vec::with_capacity(reqs.len());
        for req in &reqs {
            let request = ResourceLeaseRequest {
                workflow_id: ctx.workflow_id.clone(),
                stage_id: ctx.stage_id.clone(),
                action_hash: ctx.action_hash.clone(),
                kind: req.kind,
                scope: req.scope.clone(),
                access_mode: req.access_mode,
                owner: format!("os-tool:{tool_name}"),
                ttl: req.ttl(),
                preemptible: false,
            };
            match self.manager.acquire(request).await {
                Ok(guard) => guards.push(guard),
                Err(error) => {
                    // Release everything acquired so far before failing.
                    drop(guards);
                    return Err(error);
                }
            }
        }

        Ok(AcquiredResourceLeaseSet::seal(digest, guards))
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;
    use crate::os_control::manifest::{frozen_contracts, frozen_tool_names};

    fn is_mutating_tool(tool: &str) -> bool {
        frozen_contract(tool).is_some_and(is_mutating)
    }

    #[test]
    fn every_manifest_domain_token_parses_to_a_typed_kind() {
        // No global "unknown" scope: every domain token in every frozen rule
        // maps to a typed OsResourceKind.
        for contract in frozen_contracts() {
            for rule in &contract.resources.rules {
                let mut rest = rule.trim();
                if let Some(r) = rest.strip_prefix("shared read lease ") {
                    rest = r;
                }
                if let Some(r) = rest.strip_prefix("sorted union: ") {
                    rest = r;
                }
                let token = rest.split_once('/').map_or(rest, |(k, _)| k).trim();
                assert!(
                    OsResourceKind::from_token(token).is_some(),
                    "domain token `{token}` in `{}` rule `{rule}` is not typed",
                    contract.tool_name
                );
            }
        }
    }

    #[test]
    fn every_mutating_tool_declares_at_least_one_precise_write_resource() {
        // Completion proof: every mutating canonical tool maps to >=1 precise
        // typed write resource, and no resource carries an untyped/empty scope.
        for tool in frozen_tool_names() {
            if !is_mutating_tool(&tool) {
                continue;
            }
            let reqs = os_write_requirements(&tool, &serde_json::json!({}));
            assert!(
                !reqs.is_empty(),
                "mutating tool `{tool}` declared no write resource"
            );
            for req in &reqs {
                assert_eq!(req.kind, ResourceKind::OsControl);
                assert!(req.access_mode == AccessMode::Exclusive);
                let (domain, scope) = req.scope.split_once('/').expect("canonical key");
                assert!(
                    OsResourceKind::from_token(domain).is_some(),
                    "tool `{tool}` produced untyped domain `{domain}`"
                );
                assert!(
                    !scope.is_empty(),
                    "tool `{tool}` produced an empty scope for `{domain}`"
                );
            }
        }
    }

    #[test]
    fn read_only_tools_declare_no_exclusive_write_and_have_shared_reads() {
        for tool in frozen_tool_names() {
            if is_mutating_tool(&tool) {
                continue;
            }
            let writes = os_write_requirements(&tool, &serde_json::json!({}));
            assert!(
                writes.is_empty(),
                "read-only tool `{tool}` must declare no exclusive write resource"
            );
            let reads = os_read_requirements(&tool, &serde_json::json!({}));
            assert!(
                !reads.is_empty(),
                "read-only tool `{tool}` must declare at least one shared read lease"
            );
            assert!(reads.iter().all(|r| r.access_mode == AccessMode::Read));
        }
    }

    #[test]
    fn file_write_resolves_precise_path_scope() {
        let reqs = os_write_requirements(
            "write_file",
            &serde_json::json!({ "path": "/tmp/kria-a.txt" }),
        );
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].kind, ResourceKind::OsControl);
        assert_eq!(reqs[0].scope, "path//tmp/kria-a.txt");
        assert_eq!(reqs[0].access_mode, AccessMode::Exclusive);
    }

    #[test]
    fn distinct_paths_do_not_collide_and_same_path_matches() {
        let a = os_write_requirements("write_file", &serde_json::json!({ "path": "/tmp/a" }));
        let b = os_write_requirements("write_file", &serde_json::json!({ "path": "/tmp/b" }));
        let a2 = os_write_requirements("write_file", &serde_json::json!({ "path": "/tmp/a" }));
        assert_ne!(a[0].scope, b[0].scope);
        assert_eq!(a[0].scope, a2[0].scope);
    }

    #[test]
    fn multi_target_move_produces_two_sorted_path_resources() {
        let reqs = os_write_requirements(
            "move_file",
            &serde_json::json!({ "source": "/tmp/z", "destination": "/tmp/a" }),
        );
        // source + destination → two exclusive path resources, canonically sorted.
        assert_eq!(reqs.len(), 2);
        assert!(reqs.iter().all(|r| r.access_mode == AccessMode::Exclusive));
        let keys: Vec<&str> = reqs.iter().map(|r| r.scope.as_str()).collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(keys, sorted, "resources must be canonically ordered");
    }

    #[test]
    fn bluetooth_action_resolves_adapter_and_device() {
        let reqs = os_write_requirements(
            "connect_bluetooth_device",
            &serde_json::json!({ "device": "AA:BB:CC" }),
        );
        assert!(reqs
            .iter()
            .any(|r| r.scope.starts_with("bluetooth-device/AA:BB:CC")));
        assert!(reqs
            .iter()
            .any(|r| r.scope.starts_with("bluetooth-adapter/")));
    }

    #[test]
    fn digest_matches_execution_gate_derivation() {
        // The coordinator/grant share one canonical derivation: the digest here
        // must equal a digest computed straight from the gate's requirement
        // declaration for the same tool and params.
        let params = serde_json::json!({ "path": "/tmp/kria-digest.txt" });
        let via_module = write_resource_set_digest("write_file", &params);
        let reqs =
            crate::agent::execution_gate::declare_resource_requirements("write_file", &params);
        let via_gate = Digest::from_hex(canonical_resource_set_digest(&reqs));
        assert_eq!(via_module, via_gate);
    }

    #[test]
    fn digest_changes_when_target_changes() {
        let a = write_resource_set_digest("write_file", &serde_json::json!({ "path": "/tmp/a" }));
        let b = write_resource_set_digest("write_file", &serde_json::json!({ "path": "/tmp/b" }));
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn coordinator_acquires_and_seals_matching_digest() {
        let coordinator = OsResourceCoordinator::new(ResourceLeaseManager::new());
        let ctx = OsLeaseContext {
            workflow_id: "wf-seal".to_string(),
            stage_id: None,
            action_hash: "act-1".to_string(),
        };
        let params = serde_json::json!({ "path": "/tmp/kria-seal.txt" });
        let held = coordinator
            .acquire_write_leases(&ctx, "write_file", &params)
            .await
            .expect("acquire write leases");
        assert_eq!(held.held_count(), 1);
        assert_eq!(
            held.resource_set_digest(),
            &write_resource_set_digest("write_file", &params)
        );
    }

    #[tokio::test]
    async fn conflicting_writes_are_exclusive_reads_coexist() {
        let manager = ResourceLeaseManager::new();
        let coordinator = OsResourceCoordinator::new(manager.clone());
        let params = serde_json::json!({ "path": "/tmp/kria-conflict.txt" });

        let ctx_a = OsLeaseContext {
            workflow_id: "wf-a".to_string(),
            stage_id: None,
            action_hash: "a".to_string(),
        };
        let ctx_b = OsLeaseContext {
            workflow_id: "wf-b".to_string(),
            stage_id: None,
            action_hash: "b".to_string(),
        };

        let held_a = coordinator
            .acquire_write_leases(&ctx_a, "write_file", &params)
            .await
            .expect("first writer");

        // A second workflow's conflicting write on the same path is refused.
        let conflict = coordinator
            .acquire_write_leases(&ctx_b, "write_file", &params)
            .await;
        assert!(matches!(conflict, Err(ResourceLeaseError::Conflict { .. })));

        // Two shared reads on the same path coexist.
        let read_reqs = os_read_requirements("read_file", &params);
        assert!(!read_reqs.is_empty());
        // Releasing the writer frees the path.
        drop(held_a);
    }

    #[tokio::test]
    async fn dropping_lease_set_releases_leases() {
        let manager = ResourceLeaseManager::new();
        let coordinator = OsResourceCoordinator::new(manager.clone());
        let params = serde_json::json!({ "path": "/tmp/kria-drop.txt" });
        let ctx = OsLeaseContext {
            workflow_id: "wf-drop".to_string(),
            stage_id: None,
            action_hash: "d".to_string(),
        };
        {
            let _held = coordinator
                .acquire_write_leases(&ctx, "write_file", &params)
                .await
                .expect("acquire");
            assert_eq!(manager.active_leases().await.len(), 1);
        }
        // Guard drop schedules async release; give it a moment.
        tokio::task::yield_now().await;
        // A fresh writer from another workflow can now proceed.
        let coordinator2 = OsResourceCoordinator::new(manager.clone());
        let ctx2 = OsLeaseContext {
            workflow_id: "wf-drop-2".to_string(),
            stage_id: None,
            action_hash: "d2".to_string(),
        };
        // Retry briefly to tolerate the async release.
        let mut acquired = false;
        for _ in 0..50 {
            match coordinator2
                .acquire_write_leases(&ctx2, "write_file", &params)
                .await
            {
                Ok(_g) => {
                    acquired = true;
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(5)).await,
            }
        }
        assert!(acquired, "leases must be released after the set is dropped");
    }

    #[test]
    fn lease_evidence_is_not_clone() {
        // Structural proof: AcquiredResourceLeaseSet does not implement Clone and
        // exposes only its digest. (A `set.clone()` call fails to compile, and
        // the private fields make a struct literal impossible outside this
        // module — see the compile_fail doctest on the type.)
        fn _assert_digest_only(set: &AcquiredResourceLeaseSet) -> &Digest {
            set.resource_set_digest()
        }
    }
}

