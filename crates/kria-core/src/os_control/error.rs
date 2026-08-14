//! Canonical, pre-mutation-only OS-control error taxonomy and frozen envelope.
//!
//! linux-os-control-production **Task 1.1**, design §5 (OSC-001, OSC-005).
//!
//! [`OsControlError`] is *exclusively* pre-mutation or proven-no-effect: its
//! presence is a proof that **no host effect started**. Post-dispatch facts
//! (uncertain / partial / verification failure) are never errors; they live in
//! [`crate::os_control::receipt::MutationReceipt`]. This split is what lets the
//! runtime guarantee that an `Err(OsControlError)` from a provider mutator means
//! the mutation did not begin (design §4, §5).
//!
//! Every error serializes through **one frozen envelope** (design §5): absent
//! values are JSON `null`, not omitted, so adapters cannot invent divergent
//! shapes. The code set is closed and versioned by Task 0.1.

use crate::os_control::contract::{
    BoundedVec, CapabilityId, ProviderId, SafeCandidate, SafeField, SafeOperation, SafeResource,
    SafeText,
};

/// Closed reason a grant failed validation before mutation (design §5).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantInvalidReason {
    /// The grant's binding digest does not match the live action/params/target.
    BindingMismatch,
    /// The grant nonce was already consumed (replay).
    NonceReused,
    /// The grant references a capability snapshot revision that has changed.
    StaleSnapshot,
    /// The grant was never issued by `ExecutionGate` (forged / wrong origin).
    NotIssuedByGate,
    /// The grant's session does not match the current session.
    SessionMismatch,
}

impl GrantInvalidReason {
    /// Stable snake_case token for envelopes/traces.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BindingMismatch => "binding_mismatch",
            Self::NonceReused => "nonce_reused",
            Self::StaleSnapshot => "stale_snapshot",
            Self::NotIssuedByGate => "not_issued_by_gate",
            Self::SessionMismatch => "session_mismatch",
        }
    }
}

/// The canonical pre-mutation / proven-no-effect error taxonomy (design §5).
///
/// Never carries raw stderr, D-Bus payloads, command strings, correlatable
/// secret references, control characters, or provider object paths — every
/// field is a bounded, redacted safe-value newtype from
/// [`crate::os_control::contract`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OsControlError {
    /// The capability is not supported by any eligible provider.
    Unsupported {
        /// Capability that has no safe adapter.
        capability: CapabilityId,
        /// Redacted reason.
        reason: SafeText,
    },
    /// A provider or dependency is temporarily unavailable.
    Unavailable {
        /// Provider, if one was selected.
        provider: Option<ProviderId>,
        /// Redacted reason.
        reason: SafeText,
        /// Whether a retry may succeed.
        retryable: bool,
    },
    /// The request failed strict schema / semantic validation.
    InvalidRequest {
        /// Field at fault.
        field: SafeField,
        /// Redacted reason.
        reason: SafeText,
    },
    /// The target could not be uniquely resolved.
    AmbiguousTarget {
        /// Redacted target kind.
        kind: SafeText,
        /// Bounded redacted candidate set.
        candidates: BoundedVec<SafeCandidate>,
    },
    /// The OS authority (e.g. Polkit) denied the operation; no fallback.
    PermissionDenied {
        /// Redacted authority label.
        authority: SafeText,
        /// Redacted remediation.
        remediation: SafeText,
    },
    /// KRIA policy denied the operation.
    PolicyDenied {
        /// Redacted reason.
        reason: SafeText,
    },
    /// The durable approval expired before mutation.
    ApprovalExpired,
    /// The execution grant was invalid (stale / forged / mismatched).
    GrantInvalid {
        /// Closed reason.
        reason: GrantInvalidReason,
    },
    /// The target changed between approval and resume.
    TargetChanged,
    /// A required exclusive resource is held by another action.
    ResourceBusy {
        /// Redacted resource label.
        resource: SafeResource,
        /// Optional redacted owner label.
        owner: Option<SafeText>,
    },
    /// A bounded deadline elapsed before any mutation was attempted.
    TimedOutBeforeMutation {
        /// Redacted operation label.
        operation: SafeOperation,
        /// The elapsed timeout in milliseconds.
        timeout_ms: u64,
    },
    /// The request was cancelled before any mutation was attempted.
    CancelledBeforeMutation,
    /// A provider protocol error occurred before dispatch (proven no effect).
    ProtocolBeforeMutation {
        /// Provider at fault.
        provider: ProviderId,
        /// Redacted operation label.
        operation: SafeOperation,
    },
    /// Durable audit admission failed; the runtime fails closed before mutation.
    AuditUnavailable,
}

impl OsControlError {
    /// The closed, versioned error code string (design §5).
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unsupported { .. } => "os_control.unsupported",
            Self::Unavailable { .. } => "os_control.unavailable",
            Self::InvalidRequest { .. } => "os_control.invalid_request",
            Self::AmbiguousTarget { .. } => "os_control.ambiguous_target",
            Self::PermissionDenied { .. } => "os_control.permission_denied",
            Self::PolicyDenied { .. } => "os_control.policy_denied",
            Self::ApprovalExpired => "os_control.approval_expired",
            Self::GrantInvalid { .. } => "os_control.grant_invalid",
            Self::TargetChanged => "os_control.target_changed",
            Self::ResourceBusy { .. } => "os_control.resource_busy",
            Self::TimedOutBeforeMutation { .. } => "os_control.timed_out_before_mutation",
            Self::CancelledBeforeMutation => "os_control.cancelled_before_mutation",
            Self::ProtocolBeforeMutation { .. } => "os_control.protocol_before_mutation",
            Self::AuditUnavailable => "os_control.audit_unavailable",
        }
    }

    /// Whether a retry could plausibly succeed.
    #[must_use]
    pub fn retryable(&self) -> bool {
        match self {
            Self::Unavailable { retryable, .. } => *retryable,
            Self::ResourceBusy { .. } | Self::TimedOutBeforeMutation { .. } => true,
            _ => false,
        }
    }

    /// A bounded, redacted, human-safe message.
    #[must_use]
    pub fn message(&self) -> SafeText {
        match self {
            Self::Unsupported { reason, .. }
            | Self::Unavailable { reason, .. }
            | Self::PolicyDenied { reason }
            | Self::InvalidRequest { reason, .. } => reason.clone(),
            Self::AmbiguousTarget { kind, .. } => {
                SafeText::new(format!("ambiguous target: {}", kind.as_str()))
            }
            Self::PermissionDenied { authority, .. } => {
                SafeText::new(format!("permission denied by {}", authority.as_str()))
            }
            Self::ApprovalExpired => SafeText::new("approval expired before execution"),
            Self::GrantInvalid { reason } => {
                SafeText::new(format!("execution grant invalid: {}", reason.as_str()))
            }
            Self::TargetChanged => SafeText::new("target changed since approval"),
            Self::ResourceBusy { resource, .. } => {
                SafeText::new(format!("resource busy: {}", resource.as_str()))
            }
            Self::TimedOutBeforeMutation { operation, .. } => {
                SafeText::new(format!("timed out before mutation: {}", operation.as_str()))
            }
            Self::CancelledBeforeMutation => SafeText::new("cancelled before mutation"),
            Self::ProtocolBeforeMutation { operation, .. } => SafeText::new(format!(
                "provider protocol error before mutation: {}",
                operation.as_str()
            )),
            Self::AuditUnavailable => SafeText::new("durable audit admission unavailable"),
        }
    }

    /// Redacted remediation, if any.
    #[must_use]
    pub fn remediation(&self) -> Option<SafeText> {
        match self {
            Self::PermissionDenied { remediation, .. } => Some(remediation.clone()),
            _ => None,
        }
    }

    /// The field at fault, if any.
    #[must_use]
    pub fn field(&self) -> Option<SafeField> {
        match self {
            Self::InvalidRequest { field, .. } => Some(field.clone()),
            _ => None,
        }
    }

    /// The provider referenced, if any.
    #[must_use]
    pub fn provider(&self) -> Option<ProviderId> {
        match self {
            Self::Unavailable { provider, .. } => provider.clone(),
            Self::ProtocolBeforeMutation { provider, .. } => Some(provider.clone()),
            _ => None,
        }
    }

    /// Whether this error implies the affected capability is unavailable.
    #[must_use]
    pub fn availability(&self) -> &'static str {
        match self {
            Self::Unsupported { .. } => "unavailable",
            Self::Unavailable { .. } | Self::AuditUnavailable => "unavailable",
            _ => "available",
        }
    }

    /// Serialize to the single frozen error envelope (design §5). Absent values
    /// are explicit JSON `null`, never omitted.
    #[must_use]
    pub fn to_envelope(&self) -> serde_json::Value {
        serde_json::json!({
            "error": {
                "code": self.code(),
                "message": self.message().as_str(),
                "retryable": self.retryable(),
                "remediation": self.remediation().map(|r| r.as_str().to_string()),
                "field": self.field().map(|f| f.as_str().to_string()),
            },
            "os_control": {
                "provider": self.provider().map(|p| p.as_str().to_string()),
                "lifecycle": serde_json::Value::Null,
                "availability": self.availability(),
                "receipt_summary": serde_json::Value::Null,
            }
        })
    }
}

impl std::fmt::Display for OsControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code(), self.message())
    }
}

impl std::error::Error for OsControlError {}

// Compile-time proof that the error type is thread-safe (design §18 asserts
// `Send + Sync` across the contract surface).
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<OsControlError>();
    assert_send_sync::<GrantInvalidReason>();
};

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    fn all_variants() -> Vec<OsControlError> {
        vec![
            OsControlError::Unsupported {
                capability: CapabilityId::new("set_volume"),
                reason: SafeText::new("no provider"),
            },
            OsControlError::Unavailable {
                provider: Some(ProviderId::new("pipewire")),
                reason: SafeText::new("bus down"),
                retryable: true,
            },
            OsControlError::InvalidRequest {
                field: SafeField::new("level"),
                reason: SafeText::new("out of range"),
            },
            OsControlError::AmbiguousTarget {
                kind: SafeText::new("sink"),
                candidates: BoundedVec::new(),
            },
            OsControlError::PermissionDenied {
                authority: SafeText::new("polkit"),
                remediation: SafeText::new("authenticate"),
            },
            OsControlError::PolicyDenied {
                reason: SafeText::new("blocked"),
            },
            OsControlError::ApprovalExpired,
            OsControlError::GrantInvalid {
                reason: GrantInvalidReason::BindingMismatch,
            },
            OsControlError::TargetChanged,
            OsControlError::ResourceBusy {
                resource: SafeResource::new("audio"),
                owner: None,
            },
            OsControlError::TimedOutBeforeMutation {
                operation: SafeOperation::new("observe"),
                timeout_ms: 500,
            },
            OsControlError::CancelledBeforeMutation,
            OsControlError::ProtocolBeforeMutation {
                provider: ProviderId::new("logind"),
                operation: SafeOperation::new("suspend"),
            },
            OsControlError::AuditUnavailable,
        ]
    }

    #[test]
    fn every_variant_has_a_closed_code() {
        let mut codes = std::collections::BTreeSet::new();
        for e in all_variants() {
            let code = e.code();
            assert!(code.starts_with("os_control."));
            assert!(codes.insert(code), "duplicate code {code}");
        }
        // 14 variants → 14 distinct codes.
        assert_eq!(codes.len(), 14);
    }

    #[test]
    fn envelope_has_frozen_shape_with_explicit_nulls() {
        let err = OsControlError::Unavailable {
            provider: None,
            reason: SafeText::new("bus down"),
            retryable: true,
        };
        let env = err.to_envelope();
        // Frozen top-level keys.
        let error = env.get("error").expect("error object");
        let os = env.get("os_control").expect("os_control object");
        for key in ["code", "message", "retryable", "remediation", "field"] {
            assert!(error.get(key).is_some(), "error.{key} must be present");
        }
        for key in ["provider", "lifecycle", "availability", "receipt_summary"] {
            assert!(os.get(key).is_some(), "os_control.{key} must be present");
        }
        // Absent values are explicit JSON null, never omitted.
        assert!(error.get("remediation").unwrap().is_null());
        assert!(error.get("field").unwrap().is_null());
        assert!(os.get("provider").unwrap().is_null());
        assert!(os.get("lifecycle").unwrap().is_null());
        assert!(os.get("receipt_summary").unwrap().is_null());
        assert_eq!(env["error"]["code"], "os_control.unavailable");
        assert_eq!(env["os_control"]["availability"], "unavailable");
    }

    #[test]
    fn envelope_never_leaks_control_chars() {
        // Even if a reason is built from raw provider text, SafeText redaction
        // guarantees no control characters reach the envelope.
        let err = OsControlError::PolicyDenied {
            reason: SafeText::new("denied\nby\trule\u{1b}[0m"),
        };
        let env = err.to_envelope();
        let msg = env["error"]["message"].as_str().unwrap();
        assert!(!msg.contains('\n'));
        assert!(!msg.contains('\t'));
        assert!(!msg.contains('\u{1b}'));
    }

    #[test]
    fn retryable_matches_semantics() {
        assert!(OsControlError::ResourceBusy {
            resource: SafeResource::new("net"),
            owner: None
        }
        .retryable());
        assert!(!OsControlError::ApprovalExpired.retryable());
        assert!(!OsControlError::GrantInvalid {
            reason: GrantInvalidReason::NonceReused
        }
        .retryable());
    }
}
