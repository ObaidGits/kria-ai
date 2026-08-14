//! Ownership: the `OwnershipControl` desired-state provider (design §3,
//! §9.1, §10.1 `FileControl.set_ownership` + `BrokerOperation::SetBoundPathOwnership`).
//!
//! linux-os-control-production **Task 3.1** (OSC-010.5).
//!
//! `set_file_ownership` is RED and requires privilege (OSC-010.5): unlike
//! every other file mutation in this module (plain `std::fs`), changing a
//! path's owner is dispatched **exclusively** through the existing typed
//! `BrokerOperation::SetBoundPathOwnership` (Task 1.5's Polkit privilege
//! broker) — never a raw `chown`/`chown(2)` call from this process, and never
//! a new broker operation. The transport here observes the current owner via
//! `std::fs::metadata` (read-only, unprivileged) but the *mutation* is a
//! broker round trip whose request carries a [`BrokerBoundPath`] with the
//! path's expected device/inode/owner identity, so the broker itself
//! re-verifies the identity immediately before applying (design §12) — this
//! is what "matching path/resource/grant identity" in the task description
//! refers to.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use async_trait::async_trait;

use crate::os_control::broker::{
    dispatch_via_broker_bound, BrokerBoundPath, BrokerOperation, BrokerTransport, ExistingLocalIdentity,
};
use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    ComparatorKind, DesiredStateControl, Digest, OsEvidenceSource, ProviderId, SafeErrorCode,
    SafeField, SafeText, VerificationReliability,
};
use crate::os_control::error::OsControlError;
use crate::os_control::receipt::{
    ApplyOutcome, RedactedObservation, RollbackToken, SatisfyingVerification, UncertainDispatch,
    UncertainEffectCause, VerificationContradiction, VerificationReport,
};
use crate::os_control::runtime::NormalizedObservation;

use super::canonical_path_identity;

/// The stable provider identity for the broker-backed ownership backend.
pub const OWNERSHIP_PROVIDER_ID: &str = "ownership-broker";

/// A normalized ownership observation (design §5, §10.1): the path's
/// canonical identity bound to its current owner uid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnershipState {
    /// Canonical identity of the path.
    pub path_identity: Digest,
    /// Current owner uid.
    pub owner_uid: u32,
}

impl OwnershipState {
    /// Construct from a path and observed owner uid.
    #[must_use]
    pub fn new(path: &Path, owner_uid: u32) -> Self {
        Self {
            path_identity: canonical_path_identity(path),
            owner_uid,
        }
    }
}

impl NormalizedObservation for OwnershipState {
    fn observation_digest(&self) -> Digest {
        Digest::of_str(&format!(
            "ownership:{}:{}",
            self.path_identity, self.owner_uid
        ))
    }
}

/// A fully-described `set_file_ownership` request.
#[derive(Debug, Clone)]
pub struct OwnershipRequest {
    /// The canonical tool/action name the grant was minted against.
    pub action: String,
    /// The canonical tool parameters (must reproduce the grant's params
    /// digest).
    pub params: serde_json::Value,
    /// The canonical target path.
    pub path: PathBuf,
    /// The existing local identity to assign.
    pub owner: ExistingLocalIdentity,
}

impl OwnershipRequest {
    /// The desired end state.
    #[must_use]
    pub fn desired_state(&self) -> OwnershipState {
        OwnershipState::new(&self.path, self.owner.uid)
    }

    /// The idempotency/verification comparator.
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        ComparatorKind::Exact
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Transport seam
// ─────────────────────────────────────────────────────────────────────────────

/// The raw ownership transport seam. Observation is a plain unprivileged
/// `std::fs::metadata` read; mutation dispatches through the existing
/// [`BrokerTransport`] with a [`BrokerOperation::SetBoundPathOwnership`]
/// request — never a raw `chown` syscall/subprocess from this transport.
#[async_trait]
pub trait OwnershipTransport: Send + Sync {
    /// The stable provider identity (never model prose).
    fn provider_id(&self) -> ProviderId;

    /// Read the current owner uid of `path`.
    async fn read_owner(&self, path: &Path) -> Result<u32, OsControlError>;

    /// Build the identity-bound path descriptor the broker verifies
    /// immediately before applying (device/inode/owner at dispatch time).
    async fn bind_path_identity(&self, path: &Path) -> Result<BrokerBoundPath, OsControlError>;

    /// Dispatch a `SetBoundPathOwnership` broker request.
    async fn dispatch(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        path: &BrokerBoundPath,
        owner: &ExistingLocalIdentity,
    ) -> Result<ApplyOutcome, OsControlError>;
}

/// The `OwnershipControl` desired-state provider (design §3, §4, §10.1).
pub struct OwnershipControl<T: OwnershipTransport> {
    transport: T,
}

impl<T: OwnershipTransport> OwnershipControl<T> {
    /// Compose over a transport.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Borrow the underlying transport (used by tests).
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// The provider identity.
    #[must_use]
    pub fn provider_id(&self) -> ProviderId {
        self.transport.provider_id()
    }

    fn evidence_source(&self) -> OsEvidenceSource {
        OsEvidenceSource::AuthoritativeServiceState
    }

    fn satisfying(&self, observed: &OwnershipState) -> SatisfyingVerification<OwnershipState> {
        SatisfyingVerification::new(
            self.evidence_source(),
            VerificationReliability::Strong,
            self.transport.provider_id(),
            RedactedObservation::new(observed.clone(), observed.observation_digest()),
            None,
            SystemTime::now(),
            0,
        )
    }
}

#[async_trait]
impl<T: OwnershipTransport> DesiredStateControl<OwnershipRequest, OwnershipState>
    for OwnershipControl<T>
{
    async fn observe(
        &self,
        _ctx: &HostExecutionContext,
        request: &OwnershipRequest,
    ) -> Result<OwnershipState, OsControlError> {
        let owner_uid = self.transport.read_owner(&request.path).await?;
        Ok(OwnershipState::new(&request.path, owner_uid))
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &OwnershipRequest,
        _desired: &OwnershipState,
    ) -> Result<ApplyOutcome, OsControlError> {
        let bound_path = self.transport.bind_path_identity(&request.path).await?;
        self.transport
            .dispatch(ctx, &bound_path, &request.owner)
            .await
    }

    async fn verify(
        &self,
        _ctx: &HostExecutionContext,
        request: &OwnershipRequest,
        desired: &OwnershipState,
    ) -> Result<VerificationReport<OwnershipState>, OsControlError> {
        let owner_uid = self.transport.read_owner(&request.path).await?;
        let observed = OwnershipState::new(&request.path, owner_uid);

        if observed.observation_digest() == desired.observation_digest() {
            Ok(VerificationReport::Satisfied(self.satisfying(&observed)))
        } else {
            Ok(VerificationReport::Contradicted(
                VerificationContradiction::new(
                    desired.observation_digest(),
                    Some(observed.observation_digest()),
                    SafeErrorCode::from_static("os_control.incident.contradicted"),
                ),
            ))
        }
    }

    async fn rollback(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        _token: &RollbackToken,
    ) -> Result<ApplyOutcome, OsControlError> {
        // The frozen manifest declares `rollbackClaim: None` for
        // `set_file_ownership`: never actually invoked. Reports the truthful
        // "no inverse" fact if it ever were.
        Ok(ApplyOutcome::Uncertain(UncertainDispatch::new(
            None,
            UncertainEffectCause::Unobservable,
            crate::os_control::contract::BoundedVec::new(),
        )))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Receipt → tool-result mapping
// ─────────────────────────────────────────────────────────────────────────────

use crate::os_control::receipt::{ActionLifecycle, MutationReceipt};

/// Map a governed [`MutationReceipt`] to the `set_file_ownership` result
/// fields.
#[must_use]
pub fn set_file_ownership_result(
    receipt: &MutationReceipt<OwnershipState>,
    path: &str,
    owner_uid: u32,
) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "path": path,
        "owner_uid": owner_uid,
        "set": matches!(lifecycle, ActionLifecycle::Verified | ActionLifecycle::Unchanged),
        "already_in_desired_state": matches!(lifecycle, ActionLifecycle::Unchanged),
        "lifecycle": lifecycle.as_str(),
        "verified": receipt.verification().is_some(),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// `HostOsControl::ownership()` port seam (design §4)
// ─────────────────────────────────────────────────────────────────────────────

/// The dyn-compatible ownership domain port.
pub trait OwnershipControlPort: DesiredStateControl<OwnershipRequest, OwnershipState> {}

impl<T: OwnershipTransport> OwnershipControlPort for OwnershipControl<T> {}

// ─────────────────────────────────────────────────────────────────────────────
// Real metadata-read + broker-dispatch transport
// ─────────────────────────────────────────────────────────────────────────────

/// A fully-functional transport: unprivileged `std::fs::metadata` reads for
/// observation, and a [`BrokerTransport`]-backed dispatch for mutation. No
/// live-transport gating of its own — it delegates the actual privileged
/// syscall to the broker, whose own transport seam
/// ([`crate::os_control::broker::LiveBrokerTransport`]) is already
/// deny-live-gated.
pub struct RealOwnershipTransport<B: BrokerTransport + Send + Sync> {
    broker: B,
}

impl<B: BrokerTransport + Send + Sync> RealOwnershipTransport<B> {
    /// Compose over a broker transport.
    #[must_use]
    pub fn new(broker: B) -> Self {
        Self { broker }
    }
}

#[cfg(unix)]
fn read_identity(path: &Path) -> Result<(u64, u64, u32), OsControlError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path).map_err(|e| OsControlError::InvalidRequest {
        field: SafeField::new("path"),
        reason: SafeText::new(format!("reading path metadata failed: {e}")),
    })?;
    Ok((metadata.dev(), metadata.ino(), metadata.uid()))
}

#[cfg(not(unix))]
fn read_identity(_path: &Path) -> Result<(u64, u64, u32), OsControlError> {
    Err(OsControlError::Unsupported {
        capability: crate::os_control::contract::CapabilityId::new("set_file_ownership"),
        reason: SafeText::new("ownership changes are only supported on Unix hosts"),
    })
}

#[async_trait]
impl<B: BrokerTransport + Send + Sync> OwnershipTransport for RealOwnershipTransport<B> {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(OWNERSHIP_PROVIDER_ID)
    }

    async fn read_owner(&self, path: &Path) -> Result<u32, OsControlError> {
        let (_, _, uid) = read_identity(path)?;
        Ok(uid)
    }

    async fn bind_path_identity(&self, path: &Path) -> Result<BrokerBoundPath, OsControlError> {
        let (device, inode, owner_uid) = read_identity(path)?;
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        Ok(BrokerBoundPath {
            path: canonical.to_string_lossy().to_string(),
            device,
            inode,
            owner_uid,
        })
    }

    async fn dispatch(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        path: &BrokerBoundPath,
        owner: &ExistingLocalIdentity,
    ) -> Result<ApplyOutcome, OsControlError> {
        let operation = BrokerOperation::SetBoundPathOwnership {
            path: path.clone(),
            owner: owner.clone(),
        };
        // Bound to a local so the SAME nonce reaches both the request's caller
        // binding and the broker connection.
        let caller = caller_credentials();
        let request = crate::os_control::broker::build_broker_request(
            ctx,
            &caller,
            format!("set-ownership-{}", path.path),
            operation,
        )?;
        // The same nonce the caller binding was derived from must reach the
        // broker, or it derives a different binding and refuses the request.
        dispatch_via_broker_bound(&self.broker, &request, caller.connection_nonce.as_str())
    }
}

/// The broker caller's local peer credentials. In v1 this is always the
/// current process's own uid/gid/pid — the broker is a local Polkit-fronted
/// service the same user session talks to; there is no remote caller.
fn caller_credentials() -> crate::os_control::broker::PeerCredentials {
    crate::os_control::broker::PeerCredentials {
        // SAFETY-equivalent: `getuid`/`getgid`/`getpid` are always-succeeding
        // libc calls with no error condition.
        uid: unsafe { libc::getuid() },
        gid: unsafe { libc::getgid() },
        pid: unsafe { libc::getpid() },
        connection_nonce: format!("ownership-{}", uuid::Uuid::new_v4()),
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn digest_binds_path_identity_and_owner() {
        let a = OwnershipState::new(Path::new("/a"), 1000);
        let b = OwnershipState::new(Path::new("/a"), 1000);
        assert_eq!(a.observation_digest(), b.observation_digest());
        let c = OwnershipState::new(Path::new("/a"), 1001);
        assert_ne!(a.observation_digest(), c.observation_digest());
        let d = OwnershipState::new(Path::new("/b"), 1000);
        assert_ne!(a.observation_digest(), d.observation_digest());
    }

    #[test]
    fn desired_state_matches_requested_owner() {
        let req = OwnershipRequest {
            action: "set_file_ownership".into(),
            params: serde_json::json!({}),
            path: PathBuf::from("/tmp/x"),
            owner: ExistingLocalIdentity {
                uid: 1000,
                name: SafeText::new("alice"),
            },
        };
        assert_eq!(req.desired_state().owner_uid, 1000);
    }

    #[cfg(unix)]
    #[test]
    fn read_identity_reads_real_metadata() {
        let dir = crate::os_control::testing::temp_dir();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, b"x").unwrap();
        let (device, inode, _uid) = read_identity(&file).unwrap();
        assert!(device > 0 || inode > 0);
    }
}
