//! The broker-side request handler.
//!
//! linux-os-control-production **Task 1.5**, design §12
//! (OSC-001, OSC-002, OSC-004, OSC-007, OSC-030).
//!
//! [`LocalBroker`] is the small privileged service's decision core. It accepts
//! one length-prefixed canonical-CBOR request per authenticated local
//! connection and, in order:
//!
//! 1. decodes the frame (structural rejects have no bindable response);
//! 2. authenticates the local caller and verifies the caller channel binding it
//!    independently derives from the connection's peer credentials;
//! 3. enforces expiry;
//! 4. enforces persistent nonce replay semantics (a replay never dispatches);
//! 5. validates operation-specific freshness (stale plan / target identity);
//! 6. rejects an unsupported adapter;
//! 7. authorizes through Polkit;
//! 8. performs exactly one fixed native operation and returns a bound,
//!    effect-aware response.
//!
//! Every response byte-for-byte echoes the request's authority binding. A
//! failure at steps 2–7 is a bound `NotDispatched` (proving no effect). Once
//! step 8 may have run, only a `Dispatched` response is produced.

use std::sync::Arc;
use std::time::SystemTime;

use super::caller::PeerCredentials;
use super::native::{NativeBrokerOperations, PolkitAuthorizer, PolkitDecision};
use super::packaging::polkit_action_id;
use super::protocol::{
    BrokerPreDispatchError, BrokerRequestV1, BrokerResponseBinding, BrokerResponseV1,
    RequestDecodeError, StructuralReason,
};
use super::replay::{NonceReplayStore, ReplayCheck};

/// The authenticated-connection context the transport hands to the broker. A
/// `None` peer means the connection was not authenticated as a local caller.
#[derive(Debug, Clone)]
pub struct CallerContext {
    /// The OS-supplied peer credentials, or `None` if unauthenticated.
    pub peer: Option<PeerCredentials>,
}

impl CallerContext {
    /// An authenticated caller with the given peer credentials.
    #[must_use]
    pub fn authenticated(peer: PeerCredentials) -> Self {
        Self { peer: Some(peer) }
    }

    /// An unauthenticated connection.
    #[must_use]
    pub fn unauthenticated() -> Self {
        Self { peer: None }
    }
}

/// A structural rejection: the frame could not be bound to a response, so the
/// broker drops it at the transport level (the client maps this to a
/// pre-dispatch `OsControlError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuralReject(pub StructuralReason);

/// The typed privilege broker's request handler.
pub struct LocalBroker {
    replay: Arc<dyn NonceReplayStore>,
    polkit: Arc<dyn PolkitAuthorizer>,
    native: Arc<dyn NativeBrokerOperations>,
}

impl LocalBroker {
    /// Assemble a broker from its replay store, Polkit authorizer, and native
    /// backend.
    #[must_use]
    pub fn new(
        replay: Arc<dyn NonceReplayStore>,
        polkit: Arc<dyn PolkitAuthorizer>,
        native: Arc<dyn NativeBrokerOperations>,
    ) -> Self {
        Self {
            replay,
            polkit,
            native,
        }
    }

    /// Handle one request frame, returning the encoded response frame. A
    /// structural (unbindable) frame yields `Err(StructuralReject)`.
    pub fn handle_frame(
        &self,
        frame: &[u8],
        caller: &CallerContext,
        now: SystemTime,
    ) -> Result<Vec<u8>, StructuralReject> {
        let request = match BrokerRequestV1::decode_frame(frame) {
            Ok(request) => request,
            Err(RequestDecodeError::Structural(reason)) => return Err(StructuralReject(reason)),
            Err(RequestDecodeError::BoundRejection { binding, error }) => {
                // Bindable pre-dispatch rejection (version / operation /
                // parameters). No nonce was reserved; do not cache.
                return Ok(encode_not_dispatched(*binding, error));
            }
        };

        let binding = request.expected_binding();

        // 2. Authenticate the local caller and verify the channel binding.
        let peer = match &caller.peer {
            Some(peer) => peer,
            None => {
                return Ok(encode_not_dispatched(
                    binding,
                    BrokerPreDispatchError::AuthenticationFailed,
                ))
            }
        };
        let derived = peer.derive_binding();
        if derived != request.caller_binding {
            return Ok(encode_not_dispatched(
                binding,
                BrokerPreDispatchError::BindingMismatch,
            ));
        }

        // 3. Expiry (before any nonce reservation).
        if now >= request.expires_at {
            return Ok(encode_not_dispatched(
                binding,
                BrokerPreDispatchError::Expired,
            ));
        }

        // 4. Persistent nonce replay semantics.
        let caller_hex = request.caller_binding.as_hex().to_string();
        let nonce = request.nonce.as_str().to_string();
        match self
            .replay
            .check_and_reserve(&caller_hex, &nonce, request.expires_at, now)
        {
            ReplayCheck::ReplayCompleted(frame) => return Ok(frame),
            ReplayCheck::ReplayInFlight => {
                return Ok(encode_not_dispatched(
                    binding,
                    BrokerPreDispatchError::ReplayDetected,
                ))
            }
            ReplayCheck::Fresh => {}
        }

        // From here the nonce is reserved; cache every completed response so a
        // replay returns the identical bound response.
        let response_frame = self.process_reserved(&request, binding, peer, now);
        self.replay
            .record_completion(&caller_hex, &nonce, response_frame.clone());
        Ok(response_frame)
    }

    /// Steps 5–8, run after the nonce is reserved. Always returns an encoded
    /// response frame.
    fn process_reserved(
        &self,
        request: &BrokerRequestV1,
        binding: BrokerResponseBinding,
        peer: &PeerCredentials,
        now: SystemTime,
    ) -> Vec<u8> {
        let operation = &request.operation;

        // 5. Operation-specific freshness (stale plan / target identity / …).
        if let Err(error) = self.native.precheck(operation) {
            return encode_not_dispatched(binding, error);
        }

        // 6. Unsupported adapter.
        if !self.native.supports(operation) {
            return encode_not_dispatched(binding, BrokerPreDispatchError::UnsupportedAdapter);
        }

        // A deadline that elapsed after reservation but before dispatch is a
        // pre-dispatch timeout, not an uncertain outcome.
        if now >= request.expires_at {
            return encode_not_dispatched(binding, BrokerPreDispatchError::TimeoutBeforeDispatch);
        }

        // 7. Polkit authorization for the operation's registered action, bound
        //    to the authenticated caller.
        let action_id = polkit_action_id(operation);
        if let PolkitDecision::Denied = self.polkit.authorize(action_id, peer) {
            return encode_not_dispatched(binding, BrokerPreDispatchError::PolkitDenied);
        }

        // 8. Dispatch exactly one fixed native operation.
        let outcome = self.native.perform(operation);
        BrokerResponseV1::Dispatched { binding, outcome }
            .encode_frame()
            .expect("bounded response always encodes")
    }
}

fn encode_not_dispatched(binding: BrokerResponseBinding, error: BrokerPreDispatchError) -> Vec<u8> {
    BrokerResponseV1::NotDispatched { binding, error }
        .encode_frame()
        .expect("bounded response always encodes")
}
