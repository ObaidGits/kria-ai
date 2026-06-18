//! Mobile prompt-control foundation (Phase 4.5).
//!
//! Provides per-device pairing and short-lived signed device tokens so a phone
//! (over the private WireGuard mesh) can authenticate to `kria-server`'s agent
//! WebSocket. Tokens are HMAC-SHA256 signed with a key held in the encrypted
//! [`crate::auth::SecretsVault`] (Phase 0.1) — never stored in plaintext — and
//! every device is individually revocable for instant access withdrawal.
//!
//! Security model (see `planning_docs/phase4_5_mobile_plan.md`):
//!   * one signing key per install, persisted in the vault;
//!   * pairing codes are short-lived (default 5 min) and single-use;
//!   * device tokens carry an explicit expiry and are checked against a
//!     revocation list on every verification;
//!   * the registry never logs token or signing-key values.

pub mod pairing;

pub use pairing::{
    DeviceInfo, DeviceRegistry, MobileError, PairingChallenge, Result as MobileResult,
};
