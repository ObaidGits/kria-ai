//! Deterministic capability-set hash.
//!
//! This is the ONE reusable piece extracted from the deleted legacy
//! `ApprovalCache` (M12 Option-A migration): the frozen `DockerRuntime` uses it
//! to tag capability-lifecycle telemetry (`CapabilityAction::Requested`) with a
//! stable content hash over the granted capability set. It carries no policy —
//! permission decisions now live entirely in `capability::permission` +
//! `capability::grants` (the one permission engine + one grant store).

use super::capability::Capability;
use sha2::{Digest, Sha256};

/// Deterministic hash over the identity-affecting inputs of a capability grant
/// (canonicalized: the capability set is sorted JSON). Excludes cosmetic fields
/// so an unchanged grant set always hashes identically.
pub fn compute_hash(
    slug: &str,
    version: &str,
    granted: &[Capability],
    budget: &str,
    schema_epoch: &str,
) -> String {
    let mut caps_json: Vec<String> = granted
        .iter()
        .map(|c| serde_json::to_string(c).unwrap_or_default())
        .collect();
    caps_json.sort();
    let payload = format!(
        "{slug}|{version}|{budget}|{schema_epoch}|{}",
        caps_json.join(",")
    );
    let mut h = Sha256::new();
    h.update(payload.as_bytes());
    hex::encode(h.finalize())
}
