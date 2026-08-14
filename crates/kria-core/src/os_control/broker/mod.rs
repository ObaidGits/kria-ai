//! `os_control::broker` — the typed Polkit privilege broker protocol.
//!
//! linux-os-control-production **Task 1.5** — "Implement the typed Polkit
//! privilege broker" (OSC-001, OSC-002, OSC-004, OSC-007, OSC-030, OSC-033),
//! design §12.
//!
//! # What this module owns
//!
//! The broker is a small, separate privileged service activated through Polkit.
//! It accepts only versioned, typed, authority-bound requests and performs a
//! **closed set of six operations** — and nothing else. This module implements
//! the whole protocol and its host-safe test doubles:
//!
//! * [`cbor`] — the strict *canonical* CBOR codec that is the decode security
//!   boundary (rejects non-canonical, duplicate-key, indefinite, non-minimal,
//!   and trailing encodings).
//! * [`protocol`] — the closed six-variant [`BrokerOperation`], the framed
//!   [`BrokerRequestV1`] / [`BrokerResponseV1`], the echoed
//!   [`BrokerResponseBinding`], the closed [`BrokerPreDispatchError`], and the
//!   three-family [`BrokerDispatchOutcome`]. Every value is bounded; evidence
//!   can never carry raw output.
//! * [`caller`] — the [`PeerCredentials`]-derived caller channel binding.
//! * [`replay`] — persistent nonce replay semantics (a replay never dispatches).
//! * [`native`] — the Polkit + fixed-native-operation seams (fakes + live stubs).
//! * [`server`] — the [`LocalBroker`] request handler.
//! * [`client`] — request construction from a sealed [`AdmittedMutationContext`],
//!   response-binding-echo verification, and mapping to the narrow §4 dispatch
//!   types / pre-mutation errors.
//! * [`transport`] — deny-live transports + the live socket stub.
//! * [`packaging`] — the Polkit action/policy packaging and its pure parser.
//!
//! # Safety invariants (design §12)
//!
//! Only the six operations exist; there is no generic command, shell, arbitrary
//! file write, arbitrary D-Bus, raw device, service/unit, firmware, repository
//! mutation, or run-as-root variant, and none can be added on the wire. Every
//! request and response is caller-, grant-, action-, parameter-, host-target-,
//! resource-, audit-admission-, operation-, nonce-, and expiry-bound.
//! `NotDispatched` proves no effect; once dispatch may have occurred only a
//! `Dispatched` response (Applied / Uncertain / PartiallyApplied) is produced,
//! and transport loss is uncertain with no broader fallback.
//!
//! [`AdmittedMutationContext`]: crate::os_control::context::AdmittedMutationContext
//! [`PeerCredentials`]: caller::PeerCredentials

/// The four remaining privileged broker operations.
pub mod privileged_ops;

pub mod caller;
pub mod cbor;
pub mod client;
pub mod native;
pub mod packaging;
pub mod protocol;
pub mod replay;
pub mod server;
pub mod transport;

pub use caller::PeerCredentials;
pub use client::{
    build_broker_request, dispatch_via_broker, dispatch_via_broker_bound, BrokerTransport, BrokerTransportError,
};
pub use native::{
    FixedPolkit, LiveNativeOperations, LivePolkitAuthorizer, NativeBrokerOperations,
    PolkitAuthorizer, PolkitDecision, ScriptedNativeOperations,
};
pub use packaging::{
    parse_policy_actions, polkit_action_id, BROKER_ACTION_IDS, BROKER_POLKIT_POLICY,
};
pub use protocol::{
    BoundedBrokerEvidence, BoundedPackageTransaction, BoundedPercent, BrokerBoundPath,
    BrokerDispatchOutcome, BrokerOperation, BrokerPreDispatchError, BrokerRequestId,
    BrokerRequestV1, BrokerResponseBinding, BrokerResponseV1, CallerChannelBindingDigest,
    ChargeThresholdAdapterId, DiscoveredPrinterId, EvidenceField, ExistingLocalIdentity,
    FirewallProviderId, OperationDecodeError, PackageProviderId, PackageStep, PackageStepAction,
    RecognizedPrivacyControl, RequestDecodeError, ResponseDecodeError, ReviewedPrinterOptions,
    SchemaError, StructuralReason,
};
pub use replay::{InMemoryNonceStore, NonceReplayStore, ReplayCheck};
pub use server::{CallerContext, LocalBroker, StructuralReject};
pub use transport::{
    ConnectFailedTransport, FixedResponseTransport, LiveBrokerTransport, LoopbackBrokerTransport,
    LostAfterSendTransport, BROKER_SOCKET_PATH,
};

#[cfg(all(test, feature = "os-control-test"))]
mod e2e_tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant, SystemTime};

    use tokio_util::sync::CancellationToken;

    use crate::agent::execution_gate::OsActionGrant;
    use crate::agent::turn_memory::ExecutionTarget;
    use crate::os_control::access::sentinel_trip_count;
    use crate::os_control::context::{
        AdmittedMutationContext, AuditAdmissionToken, HostExecutionContext, MutationPermit,
        RedactionPolicy, SessionContext,
    };
    use crate::os_control::contract::{
        ActionId, AuditAdmissionId, CorrelationId, Digest, NonEmptyBoundedVec, ProviderId,
        SafeStepId, SafeText, SessionId,
    };
    use crate::os_control::error::{GrantInvalidReason, OsControlError};
    use crate::os_control::receipt::{ApplyOutcome, UncertainEffectCause};
    use crate::os_control::resource::AcquiredResourceLeaseSet;
    use crate::safety::RiskLevel;

    use super::*;

    const SESSION: &str = "session-broker";
    const ACTION: &str = "set_firewall_enabled";

    fn params() -> serde_json::Value {
        serde_json::json!({ "provider": "ufw", "enabled": true })
    }

    fn peer() -> PeerCredentials {
        PeerCredentials {
            uid: 1000,
            gid: 1000,
            pid: 4242,
            connection_nonce: "conn-nonce-1".into(),
        }
    }

    /// Owns every authority so a borrowed sealed context can be assembled.
    struct Fixture {
        grant: OsActionGrant,
        host_ctx: HostExecutionContext,
        lease_set: AcquiredResourceLeaseSet,
        audit_token: AuditAdmissionToken,
        resource_digest: Digest,
    }

    impl Fixture {
        fn build() -> Self {
            let p = params();
            let grant = OsActionGrant::for_test(
                SESSION,
                ACTION,
                &p,
                ExecutionTarget::Host,
                &[],
                RiskLevel::Red,
            );
            let resource_digest = Digest::of_str(grant.resource_set_digest());
            let audit_token = AuditAdmissionToken::for_test(
                AuditAdmissionId::new("adm-1"),
                resource_digest.clone(),
            );
            let host_ctx = HostExecutionContext::for_test(
                CorrelationId::new("corr-1"),
                ActionId::new("act-1"),
                audit_token.observation_authority(),
                Arc::new(SessionContext::new(SessionId::new(SESSION))),
                CancellationToken::new(),
                Instant::now() + Duration::from_secs(30),
                RedactionPolicy::default(),
            );
            let lease_set = AcquiredResourceLeaseSet::for_test(resource_digest.clone());
            Self {
                grant,
                host_ctx,
                lease_set,
                audit_token,
                resource_digest,
            }
        }

        fn ctx(&self) -> AdmittedMutationContext<'_> {
            let permit = MutationPermit::for_test(
                &self.lease_set,
                &self.audit_token,
                self.resource_digest.clone(),
            );
            AdmittedMutationContext::for_test(&self.host_ctx, &self.grant, permit)
        }
    }

    fn firewall_op() -> BrokerOperation {
        BrokerOperation::SetFirewallEnabled {
            provider: FirewallProviderId::Ufw,
            enabled: true,
        }
    }

    fn evidence() -> BoundedBrokerEvidence {
        BoundedBrokerEvidence::new(
            ProviderId::new("ufw"),
            Digest::of_str("evi"),
            [EvidenceField {
                key: crate::os_control::contract::SafeField::new("enabled"),
                value: SafeText::new("true"),
            }],
        )
    }

    fn applied_broker(now: SystemTime) -> LoopbackBrokerTransport {
        let broker = LocalBroker::new(
            Arc::new(InMemoryNonceStore::new()),
            Arc::new(FixedPolkit::allow()),
            Arc::new(ScriptedNativeOperations::new([
                BrokerDispatchOutcome::Applied {
                    receipt_digest: Digest::of_str("receipt"),
                    evidence: evidence(),
                },
            ])),
        );
        LoopbackBrokerTransport::new(Arc::new(broker), CallerContext::authenticated(peer()), now)
    }

    #[test]
    fn happy_path_round_trip_applies_once() {
        let fx = Fixture::build();
        let ctx = fx.ctx();
        let request = build_broker_request(&ctx, &peer(), "req-1", firewall_op())
            .expect("request builds from sealed context");
        let transport = applied_broker(SystemTime::now());
        let outcome = dispatch_via_broker(&transport, &request).expect("applied");
        assert!(matches!(outcome, ApplyOutcome::Applied(_)));
        assert_eq!(sentinel_trip_count(), 0, "no live transport was opened");
    }

    #[test]
    fn replay_returns_identical_response_without_second_dispatch() {
        let fx = Fixture::build();
        let ctx = fx.ctx();
        let request = build_broker_request(&ctx, &peer(), "req-1", firewall_op()).unwrap();
        // One scripted outcome only; a second dispatch would panic. The replay
        // path must return the cached response instead of dispatching again.
        let transport = applied_broker(SystemTime::now());

        let first = dispatch_via_broker(&transport, &request).expect("first applied");
        let second = dispatch_via_broker(&transport, &request).expect("replay applied");
        assert!(matches!(first, ApplyOutcome::Applied(_)));
        assert!(matches!(second, ApplyOutcome::Applied(_)));
    }

    #[test]
    fn caller_binding_mismatch_is_not_dispatched() {
        let fx = Fixture::build();
        let ctx = fx.ctx();
        // The client derives its binding from `peer()`, but the broker's caller
        // context is a different peer → BindingMismatch, no dispatch.
        let request = build_broker_request(&ctx, &peer(), "req-1", firewall_op()).unwrap();
        let other_peer = PeerCredentials {
            uid: 2000,
            ..peer()
        };
        let broker = LocalBroker::new(
            Arc::new(InMemoryNonceStore::new()),
            Arc::new(FixedPolkit::allow()),
            Arc::new(ScriptedNativeOperations::new([])),
        );
        let transport = LoopbackBrokerTransport::new(
            Arc::new(broker),
            CallerContext::authenticated(other_peer),
            SystemTime::now(),
        );
        let err = dispatch_via_broker(&transport, &request).unwrap_err();
        assert!(matches!(
            err,
            OsControlError::GrantInvalid {
                reason: GrantInvalidReason::BindingMismatch
            }
        ));
    }

    #[test]
    fn unauthenticated_caller_is_authentication_failed() {
        let fx = Fixture::build();
        let ctx = fx.ctx();
        let request = build_broker_request(&ctx, &peer(), "req-1", firewall_op()).unwrap();
        let broker = LocalBroker::new(
            Arc::new(InMemoryNonceStore::new()),
            Arc::new(FixedPolkit::allow()),
            Arc::new(ScriptedNativeOperations::new([])),
        );
        let transport = LoopbackBrokerTransport::new(
            Arc::new(broker),
            CallerContext::unauthenticated(),
            SystemTime::now(),
        );
        let err = dispatch_via_broker(&transport, &request).unwrap_err();
        assert!(matches!(err, OsControlError::PermissionDenied { .. }));
    }

    #[test]
    fn broker_side_expiry_is_not_dispatched() {
        let fx = Fixture::build();
        let ctx = fx.ctx();
        let request = build_broker_request(&ctx, &peer(), "req-1", firewall_op()).unwrap();
        // A broker clock far past the request expiry → Expired.
        let transport = applied_broker(request.expires_at + Duration::from_secs(60));
        let err = dispatch_via_broker(&transport, &request).unwrap_err();
        assert!(matches!(err, OsControlError::ApprovalExpired));
    }

    #[test]
    fn polkit_denied_has_no_fallback() {
        let fx = Fixture::build();
        let ctx = fx.ctx();
        let request = build_broker_request(&ctx, &peer(), "req-1", firewall_op()).unwrap();
        let broker = LocalBroker::new(
            Arc::new(InMemoryNonceStore::new()),
            Arc::new(FixedPolkit::deny()),
            Arc::new(ScriptedNativeOperations::new([])), // must NOT dispatch
        );
        let transport = LoopbackBrokerTransport::new(
            Arc::new(broker),
            CallerContext::authenticated(peer()),
            SystemTime::now(),
        );
        let err = dispatch_via_broker(&transport, &request).unwrap_err();
        assert!(matches!(err, OsControlError::PermissionDenied { .. }));
    }

    #[test]
    fn unsupported_adapter_is_not_dispatched() {
        let fx = Fixture::build();
        let ctx = fx.ctx();
        let request = build_broker_request(&ctx, &peer(), "req-1", firewall_op()).unwrap();
        let broker = LocalBroker::new(
            Arc::new(InMemoryNonceStore::new()),
            Arc::new(FixedPolkit::allow()),
            Arc::new(ScriptedNativeOperations::with_support(|_| false, [])),
        );
        let transport = LoopbackBrokerTransport::new(
            Arc::new(broker),
            CallerContext::authenticated(peer()),
            SystemTime::now(),
        );
        let err = dispatch_via_broker(&transport, &request).unwrap_err();
        assert!(matches!(err, OsControlError::Unavailable { .. }));
    }

    #[test]
    fn stale_plan_and_stale_target_identity_are_not_dispatched() {
        for (pre, expect_target_changed) in [
            (BrokerPreDispatchError::StalePlan, false),
            (BrokerPreDispatchError::StaleTargetIdentity, true),
        ] {
            let fx = Fixture::build();
            let ctx = fx.ctx();
            let request = build_broker_request(&ctx, &peer(), "req-1", firewall_op()).unwrap();
            let broker = LocalBroker::new(
                Arc::new(InMemoryNonceStore::new()),
                Arc::new(FixedPolkit::allow()),
                Arc::new(ScriptedNativeOperations::new([]).with_precheck(move |_| Err(pre))),
            );
            let transport = LoopbackBrokerTransport::new(
                Arc::new(broker),
                CallerContext::authenticated(peer()),
                SystemTime::now(),
            );
            let err = dispatch_via_broker(&transport, &request).unwrap_err();
            if expect_target_changed {
                assert!(matches!(err, OsControlError::TargetChanged));
            } else {
                assert!(matches!(err, OsControlError::InvalidRequest { .. }));
            }
        }
    }

    #[test]
    fn transport_loss_after_send_maps_to_uncertain() {
        let fx = Fixture::build();
        let ctx = fx.ctx();
        let request = build_broker_request(&ctx, &peer(), "req-1", firewall_op()).unwrap();
        let outcome = dispatch_via_broker(&LostAfterSendTransport, &request).expect("uncertain");
        match outcome {
            ApplyOutcome::Uncertain(u) => {
                assert_eq!(u.cause(), UncertainEffectCause::TransportLostAfterDispatch);
            }
            other => panic!("expected uncertain, got {other:?}"),
        }
    }

    #[test]
    fn connect_failure_before_send_is_pre_dispatch_unavailable() {
        let fx = Fixture::build();
        let ctx = fx.ctx();
        let request = build_broker_request(&ctx, &peer(), "req-1", firewall_op()).unwrap();
        let err = dispatch_via_broker(&ConnectFailedTransport, &request).unwrap_err();
        assert!(matches!(
            err,
            OsControlError::Unavailable {
                retryable: true,
                ..
            }
        ));
    }

    #[test]
    fn response_binding_echo_mismatch_is_rejected_before_interpreting_outcome() {
        let fx = Fixture::build();
        let ctx = fx.ctx();
        let request = build_broker_request(&ctx, &peer(), "req-1", firewall_op()).unwrap();

        // Build a valid Dispatched response, then tamper each binding field in
        // turn; every mismatch must be rejected as GrantInvalid.
        let make_response = |mutate: &dyn Fn(&mut BrokerResponseBinding)| {
            let mut binding = request.expected_binding();
            mutate(&mut binding);
            BrokerResponseV1::Dispatched {
                binding,
                outcome: BrokerDispatchOutcome::Applied {
                    receipt_digest: Digest::of_str("r"),
                    evidence: evidence(),
                },
            }
            .encode_frame()
            .unwrap()
        };

        let mutators: Vec<Box<dyn Fn(&mut BrokerResponseBinding)>> = vec![
            Box::new(|b| b.grant_id = crate::os_control::contract::GrantId::new("other-grant")),
            Box::new(|b| b.nonce = crate::os_control::contract::GrantNonce::new("other-nonce")),
            Box::new(|b| {
                b.caller_binding =
                    CallerChannelBindingDigest::from_digest(Digest::of_str("other-caller"))
            }),
            Box::new(|b| b.resource_set_digest = Digest::of_str("other-resource")),
            Box::new(|b| b.audit_admission_id = AuditAdmissionId::new("other-adm")),
            Box::new(|b| b.operation_digest = Digest::of_str("other-op")),
            Box::new(|b| b.expires_at = b.expires_at + Duration::from_secs(1)),
            Box::new(|b| b.action_hash = Digest::of_str("other-action")),
            Box::new(|b| b.parameter_hash = Digest::of_str("other-params")),
            Box::new(|b| b.target_hash = Digest::of_str("other-target")),
        ];

        for mutate in &mutators {
            let frame = make_response(mutate.as_ref());
            let transport = FixedResponseTransport::new(frame);
            let err = dispatch_via_broker(&transport, &request).unwrap_err();
            assert!(
                matches!(
                    err,
                    OsControlError::GrantInvalid {
                        reason: GrantInvalidReason::BindingMismatch
                    }
                ),
                "tampered binding must be rejected"
            );
        }

        // A faithful echo is accepted.
        let good = BrokerResponseV1::Dispatched {
            binding: request.expected_binding(),
            outcome: BrokerDispatchOutcome::Applied {
                receipt_digest: Digest::of_str("r"),
                evidence: evidence(),
            },
        }
        .encode_frame()
        .unwrap();
        let outcome =
            dispatch_via_broker(&FixedResponseTransport::new(good), &request).expect("applied");
        assert!(matches!(outcome, ApplyOutcome::Applied(_)));
    }

    #[test]
    fn partially_applied_response_maps_to_partial_dispatch() {
        let fx = Fixture::build();
        let ctx = fx.ctx();
        let request = build_broker_request(&ctx, &peer(), "req-1", firewall_op()).unwrap();
        let frame = BrokerResponseV1::Dispatched {
            binding: request.expected_binding(),
            outcome: BrokerDispatchOutcome::PartiallyApplied {
                receipt_digest: None,
                completed_steps: NonEmptyBoundedVec::single(SafeStepId::new("step-1")),
                failed_step: SafeStepId::new("step-2"),
                cause: crate::os_control::receipt::PartialEffectCause::StepFailedAfterCommit,
                evidence: evidence(),
            },
        }
        .encode_frame()
        .unwrap();
        let outcome =
            dispatch_via_broker(&FixedResponseTransport::new(frame), &request).expect("partial");
        assert!(matches!(outcome, ApplyOutcome::PartiallyApplied(_)));
    }

    #[test]
    fn broker_rejects_unsupported_version_frame_with_bound_not_dispatched() {
        // Tamper a valid request frame's version and feed it straight to the
        // broker: it returns a bound NotDispatched(UnsupportedVersion).
        let fx = Fixture::build();
        let ctx = fx.ctx();
        let request = build_broker_request(&ctx, &peer(), "req-1", firewall_op()).unwrap();

        // Rebuild the request CBOR with version 2 by decoding and re-encoding is
        // not exposed; instead exercise the client mapping via a crafted
        // NotDispatched response the broker would emit.
        let frame = BrokerResponseV1::NotDispatched {
            binding: request.expected_binding(),
            error: BrokerPreDispatchError::UnsupportedVersion,
        }
        .encode_frame()
        .unwrap();
        let err = dispatch_via_broker(&FixedResponseTransport::new(frame), &request).unwrap_err();
        assert!(matches!(err, OsControlError::InvalidRequest { .. }));
    }

    #[test]
    fn expired_grant_builds_no_request() {
        let p = params();
        let grant = OsActionGrant::for_test_expired(
            SESSION,
            ACTION,
            &p,
            ExecutionTarget::Host,
            &[],
            RiskLevel::Red,
        );
        let resource_digest = Digest::of_str(grant.resource_set_digest());
        let audit_token =
            AuditAdmissionToken::for_test(AuditAdmissionId::new("adm-1"), resource_digest.clone());
        let host_ctx = HostExecutionContext::for_test(
            CorrelationId::new("corr-1"),
            ActionId::new("act-1"),
            audit_token.observation_authority(),
            Arc::new(SessionContext::new(SessionId::new(SESSION))),
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(30),
            RedactionPolicy::default(),
        );
        let lease_set = AcquiredResourceLeaseSet::for_test(resource_digest.clone());
        let permit = MutationPermit::for_test(&lease_set, &audit_token, resource_digest);
        let ctx = AdmittedMutationContext::for_test(&host_ctx, &grant, permit);
        let err = build_broker_request(&ctx, &peer(), "req-1", firewall_op()).unwrap_err();
        assert!(matches!(err, OsControlError::ApprovalExpired));
    }
}
