//! Security boundaries (HRA Task 38 / R23.1, R23.2).
//!
//! Two pure gates the runtime must pass before destructive/egress actions:
//! - `KillAuthorizer`: process termination requires a valid capability token AND the target PID
//!   must be in the RA-spawned registry. Foreign PIDs are never killable (kill-scope).
//! - `egress_allowed`: privacy-strict data must never leave the device (no cloud egress).

use std::collections::HashSet;

use super::types::PrivacyReq;

/// Opaque capability token required to authorize a reclaim/kill. Minted by the RA; held by the
/// Reconciler. A consumer cannot fabricate one (it is not constructible from outside via a public
/// value — only via `issue`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CapabilityToken(u64);

pub struct KillAuthorizer {
    valid: CapabilityToken,
    ra_spawned: HashSet<u32>,
}

impl KillAuthorizer {
    /// Issue an authorizer with a fresh token and the current RA-spawned PID set.
    pub fn new(token_seed: u64, ra_spawned: HashSet<u32>) -> Self {
        Self {
            valid: CapabilityToken(token_seed),
            ra_spawned,
        }
    }

    pub fn token(&self) -> CapabilityToken {
        self.valid.clone()
    }

    pub fn register_spawned(&mut self, pid: u32) {
        self.ra_spawned.insert(pid);
    }

    pub fn forget_spawned(&mut self, pid: u32) {
        self.ra_spawned.remove(&pid);
    }

    /// Authorize killing `pid`. Requires the correct token AND `pid` in the RA-spawned set.
    pub fn authorize_kill(&self, token: &CapabilityToken, pid: u32) -> bool {
        *token == self.valid && self.ra_spawned.contains(&pid)
    }
}

/// Whether data with `privacy` may egress to a cloud device. Privacy-strict never egresses.
pub fn egress_allowed(privacy: PrivacyReq, target_is_cloud: bool) -> bool {
    if target_is_cloud {
        privacy != PrivacyReq::Strict
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(pids: &[u32]) -> HashSet<u32> {
        pids.iter().copied().collect()
    }

    #[test]
    fn kill_requires_token_and_ra_spawned_pid() {
        let auth = KillAuthorizer::new(0xABCD, set(&[100, 200]));
        let tok = auth.token();
        assert!(auth.authorize_kill(&tok, 100));
        assert!(!auth.authorize_kill(&tok, 999)); // not RA-spawned
    }

    #[test]
    fn wrong_token_never_authorizes() {
        let auth = KillAuthorizer::new(0xABCD, set(&[100]));
        let forged = CapabilityToken(0xDEAD);
        assert!(!auth.authorize_kill(&forged, 100));
    }

    #[test]
    fn forgetting_pid_revokes_kill() {
        let mut auth = KillAuthorizer::new(1, set(&[100]));
        let tok = auth.token();
        assert!(auth.authorize_kill(&tok, 100));
        auth.forget_spawned(100);
        assert!(!auth.authorize_kill(&tok, 100));
    }

    #[test]
    fn privacy_strict_never_egresses() {
        assert!(!egress_allowed(PrivacyReq::Strict, true));
        assert!(egress_allowed(PrivacyReq::Strict, false)); // local ok
        assert!(egress_allowed(PrivacyReq::Standard, true)); // standard may use cloud
    }
}
