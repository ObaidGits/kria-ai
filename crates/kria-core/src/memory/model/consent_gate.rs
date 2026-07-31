//! Consent-gated source ingestion types (design §5.4, task F2.6.2 / MGR-046).
//!
//! This module enforces the invariant that **no scan, filesystem access,
//! repository access, MCP call, or shell-history read may occur until the user
//! has explicitly granted consent** for the source.
//!
//! ## Key behavioral rules (MGR-046)
//!
//! 1. **No consent = no scan**: [`ConsentGate::check_before_discovery`] blocks
//!    any discovery when the source's consent state is not
//!    [`ConsentState::Approved`].
//! 2. **Approve** → [`ConsentState::Approved`],
//!    [`SourceLifecycleState::Registered`], `should_start_ingestion = true`.
//! 3. **Exclude** → [`ConsentState::Excluded`],
//!    [`SourceLifecycleState::Registered`], `should_start_ingestion = false`.
//! 4. **ManualOnboarding** → [`ConsentState::Pending`] (user will configure),
//!    [`SourceLifecycleState::Paused`], `should_start_ingestion = false`.
//! 5. **Preview before consent**: [`ConsentGate::build_request`] constructs a
//!    [`ConsentRequest`] showing what *will* happen **without** reading any
//!    source content.

use serde::{Deserialize, Serialize};

use super::source_state::{ConsentState, SourceKind, SourceLifecycleState, SourceTrustClass};

// ── DiscoveryCandidate ─────────────────────────────────────────────────────

/// A source candidate discovered before consent has been requested.
///
/// No data is read, scanned, or written until the user approves or explicitly
/// sets up manual onboarding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryCandidate {
    /// A stable candidate identity (opaque, not an authority ID yet).
    pub candidate_id: String,
    /// The kind of source.
    pub source_kind: SourceKind,
    /// The external identity (e.g. path, URL, MCP server name).
    pub external_identity: String,
    /// Optional version string.
    pub version: Option<String>,
    /// The trust classification of this candidate.
    pub trust_class: SourceTrustClass,
    /// A human-readable description of what this source contains.
    pub description: Option<String>,
    /// Estimated item count (for preview — not a guarantee).
    pub estimated_item_count: Option<u64>,
    /// When the candidate was discovered (RFC 3339 UTC text).
    pub discovered_at: String,
}

// ── ConsentDecision ────────────────────────────────────────────────────────

/// The user's consent decision for a source candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsentDecision {
    /// User approves ingestion of this source.
    Approve,
    /// User excludes this source (no further prompting).
    Exclude,
    /// User wants to configure ingestion manually (e.g. choose specific items).
    ManualOnboarding,
}

impl ConsentDecision {
    /// The resulting [`ConsentState`] for this decision.
    ///
    /// | Decision          | ConsentState  |
    /// |-------------------|---------------|
    /// | `Approve`         | `Approved`    |
    /// | `Exclude`         | `Excluded`    |
    /// | `ManualOnboarding`| `Pending`     |
    pub fn resulting_consent_state(self) -> ConsentState {
        match self {
            ConsentDecision::Approve => ConsentState::Approved,
            ConsentDecision::Exclude => ConsentState::Excluded,
            // ManualOnboarding leaves the user in control; consent reverts to
            // Pending while the user configures the exact scope to ingest.
            ConsentDecision::ManualOnboarding => ConsentState::Pending,
        }
    }

    /// Whether this decision starts automatic ingestion.
    ///
    /// Only `Approve` starts ingestion automatically; `Exclude` and
    /// `ManualOnboarding` do not.
    pub fn starts_automatic_ingestion(self) -> bool {
        matches!(self, ConsentDecision::Approve)
    }
}

// ── ConsentRequest ─────────────────────────────────────────────────────────

/// The preview shown to the user before they grant consent for a source.
///
/// This is constructed by [`ConsentGate::build_request`] **without reading any
/// source content**; all fields are metadata derived from the discovered
/// candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsentRequest {
    /// The candidate being previewed.
    pub candidate: DiscoveryCandidate,
    /// A list of what will be read/ingested if approved.
    pub preview_items: Vec<String>,
    /// Any warnings (e.g. sensitive content, large volume).
    pub warnings: Vec<String>,
    /// Whether manual onboarding is available for this source kind.
    pub supports_manual_onboarding: bool,
    /// The policy namespace that would be applied to ingested content.
    pub proposed_policy_namespace: String,
    /// The policy scope that would be applied to ingested content.
    pub proposed_policy_scope: String,
    /// The sensitivity level that would be applied (`0..=3`).
    pub proposed_policy_sensitivity: u8,
}

// ── ConsentOutcome ─────────────────────────────────────────────────────────

/// The result of processing a consent decision for a source candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsentOutcome {
    /// The candidate that was decided on.
    pub candidate_id: String,
    /// The decision made.
    pub decision: ConsentDecision,
    /// The resulting consent state.
    pub consent_state: ConsentState,
    /// Whether ingestion should start automatically.
    pub should_start_ingestion: bool,
    /// The lifecycle state to set on the source record.
    pub resulting_lifecycle_state: SourceLifecycleState,
}

// ── ConsentGateError ───────────────────────────────────────────────────────

/// Errors produced by [`ConsentGate::check_before_discovery`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsentGateError {
    /// Consent has not been granted (`Pending`, `Excluded`, or `Revoked`).
    ConsentNotGranted {
        /// The consent state that blocked discovery.
        state: ConsentState,
    },
}

impl std::fmt::Display for ConsentGateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConsentGateError::ConsentNotGranted { state } => {
                write!(
                    f,
                    "discovery blocked: consent state is {state} \
                     (explicit Approved consent is required before any scan)"
                )
            }
        }
    }
}

impl std::error::Error for ConsentGateError {}

// ── ConsentGate ────────────────────────────────────────────────────────────

/// Evaluates consent before any source discovery or ingestion takes place.
///
/// This is a **stateless** gate — all methods are pure functions over their
/// arguments. There is nothing to construct; call the associated functions
/// directly.
pub struct ConsentGate;

impl ConsentGate {
    /// Check whether a source may be discovered or scanned.
    ///
    /// Returns `Ok(())` when the source's consent state permits ingestion (i.e.
    /// the state is [`ConsentState::Approved`]). Returns
    /// [`ConsentGateError::ConsentNotGranted`] for every other consent state.
    ///
    /// **This MUST be called before any filesystem scan, repository access,
    /// MCP call, or shell history read.** Only `Approved` passes; `Pending`,
    /// `Excluded`, and `Revoked` are all blocked.
    pub fn check_before_discovery(consent: ConsentState) -> Result<(), ConsentGateError> {
        if consent.permits_ingestion() {
            Ok(())
        } else {
            Err(ConsentGateError::ConsentNotGranted { state: consent })
        }
    }

    /// Process a consent decision for a candidate and produce a
    /// [`ConsentOutcome`].
    ///
    /// Rules applied (MGR-046):
    ///
    /// | Decision           | ConsentState | LifecycleState | starts ingestion |
    /// |--------------------|--------------|----------------|-----------------|
    /// | `Approve`          | `Approved`   | `Registered`   | `true`          |
    /// | `Exclude`          | `Excluded`   | `Registered`   | `false`         |
    /// | `ManualOnboarding` | `Pending`    | `Paused`       | `false`         |
    pub fn process_decision(candidate_id: String, decision: ConsentDecision) -> ConsentOutcome {
        let consent_state = decision.resulting_consent_state();
        let should_start_ingestion = decision.starts_automatic_ingestion();

        // Lifecycle rule:
        //  - Approve / Exclude → Registered (source is known, awaits or skips
        //    ingestion start).
        //  - ManualOnboarding → Paused (ingestion is deferred while the user
        //    configures the exact scope).
        let resulting_lifecycle_state = match decision {
            ConsentDecision::Approve | ConsentDecision::Exclude => SourceLifecycleState::Registered,
            ConsentDecision::ManualOnboarding => SourceLifecycleState::Paused,
        };

        ConsentOutcome {
            candidate_id,
            decision,
            consent_state,
            should_start_ingestion,
            resulting_lifecycle_state,
        }
    }

    /// Build a [`ConsentRequest`] preview for a candidate.
    ///
    /// This constructs the preview **without reading any source content**; all
    /// information is derived from the metadata already present in the
    /// [`DiscoveryCandidate`].  The caller supplies the proposed policy
    /// parameters; the gate does not query any store.
    ///
    /// `supports_manual_onboarding` is determined by the source kind: all
    /// source kinds support manual onboarding.
    pub fn build_request(
        candidate: DiscoveryCandidate,
        proposed_namespace: &str,
        proposed_scope: &str,
        proposed_sensitivity: u8,
    ) -> ConsentRequest {
        let supports_manual_onboarding =
            Self::kind_supports_manual_onboarding(&candidate.source_kind);

        // Build a human-readable preview without touching the actual source.
        let preview_items = Self::preview_items_for(&candidate);
        let warnings = Self::warnings_for(&candidate, proposed_sensitivity);

        ConsentRequest {
            candidate,
            preview_items,
            warnings,
            supports_manual_onboarding,
            proposed_policy_namespace: proposed_namespace.to_owned(),
            proposed_policy_scope: proposed_scope.to_owned(),
            proposed_policy_sensitivity: proposed_sensitivity,
        }
    }

    // ── Internal helpers ────────────────────────────────────────────────

    /// All source kinds support manual onboarding (so the user can always
    /// choose to configure an exact scope before ingestion begins).
    fn kind_supports_manual_onboarding(_kind: &SourceKind) -> bool {
        true
    }

    /// Build a preview item list from candidate metadata only (no I/O).
    fn preview_items_for(candidate: &DiscoveryCandidate) -> Vec<String> {
        let mut items = Vec::new();

        items.push(format!("Source kind: {}", candidate.source_kind.as_str()));
        items.push(format!(
            "External identity: {}",
            candidate.external_identity
        ));
        if let Some(version) = &candidate.version {
            items.push(format!("Version: {version}"));
        }
        if let Some(description) = &candidate.description {
            items.push(format!("Description: {description}"));
        }
        if let Some(count) = candidate.estimated_item_count {
            items.push(format!("Estimated items: {count}"));
        }
        items.push(format!(
            "Trust classification: {}",
            candidate.trust_class.as_str()
        ));

        items
    }

    /// Build a warning list from candidate metadata.
    ///
    /// Warnings are added for:
    /// - High sensitivity (≥ 2).
    /// - Large estimated item counts (≥ 10 000).
    /// - Unknown trust class.
    fn warnings_for(candidate: &DiscoveryCandidate, proposed_sensitivity: u8) -> Vec<String> {
        let mut warnings = Vec::new();

        if proposed_sensitivity >= 2 {
            warnings.push(format!(
                "High sensitivity content (level {proposed_sensitivity}): \
                 ingested items will be restricted accordingly."
            ));
        }
        if let Some(count) = candidate.estimated_item_count {
            if count >= 10_000 {
                warnings.push(format!(
                    "Large source: estimated {count} items may take significant time to ingest."
                ));
            }
        }
        if candidate.trust_class == SourceTrustClass::Unknown {
            warnings.push("Unknown trust classification: proceed with caution.".to_owned());
        }

        warnings
    }
}

// ── Unit tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ──────────────────────────────────────────────────────────

    fn make_candidate(kind: SourceKind, estimated_items: Option<u64>) -> DiscoveryCandidate {
        DiscoveryCandidate {
            candidate_id: "cand-001".to_owned(),
            source_kind: kind,
            external_identity: "/home/user/documents".to_owned(),
            version: None,
            trust_class: SourceTrustClass::External,
            description: Some("Test candidate".to_owned()),
            estimated_item_count: estimated_items,
            discovered_at: "2024-01-15T12:00:00+00:00".to_owned(),
        }
    }

    // ── ConsentGate::check_before_discovery ──────────────────────────────

    #[test]
    fn check_before_discovery_ok_when_approved() {
        assert!(ConsentGate::check_before_discovery(ConsentState::Approved).is_ok());
    }

    #[test]
    fn check_before_discovery_err_when_pending() {
        let err = ConsentGate::check_before_discovery(ConsentState::Pending).unwrap_err();
        assert!(
            matches!(
                err,
                ConsentGateError::ConsentNotGranted {
                    state: ConsentState::Pending
                }
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn check_before_discovery_err_when_excluded() {
        let err = ConsentGate::check_before_discovery(ConsentState::Excluded).unwrap_err();
        assert!(matches!(
            err,
            ConsentGateError::ConsentNotGranted {
                state: ConsentState::Excluded
            }
        ));
    }

    #[test]
    fn check_before_discovery_err_when_revoked() {
        let err = ConsentGate::check_before_discovery(ConsentState::Revoked).unwrap_err();
        assert!(matches!(
            err,
            ConsentGateError::ConsentNotGranted {
                state: ConsentState::Revoked
            }
        ));
    }

    // ── ConsentGate::process_decision — Approve ──────────────────────────

    #[test]
    fn process_decision_approve_produces_correct_outcome() {
        let outcome =
            ConsentGate::process_decision("cand-001".to_owned(), ConsentDecision::Approve);

        assert_eq!(outcome.candidate_id, "cand-001");
        assert_eq!(outcome.decision, ConsentDecision::Approve);
        assert_eq!(outcome.consent_state, ConsentState::Approved);
        assert!(outcome.should_start_ingestion);
        assert_eq!(
            outcome.resulting_lifecycle_state,
            SourceLifecycleState::Registered
        );
    }

    // ── ConsentGate::process_decision — Exclude ──────────────────────────

    #[test]
    fn process_decision_exclude_produces_no_ingestion() {
        let outcome =
            ConsentGate::process_decision("cand-002".to_owned(), ConsentDecision::Exclude);

        assert_eq!(outcome.consent_state, ConsentState::Excluded);
        assert!(!outcome.should_start_ingestion);
        assert_eq!(
            outcome.resulting_lifecycle_state,
            SourceLifecycleState::Registered
        );
    }

    // ── ConsentGate::process_decision — ManualOnboarding ─────────────────

    #[test]
    fn process_decision_manual_onboarding_pending_and_paused() {
        let outcome =
            ConsentGate::process_decision("cand-003".to_owned(), ConsentDecision::ManualOnboarding);

        assert_eq!(outcome.consent_state, ConsentState::Pending);
        assert!(!outcome.should_start_ingestion);
        assert_eq!(
            outcome.resulting_lifecycle_state,
            SourceLifecycleState::Paused
        );
    }

    // ── ConsentDecision::resulting_consent_state ─────────────────────────

    #[test]
    fn consent_decision_resulting_state_approve() {
        assert_eq!(
            ConsentDecision::Approve.resulting_consent_state(),
            ConsentState::Approved
        );
    }

    #[test]
    fn consent_decision_resulting_state_exclude() {
        assert_eq!(
            ConsentDecision::Exclude.resulting_consent_state(),
            ConsentState::Excluded
        );
    }

    #[test]
    fn consent_decision_resulting_state_manual_onboarding() {
        assert_eq!(
            ConsentDecision::ManualOnboarding.resulting_consent_state(),
            ConsentState::Pending
        );
    }

    // ── ConsentDecision::starts_automatic_ingestion ──────────────────────

    #[test]
    fn starts_automatic_ingestion_only_for_approve() {
        assert!(ConsentDecision::Approve.starts_automatic_ingestion());
        assert!(!ConsentDecision::Exclude.starts_automatic_ingestion());
        assert!(!ConsentDecision::ManualOnboarding.starts_automatic_ingestion());
    }

    // ── ConsentGate::build_request ───────────────────────────────────────

    #[test]
    fn build_request_includes_candidate_and_policy() {
        let candidate = make_candidate(SourceKind::Filesystem, Some(500));
        let req = ConsentGate::build_request(candidate, "user", "personal", 1);

        assert_eq!(req.proposed_policy_namespace, "user");
        assert_eq!(req.proposed_policy_scope, "personal");
        assert_eq!(req.proposed_policy_sensitivity, 1);
        assert!(req.supports_manual_onboarding);
        // Preview items must be non-empty.
        assert!(!req.preview_items.is_empty());
        // No warnings for low sensitivity and small item count.
        assert!(
            req.warnings.is_empty(),
            "unexpected warnings: {:?}",
            req.warnings
        );
    }

    #[test]
    fn build_request_high_sensitivity_adds_warning() {
        let candidate = make_candidate(SourceKind::Repository, None);
        let req = ConsentGate::build_request(candidate, "user", "work", 3);

        assert!(
            req.warnings.iter().any(|w| w.contains("High sensitivity")),
            "expected high-sensitivity warning, got: {:?}",
            req.warnings
        );
    }

    #[test]
    fn build_request_large_source_adds_warning() {
        let candidate = make_candidate(SourceKind::Library, Some(50_000));
        let req = ConsentGate::build_request(candidate, "user", "docs", 0);

        assert!(
            req.warnings.iter().any(|w| w.contains("Large source")),
            "expected large-source warning, got: {:?}",
            req.warnings
        );
    }

    #[test]
    fn build_request_unknown_trust_adds_warning() {
        let mut candidate = make_candidate(SourceKind::Mcp, None);
        candidate.trust_class = SourceTrustClass::Unknown;
        let req = ConsentGate::build_request(candidate, "user", "external", 0);

        assert!(
            req.warnings.iter().any(|w| w.contains("Unknown trust")),
            "expected unknown-trust warning, got: {:?}",
            req.warnings
        );
    }

    #[test]
    fn build_request_all_source_kinds_support_manual_onboarding() {
        let kinds = [
            SourceKind::Native,
            SourceKind::Mcp,
            SourceKind::OpenClaw,
            SourceKind::Sidecar,
            SourceKind::Import,
            SourceKind::Library,
            SourceKind::Conversation,
            SourceKind::Filesystem,
            SourceKind::Repository,
            SourceKind::ShellHistory,
        ];
        for kind in kinds {
            let candidate = make_candidate(kind, None);
            let req = ConsentGate::build_request(candidate, "ns", "sc", 0);
            assert!(
                req.supports_manual_onboarding,
                "expected manual onboarding support for {:?}",
                req.candidate.source_kind
            );
        }
    }

    // ── ConsentGateError display ─────────────────────────────────────────

    #[test]
    fn consent_gate_error_display_is_informative() {
        let err = ConsentGateError::ConsentNotGranted {
            state: ConsentState::Revoked,
        };
        let msg = err.to_string();
        assert!(msg.contains("revoked"), "display missing state: {msg}");
        assert!(msg.contains("Approved"), "display missing hint: {msg}");
    }

    // ── Serde round-trips ────────────────────────────────────────────────

    #[test]
    fn consent_decision_serde_roundtrip() {
        for decision in [
            ConsentDecision::Approve,
            ConsentDecision::Exclude,
            ConsentDecision::ManualOnboarding,
        ] {
            let json = serde_json::to_string(&decision).unwrap();
            let back: ConsentDecision = serde_json::from_str(&json).unwrap();
            assert_eq!(back, decision, "serde roundtrip failed for {decision:?}");
        }
    }

    #[test]
    fn discovery_candidate_serde_roundtrip() {
        let candidate = make_candidate(SourceKind::Filesystem, Some(42));
        let json = serde_json::to_string(&candidate).unwrap();
        let back: DiscoveryCandidate = serde_json::from_str(&json).unwrap();
        assert_eq!(back.candidate_id, candidate.candidate_id);
        assert_eq!(back.estimated_item_count, Some(42));
    }

    #[test]
    fn consent_outcome_serde_roundtrip() {
        let outcome = ConsentGate::process_decision("cand-x".to_owned(), ConsentDecision::Approve);
        let json = serde_json::to_string(&outcome).unwrap();
        let back: ConsentOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(back.candidate_id, "cand-x");
        assert_eq!(back.consent_state, ConsentState::Approved);
        assert!(back.should_start_ingestion);
    }
}
