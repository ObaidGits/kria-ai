//! The KRIA-side broker client: builds authority-bound requests from a sealed
//! mutation context, validates the echoed response binding, and maps outcomes
//! to the narrow §4 dispatch types.
//!
//! linux-os-control-production **Task 1.5**, design §12
//! (OSC-001, OSC-005, OSC-007).
//!
//! # Construction authority
//!
//! [`build_broker_request`] takes a borrowed [`AdmittedMutationContext`], so a
//! broker request — like a structured command — cannot exist without the full
//! governed lifecycle (approval + resource leases + audit admission). The
//! request copies the grant/resource/audit bindings out of the sealed context.
//!
//! # Response handling
//!
//! The client rejects a response whose binding does not byte-for-byte echo the
//! request's binding *before* interpreting the outcome (design §12). A
//! `NotDispatched` response maps to a pre-mutation [`OsControlError`] (proving no
//! effect); a `Dispatched` response maps to an [`ApplyOutcome`]. Transport loss
//! **after** the request was sent is an uncertain outcome, never a pre-dispatch
//! error and never a fallback trigger.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::agent::turn_memory::ExecutionTarget;
use crate::os_control::context::AdmittedMutationContext;
use crate::os_control::contract::{
    AuditAdmissionId, BoundedVec, Digest, GrantId, GrantNonce, SafeField, SafeOperation, SafeText,
};
use crate::os_control::error::{GrantInvalidReason, OsControlError};
use crate::os_control::receipt::{
    AppliedDispatch, ApplyOutcome, PartialDispatch, UncertainDispatch, UncertainEffectCause,
};

use super::caller::PeerCredentials;
use super::protocol::{
    BrokerDispatchOutcome, BrokerOperation, BrokerPreDispatchError, BrokerRequestId,
    BrokerRequestV1, BrokerResponseV1, ResponseDecodeError,
};

/// A transport error surfaced by [`BrokerTransport::round_trip`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokerTransportError {
    /// The connection could not be established / the request was never sent.
    /// This is *before* dispatch, so the client maps it to a pre-dispatch error.
    ConnectFailed,
    /// The request was sent but the response was lost. Dispatch may have
    /// occurred, so the client maps this to an uncertain outcome.
    LostAfterSend,
}

/// A single-request broker transport (one request per authenticated local
/// connection, design §12).
pub trait BrokerTransport {
    /// Send one request frame and read the response frame.
    fn round_trip(&self, request_frame: &[u8]) -> Result<Vec<u8>, BrokerTransportError>;

    /// Send a frame together with the connection nonce the request's caller
    /// binding was derived from.
    ///
    /// The broker derives its own binding from the kernel-supplied peer
    /// credentials (`SO_PEERCRED`) **plus** this nonce. uid/gid/pid come from the
    /// kernel and cannot be spoofed; the nonce only has to be unique per
    /// connection, so the caller choosing it is safe — replaying a captured frame
    /// means replaying its nonce, which the persistent replay store rejects.
    ///
    /// Defaults to [`Self::round_trip`] for in-process fakes, which have no
    /// connection to bind to.
    fn round_trip_bound(
        &self,
        request_frame: &[u8],
        connection_nonce: &str,
    ) -> Result<Vec<u8>, BrokerTransportError> {
        let _ = connection_nonce;
        self.round_trip(request_frame)
    }
}

/// Build an authority-bound broker request from a sealed mutation context.
///
/// Pre-dispatch validation mirrors the structured-command builder: an expired
/// grant or a non-host target is a pre-mutation error proving no effect.
pub fn build_broker_request(
    ctx: &AdmittedMutationContext<'_>,
    caller: &PeerCredentials,
    request_id: impl Into<String>,
    operation: BrokerOperation,
) -> Result<BrokerRequestV1, OsControlError> {
    let grant = ctx.grant();
    let observation = ctx.observation();

    if grant.is_expired(SystemTime::now()) {
        return Err(OsControlError::ApprovalExpired);
    }
    if grant.target() != ExecutionTarget::Host {
        return Err(OsControlError::InvalidRequest {
            field: SafeField::new("target"),
            reason: SafeText::new("broker operations are host-only; non-host targets are rejected"),
        });
    }

    Ok(BrokerRequestV1 {
        request_id: BrokerRequestId::new(request_id),
        caller_binding: caller.derive_binding(),
        operation_digest: operation.operation_digest(),
        operation,
        grant_id: GrantId::new(grant.grant_id().as_str()),
        action_hash: Digest::of_str(grant.action()),
        parameter_hash: Digest::from_hex(grant.params_digest()),
        target_hash: Digest::of_str(grant.target().as_str()),
        resource_set_digest: Digest::from_hex(grant.resource_set_digest()),
        audit_admission_id: AuditAdmissionId::new(
            observation.observation_audit().admission_id().as_str(),
        ),
        nonce: GrantNonce::new(grant.nonce().as_str()),
        // Normalize to millisecond precision so the wire encoding of `expires_at`
        // is lossless and the response binding echoes it byte-for-byte.
        expires_at: truncate_to_millis(grant.expires_at()),
    })
}

/// Truncate a [`SystemTime`] to whole-millisecond precision (the wire
/// resolution of `expires_at`), so encode→decode is lossless.
fn truncate_to_millis(t: SystemTime) -> SystemTime {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => {
            UNIX_EPOCH + Duration::from_millis(u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        }
        Err(_) => t,
    }
}

/// Send a request through the transport and resolve it to a narrow dispatch
/// outcome or a pre-mutation error.
///
/// * A `NotDispatched` response → `Err(OsControlError)` (no effect).
/// * A `Dispatched` response (after binding echo verification) → `Ok(ApplyOutcome)`.
/// * Transport loss after send → `Ok(ApplyOutcome::Uncertain(TransportLost…))`.
/// * A binding that does not echo the request → `Err(GrantInvalid)`.
pub fn dispatch_via_broker(
    transport: &dyn BrokerTransport,
    request: &BrokerRequestV1,
) -> Result<ApplyOutcome, OsControlError> {
    dispatch_via_broker_bound(transport, request, "")
}

/// As [`dispatch_via_broker`], but carrying the connection nonce the request's
/// caller binding was derived from so the broker can derive the identical value.
pub fn dispatch_via_broker_bound(
    transport: &dyn BrokerTransport,
    request: &BrokerRequestV1,
    connection_nonce: &str,
) -> Result<ApplyOutcome, OsControlError> {
    let frame = request
        .encode_frame()
        .map_err(|_| OsControlError::InvalidRequest {
            field: SafeField::new("request"),
            reason: SafeText::new("request exceeds the broker frame bound"),
        })?;

    let response_frame = match transport.round_trip_bound(&frame, connection_nonce) {
        Ok(bytes) => bytes,
        // Sent but lost: dispatch may have occurred → uncertain, no fallback.
        Err(BrokerTransportError::LostAfterSend) => {
            return Ok(ApplyOutcome::Uncertain(UncertainDispatch::new(
                None,
                UncertainEffectCause::TransportLostAfterDispatch,
                BoundedVec::new(),
            )));
        }
        // Never sent: pre-dispatch, retryable.
        Err(BrokerTransportError::ConnectFailed) => {
            return Err(OsControlError::Unavailable {
                provider: None,
                reason: SafeText::new("broker connection failed before dispatch"),
                retryable: true,
            });
        }
    };

    let response = BrokerResponseV1::decode_frame(&response_frame).map_err(map_response_decode)?;

    // Reject any response whose binding does not byte-for-byte echo the request
    // before interpreting its outcome (design §12).
    if *response.binding() != request.expected_binding() {
        return Err(OsControlError::GrantInvalid {
            reason: GrantInvalidReason::BindingMismatch,
        });
    }

    match response {
        BrokerResponseV1::NotDispatched { error, .. } => {
            Err(map_pre_dispatch_error(error, request.operation.token()))
        }
        BrokerResponseV1::Dispatched { outcome, .. } => Ok(map_outcome(outcome)),
    }
}

fn map_response_decode(_error: ResponseDecodeError) -> OsControlError {
    // A malformed response cannot be trusted; treat it as an unbindable protocol
    // failure. We have not observed a valid dispatch, so it is pre-mutation.
    OsControlError::GrantInvalid {
        reason: GrantInvalidReason::BindingMismatch,
    }
}

/// Map a `Dispatched` outcome to the narrow §4 dispatch type. The bounded broker
/// evidence is verified by the runtime's verification step; the outcome carries
/// the provider receipt digest.
fn map_outcome(outcome: BrokerDispatchOutcome) -> ApplyOutcome {
    match outcome {
        BrokerDispatchOutcome::Applied { receipt_digest, .. } => ApplyOutcome::Applied(
            AppliedDispatch::new(Some(receipt_digest), BoundedVec::new()),
        ),
        BrokerDispatchOutcome::Uncertain {
            receipt_digest,
            cause,
            ..
        } => ApplyOutcome::Uncertain(UncertainDispatch::new(
            receipt_digest,
            cause,
            BoundedVec::new(),
        )),
        BrokerDispatchOutcome::PartiallyApplied {
            receipt_digest,
            completed_steps,
            failed_step,
            cause,
            ..
        } => ApplyOutcome::PartiallyApplied(PartialDispatch::new(
            receipt_digest,
            completed_steps,
            failed_step,
            cause,
            BoundedVec::new(),
        )),
    }
}

/// Map a closed pre-dispatch error to the canonical pre-mutation error taxonomy.
/// Every mapping is a proven-no-effect [`OsControlError`].
fn map_pre_dispatch_error(error: BrokerPreDispatchError, operation_token: &str) -> OsControlError {
    match error {
        BrokerPreDispatchError::AuthenticationFailed => OsControlError::PermissionDenied {
            authority: SafeText::new("broker"),
            remediation: SafeText::new("re-establish an authenticated local connection"),
        },
        BrokerPreDispatchError::BindingMismatch => OsControlError::GrantInvalid {
            reason: GrantInvalidReason::BindingMismatch,
        },
        BrokerPreDispatchError::ReplayDetected => OsControlError::GrantInvalid {
            reason: GrantInvalidReason::NonceReused,
        },
        BrokerPreDispatchError::Expired => OsControlError::ApprovalExpired,
        BrokerPreDispatchError::UnsupportedVersion => OsControlError::InvalidRequest {
            field: SafeField::new("protocol_version"),
            reason: SafeText::new("broker protocol version is unsupported"),
        },
        BrokerPreDispatchError::UnsupportedOperation => OsControlError::InvalidRequest {
            field: SafeField::new("operation"),
            reason: SafeText::new("broker operation is not in the closed operation set"),
        },
        BrokerPreDispatchError::InvalidParameters => OsControlError::InvalidRequest {
            field: SafeField::new("parameters"),
            reason: SafeText::new("broker operation parameters failed validation"),
        },
        BrokerPreDispatchError::StalePlan => OsControlError::InvalidRequest {
            field: SafeField::new("plan"),
            reason: SafeText::new("the approved plan is stale versus current state"),
        },
        BrokerPreDispatchError::StaleTargetIdentity => OsControlError::TargetChanged,
        BrokerPreDispatchError::UnsupportedAdapter => OsControlError::Unavailable {
            provider: None,
            reason: SafeText::new("no supported broker adapter for this operation"),
            retryable: false,
        },
        BrokerPreDispatchError::PolkitDenied => OsControlError::PermissionDenied {
            authority: SafeText::new("polkit"),
            remediation: SafeText::new("authenticate as an administrator"),
        },
        BrokerPreDispatchError::TimeoutBeforeDispatch => OsControlError::TimedOutBeforeMutation {
            operation: SafeOperation::new(operation_token),
            timeout_ms: 0,
        },
    }
}
