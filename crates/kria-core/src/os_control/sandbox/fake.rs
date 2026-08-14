//! Deny-live fake [`SandboxGrantControl`] (OSC-026, OSC-029), Task 1.10.
//!
//! Compiled only under `os-control-test`. It issues, re-validates and revokes
//! scoped skill grants entirely in memory — never a live bus, broker, device
//! node or shell. Grant minting goes through [`SandboxGrantAuthority::for_test`],
//! so the fake exercises the same unforgeable path the live authority uses.

use std::collections::HashSet;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::os_control::error::OsControlError;

use super::{
    is_known_capability, GrantRequest, SandboxDenyReason, SandboxGrant, SandboxGrantAuthority,
    SandboxGrantControl, SandboxGrantId, SkillOperationRequest, GrantDecision,
    SANDBOX_GRANT_MAX_TTL_SECS,
};

/// A scripted, in-memory sandbox grant control.
///
/// Time is injected rather than read from the clock so expiry is deterministic:
/// [`Self::with_now`] fixes "now" for minting, and `revalidate` takes the caller's
/// `now_unix` exactly as the trait specifies.
pub struct FakeSandboxGrantControl {
    now_unix: u64,
    next_id: Mutex<u64>,
    revoked: Mutex<HashSet<SandboxGrantId>>,
    issued: Mutex<Vec<SandboxGrantId>>,
    revalidations: Mutex<Vec<(SandboxGrantId, Result<(), SandboxDenyReason>)>>,
}

impl FakeSandboxGrantControl {
    /// Create a fresh fake with no issued grants and `now = 0`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            now_unix: 0,
            next_id: Mutex::new(1),
            revoked: Mutex::new(HashSet::new()),
            issued: Mutex::new(Vec::new()),
            revalidations: Mutex::new(Vec::new()),
        }
    }

    /// Builder: fix the mint-time clock so expiry assertions are deterministic.
    #[must_use]
    pub fn with_now(mut self, now_unix: u64) -> Self {
        self.now_unix = now_unix;
        self
    }

    /// Builder: pre-revoke a grant id, so the first `revalidate` denies.
    #[must_use]
    pub fn with_revoked(self, grant_id: SandboxGrantId) -> Self {
        self.revoked.lock().unwrap().insert(grant_id);
        self
    }

    /// The grant ids issued by this fake, in order.
    #[must_use]
    pub fn issued_grants(&self) -> Vec<SandboxGrantId> {
        self.issued.lock().unwrap().clone()
    }

    /// Every recorded revalidation outcome, in order.
    #[must_use]
    pub fn revalidations(&self) -> Vec<(SandboxGrantId, Result<(), SandboxDenyReason>)> {
        self.revalidations.lock().unwrap().clone()
    }

    /// The mint-time clock this fake was configured with.
    #[must_use]
    pub fn now_unix(&self) -> u64 {
        self.now_unix
    }

    fn mint_id(&self) -> SandboxGrantId {
        let mut n = self.next_id.lock().unwrap();
        let id = SandboxGrantId::new(format!("fake-grant-{n}"));
        *n += 1;
        id
    }
}

impl Default for FakeSandboxGrantControl {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SandboxGrantControl for FakeSandboxGrantControl {
    async fn request_grant(&self, request: &GrantRequest) -> Result<SandboxGrant, OsControlError> {
        // Deny-by-default: an operation outside the frozen canonical capability
        // set is never grantable (raw HostOsControl / broker requests land here).
        if !is_known_capability(&request.operation) {
            return Err(SandboxDenyReason::UnknownCapability.to_error());
        }
        // Grant creation requires an approved decision.
        if request.decision != GrantDecision::Approved {
            return Err(SandboxDenyReason::ApprovalRequired.to_error());
        }
        let ttl = request.ttl_secs.min(SANDBOX_GRANT_MAX_TTL_SECS);
        let grant_id = self.mint_id();
        self.issued.lock().unwrap().push(grant_id.clone());
        Ok(SandboxGrant::mint(
            &SandboxGrantAuthority::for_test(),
            grant_id,
            request.skill.clone(),
            request.operation.clone(),
            request.scope.clone(),
            request.purpose.clone(),
            request.max_risk,
            request.decision,
            self.now_unix,
            self.now_unix.saturating_add(ttl),
        ))
    }

    fn revalidate(
        &self,
        grant: &SandboxGrant,
        request: &SkillOperationRequest,
        now_unix: u64,
    ) -> Result<(), SandboxDenyReason> {
        // Revocation is consulted FIRST (OSC-026.5) so it takes effect before
        // any subsequent provider call.
        let outcome = if self.is_revoked(grant.grant_id()) {
            Err(SandboxDenyReason::Revoked)
        } else {
            grant.authorizes(request, now_unix)
        };
        self.revalidations
            .lock()
            .unwrap()
            .push((grant.grant_id().clone(), outcome));
        outcome
    }

    async fn revoke(&self, grant_id: &SandboxGrantId) -> Result<(), OsControlError> {
        self.revoked.lock().unwrap().insert(grant_id.clone());
        Ok(())
    }

    fn is_revoked(&self, grant_id: &SandboxGrantId) -> bool {
        self.revoked.lock().unwrap().contains(grant_id)
    }
}
