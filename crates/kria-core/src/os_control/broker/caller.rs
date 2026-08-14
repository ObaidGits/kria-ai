//! Authenticated local caller identity and channel binding derivation.
//!
//! linux-os-control-production **Task 1.5**, design §12 (OSC-001).
//!
//! `CallerChannelBindingDigest` is derived from the authenticated local
//! connection's **peer credentials** (`SO_PEERCRED` uid/gid/pid plus a
//! per-connection nonce), never from a self-asserted username or PID in the
//! request body. Both the KRIA client transport and the broker derive the value
//! independently from the OS-supplied credentials; the broker rejects any
//! request whose embedded caller binding does not match the value it derives
//! from the live connection, before Polkit or dispatch.

use crate::os_control::contract::Digest;

use super::protocol::CallerChannelBindingDigest;

/// OS-supplied authenticated peer credentials for a local connection. These come
/// from the kernel (`SO_PEERCRED` on a Unix domain socket), not from the request
/// body, so a caller cannot spoof them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerCredentials {
    /// Peer user id.
    pub uid: u32,
    /// Peer group id.
    pub gid: u32,
    /// Peer process id.
    pub pid: i32,
    /// A per-connection nonce established by the transport handshake, so two
    /// connections from the same uid/pid derive distinct bindings and a captured
    /// request cannot be replayed on a fresh connection.
    pub connection_nonce: String,
}

impl PeerCredentials {
    /// Derive the caller channel binding digest from these credentials. The
    /// derivation is deterministic and domain-separated, so the broker and
    /// client compute the identical value for the same connection.
    #[must_use]
    pub fn derive_binding(&self) -> CallerChannelBindingDigest {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"kria.broker.caller-binding.v1\x1f");
        buf.extend_from_slice(&self.uid.to_be_bytes());
        buf.push(0x1f);
        buf.extend_from_slice(&self.gid.to_be_bytes());
        buf.push(0x1f);
        buf.extend_from_slice(&self.pid.to_be_bytes());
        buf.push(0x1f);
        buf.extend_from_slice(self.connection_nonce.as_bytes());
        CallerChannelBindingDigest::from_digest(Digest::of_bytes(&buf))
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    fn creds(uid: u32, pid: i32, nonce: &str) -> PeerCredentials {
        PeerCredentials {
            uid,
            gid: uid,
            pid,
            connection_nonce: nonce.to_string(),
        }
    }

    #[test]
    fn binding_is_deterministic_for_the_same_connection() {
        let a = creds(1000, 4242, "conn-nonce-a").derive_binding();
        let b = creds(1000, 4242, "conn-nonce-a").derive_binding();
        assert_eq!(a, b);
    }

    #[test]
    fn different_credentials_or_connection_produce_different_bindings() {
        let base = creds(1000, 4242, "conn-a").derive_binding();
        assert_ne!(base, creds(1001, 4242, "conn-a").derive_binding());
        assert_ne!(base, creds(1000, 9999, "conn-a").derive_binding());
        assert_ne!(base, creds(1000, 4242, "conn-b").derive_binding());
    }
}
