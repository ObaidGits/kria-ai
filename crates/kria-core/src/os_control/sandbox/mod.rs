//! `os_control::sandbox` — scoped, expiring per-domain-operation skill grants.
//!
//! linux-os-control-production **Task 1.10** (OSC-026, OSC-029), design §2.1,
//! §9.11 and Correctness Property 26.
//!
//! # The only bridge from a skill to a host effect
//!
//! Extension (OpenClaw skill) grants **cannot** authorize OS mutation (design
//! §2.1, Task 0.3). When a skill needs a host effect it must re-enter a
//! canonical OS tool under a **scoped skill grant** — the [`SandboxGrant`] this
//! module defines. A grant is bound to exactly one domain operation
//! ([`CapabilityId`]), an explicit resource [`SandboxScope`] (with network and
//! filesystem limits), a source [`SkillIdentity`], a purpose, a risk ceiling,
//! and an expiry, and it is issued only after policy evaluation + approval
//! (OSC-026.1/.3).
//!
//! # Least authority, deny-by-default (Property 26, OSC-026.2/.7)
//!
//! A [`SandboxGrant`] exposes only typed identifiers — it never carries a
//! `HostOsControl` handle, a privilege-broker handle, a bus connection, a device
//! node, or a shell. The only thing it can do is *authorize one operation within
//! its exact scope*; every other request is denied. Grant construction is gated
//! by a [`SandboxGrantAuthority`] witness whose field is private to this module,
//! so a skill/tool cannot forge a grant, and [`SandboxGrantControl::request_grant`]
//! denies any operation that is not a known canonical OS capability (so a request
//! for raw `HostOsControl`/broker access, being unknown, fails closed).
//!
//! # Revalidation and revocation (OSC-026.4/.5)
//!
//! Every invocation re-validates operation, scope, skill identity, purpose,
//! expiry, and current risk against the grant. Revocation is tracked by the
//! control and takes effect **before** the next provider call.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::os_control::contract::{BoundedVec, CapabilityId, Digest, GrantDecision, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::manifest::frozen_tool_names;
use crate::os_control::resource::OsResourceKind;
use crate::safety::RiskLevel;

/// Maximum length (chars) of a skill identity / grant id token.
pub const SANDBOX_TOKEN_MAX_CHARS: usize = 128;
/// Maximum entries in a network/filesystem allow-list.
pub const SANDBOX_LIMIT_CAP: usize = 32;
/// Maximum grant lifetime (seconds); no skill grant is unbounded (OSC-026.1).
pub const SANDBOX_GRANT_MAX_TTL_SECS: u64 = 60 * 60;

fn sanitize_token(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(SANDBOX_TOKEN_MAX_CHARS));
    for ch in raw.chars() {
        if out.chars().count() >= SANDBOX_TOKEN_MAX_CHARS {
            break;
        }
        if !ch.is_control() {
            out.push(ch);
        }
    }
    out.trim().to_string()
}

// ─────────────────────────────────────────────────────────────────────────────
// Identities
// ─────────────────────────────────────────────────────────────────────────────

macro_rules! sandbox_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            /// Construct a bounded, control-char-free token.
            #[must_use]
            pub fn new(raw: impl Into<String>) -> Self {
                Self(sanitize_token(&raw.into()))
            }

            /// Borrow the token.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.0)
            }
        }
    };
}

sandbox_id!(
    /// The source skill (OpenClaw extension) identity a grant is bound to.
    SkillIdentity
);
sandbox_id!(
    /// A sandbox grant's opaque identity.
    SandboxGrantId
);

// ─────────────────────────────────────────────────────────────────────────────
// Scope + network/filesystem limits (OSC-026.1)
// ─────────────────────────────────────────────────────────────────────────────

/// The network limit attached to a grant. Deny-by-default: [`NetworkLimit::None`]
/// is the default and allows no network access.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "hosts")]
pub enum NetworkLimit {
    /// No network access.
    #[default]
    None,
    /// Loopback only.
    Loopback,
    /// A bounded allow-list of host labels.
    AllowList(BoundedVec<SafeText>),
}

/// The filesystem limit attached to a grant. Deny-by-default:
/// [`FilesystemLimit::None`] is the default and allows no path access.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "paths")]
pub enum FilesystemLimit {
    /// No filesystem access.
    #[default]
    None,
    /// Read-only over a bounded set of path labels.
    ReadOnly(BoundedVec<SafeText>),
    /// Read-write over a bounded set of path labels.
    ReadWrite(BoundedVec<SafeText>),
}

/// The explicit resource scope a grant is bound to (OSC-026.1). Carries a typed
/// resource domain, an opaque bounded scope token, and the network/filesystem
/// limits. Scope equality is exact so a grant cannot be used outside its scope.
/// Serialize an [`OsResourceKind`] as its stable domain token (the enum itself
/// is not `Serialize`).
fn serialize_resource_kind<S: serde::Serializer>(
    kind: &OsResourceKind,
    s: S,
) -> Result<S::Ok, S::Error> {
    s.serialize_str(kind.token())
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SandboxScope {
    /// The typed OS resource domain (e.g. `network-profile`, `path-subtree`).
    #[serde(serialize_with = "serialize_resource_kind")]
    pub resource_kind: OsResourceKind,
    /// The opaque bounded scope identity.
    pub scope: SafeText,
    /// Network access limit (deny-by-default).
    #[serde(default)]
    pub network: NetworkLimit,
    /// Filesystem access limit (deny-by-default).
    #[serde(default)]
    pub filesystem: FilesystemLimit,
}

impl SandboxScope {
    /// Construct a scope with deny-by-default network/filesystem limits.
    #[must_use]
    pub fn new(resource_kind: OsResourceKind, scope: impl Into<String>) -> Self {
        Self {
            resource_kind,
            scope: SafeText::new(scope.into()),
            network: NetworkLimit::None,
            filesystem: FilesystemLimit::None,
        }
    }

    /// Builder: set the network limit.
    #[must_use]
    pub fn with_network(mut self, network: NetworkLimit) -> Self {
        self.network = network;
        self
    }

    /// Builder: set the filesystem limit.
    #[must_use]
    pub fn with_filesystem(mut self, filesystem: FilesystemLimit) -> Self {
        self.filesystem = filesystem;
        self
    }

    /// A correlation-safe digest of the resource kind + scope identity.
    #[must_use]
    pub fn digest(&self) -> Digest {
        Digest::of_str(&format!(
            "{}/{}",
            self.resource_kind.token(),
            self.scope.as_str()
        ))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The unforgeable grant + its authority witness (Property 26)
// ─────────────────────────────────────────────────────────────────────────────

/// Runtime-only witness proving a grant is being minted by the sandbox granting
/// authority (policy evaluation + approval). Its field is private to this
/// module, so no skill/tool/adapter can construct one and thus cannot forge a
/// [`SandboxGrant`].
///
/// ```compile_fail
/// use kria_core::os_control::sandbox::SandboxGrantAuthority;
/// let _forged = SandboxGrantAuthority(()); // error: constructor is private
/// ```
pub struct SandboxGrantAuthority(());

impl SandboxGrantAuthority {
}

#[cfg(feature = "os-control-test")]
impl SandboxGrantAuthority {
    /// Mint a witness for deny-live tests. Gated to `os-control-test`.
    #[must_use]
    pub fn for_test() -> Self {
        Self(())
    }
}

/// A scoped, expiring per-domain-operation skill grant (OSC-026). Private fields
/// and no public struct-literal constructor; the only producer is
/// [`SandboxGrant::mint`], which requires a [`SandboxGrantAuthority`]. It exposes
/// only typed identifiers — never a host/broker/bus handle (Property 26).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SandboxGrant {
    grant_id: SandboxGrantId,
    skill: SkillIdentity,
    operation: CapabilityId,
    scope: SandboxScope,
    purpose: SafeText,
    max_risk: RiskLevel,
    decision: GrantDecision,
    issued_unix: u64,
    expires_unix: u64,
}

impl SandboxGrant {
    /// Mint a grant. Requires a [`SandboxGrantAuthority`], so only the granting
    /// authority can construct one.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn mint(
        _authority: &SandboxGrantAuthority,
        grant_id: SandboxGrantId,
        skill: SkillIdentity,
        operation: CapabilityId,
        scope: SandboxScope,
        purpose: SafeText,
        max_risk: RiskLevel,
        decision: GrantDecision,
        issued_unix: u64,
        expires_unix: u64,
    ) -> Self {
        Self {
            grant_id,
            skill,
            operation,
            scope,
            purpose,
            max_risk,
            decision,
            issued_unix,
            expires_unix,
        }
    }

    /// The grant's opaque identity.
    #[must_use]
    pub fn grant_id(&self) -> &SandboxGrantId {
        &self.grant_id
    }

    /// The source skill this grant is bound to.
    #[must_use]
    pub fn skill(&self) -> &SkillIdentity {
        &self.skill
    }

    /// The single canonical OS operation this grant authorizes.
    #[must_use]
    pub fn operation(&self) -> &CapabilityId {
        &self.operation
    }

    /// The bound resource scope.
    #[must_use]
    pub fn scope(&self) -> &SandboxScope {
        &self.scope
    }

    /// The risk ceiling this grant was approved at.
    #[must_use]
    pub fn max_risk(&self) -> RiskLevel {
        self.max_risk
    }

    /// Whether the grant has expired at `now_unix`.
    #[must_use]
    pub fn is_expired(&self, now_unix: u64) -> bool {
        now_unix >= self.expires_unix
    }

    /// Re-validate an invocation against this grant (OSC-026.4). Deny-by-default:
    /// any mismatch of operation, scope, skill, purpose, an expiry, or a risk
    /// increase fails closed. This does **not** consult revocation — the control
    /// checks revocation first (OSC-026.5).
    pub fn authorizes(
        &self,
        request: &SkillOperationRequest,
        now_unix: u64,
    ) -> Result<(), SandboxDenyReason> {
        if self.is_expired(now_unix) {
            return Err(SandboxDenyReason::Expired);
        }
        if self.skill != request.skill {
            return Err(SandboxDenyReason::SkillMismatch);
        }
        if self.operation != request.operation {
            return Err(SandboxDenyReason::OperationMismatch);
        }
        if self.scope != request.scope {
            return Err(SandboxDenyReason::ScopeMismatch);
        }
        if self.purpose != request.purpose {
            return Err(SandboxDenyReason::PurposeMismatch);
        }
        if request.risk > self.max_risk {
            return Err(SandboxDenyReason::RiskIncreased);
        }
        Ok(())
    }
}

/// One re-validated skill invocation request (OSC-026.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillOperationRequest {
    /// The invoking skill identity (must match the grant).
    pub skill: SkillIdentity,
    /// The canonical operation being invoked (must match the grant).
    pub operation: CapabilityId,
    /// The resource scope of this invocation (must match the grant exactly).
    pub scope: SandboxScope,
    /// The invocation purpose (must match the grant).
    pub purpose: SafeText,
    /// The current risk of the invocation (must not exceed the grant ceiling).
    pub risk: RiskLevel,
}

/// A request to create a new sandbox grant (OSC-026.1/.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantRequest {
    /// The requesting skill identity.
    pub skill: SkillIdentity,
    /// The canonical OS operation requested (must be a known capability).
    pub operation: CapabilityId,
    /// The explicit resource scope + network/filesystem limits.
    pub scope: SandboxScope,
    /// The bounded purpose.
    pub purpose: SafeText,
    /// The risk ceiling the grant is approved at.
    pub max_risk: RiskLevel,
    /// Requested lifetime in seconds (clamped to [`SANDBOX_GRANT_MAX_TTL_SECS`]).
    pub ttl_secs: u64,
    /// The policy/approval decision. Must be [`GrantDecision::Approved`].
    pub decision: GrantDecision,
}

/// The closed set of reasons a sandbox grant denies an operation (fail closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxDenyReason {
    /// The requested operation is not a known canonical OS capability
    /// (deny-by-default — includes raw HostOsControl / broker requests).
    UnknownCapability,
    /// The invoked operation differs from the granted operation.
    OperationMismatch,
    /// The invocation scope differs from the granted scope.
    ScopeMismatch,
    /// The invoking skill differs from the grant's bound skill.
    SkillMismatch,
    /// The invocation purpose differs from the granted purpose.
    PurposeMismatch,
    /// The grant has expired.
    Expired,
    /// The grant was revoked.
    Revoked,
    /// The current risk exceeds the grant's approved ceiling.
    RiskIncreased,
    /// Grant creation/escalation was not approved.
    ApprovalRequired,
}

impl SandboxDenyReason {
    /// Stable snake_case token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnknownCapability => "unknown_capability",
            Self::OperationMismatch => "operation_mismatch",
            Self::ScopeMismatch => "scope_mismatch",
            Self::SkillMismatch => "skill_mismatch",
            Self::PurposeMismatch => "purpose_mismatch",
            Self::Expired => "expired",
            Self::Revoked => "revoked",
            Self::RiskIncreased => "risk_increased",
            Self::ApprovalRequired => "approval_required",
        }
    }

    /// Map a deny reason to the fail-closed pre-mutation error.
    #[must_use]
    pub fn to_error(self) -> OsControlError {
        match self {
            Self::UnknownCapability => OsControlError::Unsupported {
                capability: CapabilityId::new("sandbox_grant"),
                reason: SafeText::new(
                    "skill requested an operation outside the canonical OS capability set",
                ),
            },
            Self::ApprovalRequired => OsControlError::PolicyDenied {
                reason: SafeText::new("sandbox grant creation requires approval"),
            },
            Self::Expired => OsControlError::ApprovalExpired,
            other => OsControlError::PolicyDenied {
                reason: SafeText::new(format!("sandbox grant denied: {}", other.as_str())),
            },
        }
    }
}

/// Whether `operation` is a known canonical OS capability from the frozen
/// manifest. Deny-by-default: an unknown capability (including any attempt to
/// name raw `HostOsControl` or the privilege broker) is never grantable.
#[must_use]
pub fn is_known_capability(operation: &CapabilityId) -> bool {
    frozen_tool_names().iter().any(|t| t == operation.as_str())
}

/// The current unix-seconds timestamp.
#[must_use]
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ─────────────────────────────────────────────────────────────────────────────
// The SandboxGrantControl contract (design §9.11)
// ─────────────────────────────────────────────────────────────────────────────

/// The frozen skill-grant port (design §9.11). It issues scoped expiring grants
/// after policy/approval, re-validates every invocation, and revokes grants so
/// revocation takes effect before subsequent provider calls (OSC-026).
#[async_trait::async_trait]
pub trait SandboxGrantControl: Send + Sync {
    /// Create a grant after policy evaluation + approval. Deny-by-default: an
    /// unknown capability or an unapproved decision fails closed.
    async fn request_grant(&self, request: &GrantRequest) -> Result<SandboxGrant, OsControlError>;

    /// Re-validate an invocation against a grant, consulting revocation first
    /// (OSC-026.4/.5). Returns `Ok(())` only when the grant currently authorizes
    /// the exact operation/scope/skill/purpose/risk.
    fn revalidate(
        &self,
        grant: &SandboxGrant,
        request: &SkillOperationRequest,
        now_unix: u64,
    ) -> Result<(), SandboxDenyReason>;

    /// Revoke a grant. Effective before the next [`Self::revalidate`].
    async fn revoke(&self, grant_id: &SandboxGrantId) -> Result<(), OsControlError>;

    /// Whether a grant id is currently revoked.
    fn is_revoked(&self, grant_id: &SandboxGrantId) -> bool;
}

#[cfg(feature = "os-control-test")]
mod fake;
#[cfg(feature = "os-control-test")]
pub use fake::FakeSandboxGrantControl;

