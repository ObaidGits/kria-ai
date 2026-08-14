//! Gate 1 & 2 of the retrieval pipeline (design §6.4, task F3.5.1).
//!
//! Gate 1: Authorize source/record BEFORE strategy candidate creation.
//! Gate 2: Reject Deleted/Forgotten and default-current Superseded;
//!         apply Stale/Unverified/Contradicted policy from caller allowlist.
//!
//! # Design invariants
//! * Policy gates are applied in the fixed order: auth → hard exclusions → policy.
//! * `Deleted`, `Forgotten` are ALWAYS excluded regardless of caller allowlist.
//! * `Superseded` is excluded by default (default-current behavior); it can be
//!   included ONLY if the caller explicitly lists `"superseded"` in their
//!   allowed_truth_states.
//! * `Stale`, `Unverified`, `Contradicted` follow caller allowed_truth_states:
//!   included when allowlist is empty (default) or when explicitly listed.
//! * Unauthorized candidates produce OPAQUE reason codes with no hidden IDs.

// ── Caller authorization ──────────────────────────────────────────────────────

/// Caller authorization context for the retrieval gate.
///
/// All fields are policy-safe (no hidden record IDs, counts, or topology).
#[derive(Debug, Clone)]
pub struct CallerAuthorization {
    /// Caller namespace.
    pub namespace: String,
    /// Caller scope.
    pub scope: String,
    /// Maximum sensitivity level the caller may see (0–3).
    pub max_sensitivity: i64,
    /// Allowed truth states. When empty, uses the conservative default allowlist
    /// that excludes Deleted/Forgotten/default-Superseded.
    pub allowed_truth_states: Vec<String>,
}

// ── Candidate metadata ────────────────────────────────────────────────────────

/// One candidate's authorization metadata (provided by the caller/strategy).
#[derive(Debug, Clone)]
pub struct CandidateMetadata {
    /// Candidate namespace.
    pub namespace: String,
    /// Candidate scope.
    pub scope: String,
    /// Candidate sensitivity.
    pub sensitivity: i64,
    /// Candidate truth state (e.g., "current", "deleted", "stale").
    pub truth_state: String,
}

// ── Gate disposition ──────────────────────────────────────────────────────────

/// The policy-safe disposition of one candidate after gate evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDisposition {
    /// Candidate passed all gates and may proceed to fusion.
    Pass,
    /// Candidate was excluded (authorization failure, hard exclusion, or policy).
    /// The reason_code is opaque — it MUST NOT encode hidden record IDs, counts,
    /// namespace tokens, or topology about invisible records.
    Excluded { reason_code: ReasonCode },
}

// ── Reason codes ──────────────────────────────────────────────────────────────

/// Opaque reason code for a gate exclusion.
///
/// Values must not reveal protected record IDs, counts, or topology.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasonCode {
    /// Candidate was excluded by authorization check (namespace/scope/sensitivity).
    Unauthorized,
    /// Candidate truth state is permanently excluded (Deleted or Forgotten).
    HardExcluded,
    /// Candidate truth state is `Superseded` and default-current behavior applies.
    DefaultSuperseded,
    /// Candidate truth state is not in the caller's allowed_truth_states allowlist.
    PolicyFiltered,
}

impl ReasonCode {
    /// Opaque string label safe to include in trace output.
    pub fn as_str(&self) -> &'static str {
        match self {
            ReasonCode::Unauthorized => "unauthorized",
            ReasonCode::HardExcluded => "hard_excluded",
            ReasonCode::DefaultSuperseded => "default_superseded",
            ReasonCode::PolicyFiltered => "policy_filtered",
        }
    }
}

// ── Builder helper ────────────────────────────────────────────────────────────

/// Build a `CallerAuthorization` from common policy fields.
pub fn caller_auth(
    namespace: impl Into<String>,
    scope: impl Into<String>,
    max_sensitivity: i64,
    allowed_truth_states: Vec<String>,
) -> CallerAuthorization {
    CallerAuthorization {
        namespace: namespace.into(),
        scope: scope.into(),
        max_sensitivity,
        allowed_truth_states,
    }
}

// ── Gate evaluation ───────────────────────────────────────────────────────────

/// Evaluate whether a candidate passes the authorization and truth-state gates.
///
/// Gate order (fixed):
/// 1. Authorization: namespace == caller.namespace, scope == caller.scope,
///    sensitivity <= caller.max_sensitivity → `GateDisposition::Excluded { Unauthorized }` if fails
/// 2. Hard exclusion: truth_state ∈ {"deleted", "forgotten"} → `HardExcluded`
/// 3. Default-Superseded: truth_state == "superseded" AND "superseded" NOT in caller.allowed_truth_states → `DefaultSuperseded`
/// 4. Policy allowlist: when caller.allowed_truth_states is non-empty and truth_state NOT in it → `PolicyFiltered`
/// 5. Otherwise: `GateDisposition::Pass`
pub fn evaluate_gate(auth: &CallerAuthorization, candidate: &CandidateMetadata) -> GateDisposition {
    // Gate 1: Authorization
    if candidate.namespace != auth.namespace
        || candidate.scope != auth.scope
        || candidate.sensitivity > auth.max_sensitivity
    {
        return GateDisposition::Excluded {
            reason_code: ReasonCode::Unauthorized,
        };
    }

    let ts_lower = candidate.truth_state.to_lowercase();

    // Gate 2: Hard exclusions — Deleted and Forgotten are always excluded
    if ts_lower == "deleted" || ts_lower == "forgotten" {
        return GateDisposition::Excluded {
            reason_code: ReasonCode::HardExcluded,
        };
    }

    // Gate 3: Default-Superseded — excluded unless caller explicitly allows it
    if ts_lower == "superseded" {
        let superseded_allowed = auth
            .allowed_truth_states
            .iter()
            .any(|s| s.to_lowercase() == "superseded");
        if !superseded_allowed {
            return GateDisposition::Excluded {
                reason_code: ReasonCode::DefaultSuperseded,
            };
        }
    }

    // Gate 4: Policy allowlist — only applied when allowlist is non-empty
    if !auth.allowed_truth_states.is_empty() {
        let in_allowlist = auth
            .allowed_truth_states
            .iter()
            .any(|s| s.to_lowercase() == ts_lower);
        if !in_allowlist {
            return GateDisposition::Excluded {
                reason_code: ReasonCode::PolicyFiltered,
            };
        }
    }

    GateDisposition::Pass
}

/// Evaluate the gate for a slice of candidates, returning one disposition per candidate.
pub fn evaluate_gates(
    auth: &CallerAuthorization,
    candidates: &[CandidateMetadata],
) -> Vec<GateDisposition> {
    candidates.iter().map(|c| evaluate_gate(auth, c)).collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_auth() -> CallerAuthorization {
        caller_auth("ns-a", "scope-x", 2, vec![])
    }

    fn make_candidate(
        namespace: &str,
        scope: &str,
        sensitivity: i64,
        truth_state: &str,
    ) -> CandidateMetadata {
        CandidateMetadata {
            namespace: namespace.to_string(),
            scope: scope.to_string(),
            sensitivity,
            truth_state: truth_state.to_string(),
        }
    }

    // 1. current, authorized → Pass
    #[test]
    fn pass_current_record_in_correct_namespace() {
        let auth = default_auth();
        let candidate = make_candidate("ns-a", "scope-x", 1, "current");
        assert_eq!(evaluate_gate(&auth, &candidate), GateDisposition::Pass);
    }

    // 2. wrong namespace → Unauthorized
    #[test]
    fn fail_wrong_namespace() {
        let auth = default_auth();
        let candidate = make_candidate("ns-b", "scope-x", 1, "current");
        assert_eq!(
            evaluate_gate(&auth, &candidate),
            GateDisposition::Excluded {
                reason_code: ReasonCode::Unauthorized
            }
        );
    }

    // 3. wrong scope → Unauthorized
    #[test]
    fn fail_wrong_scope() {
        let auth = default_auth();
        let candidate = make_candidate("ns-a", "scope-y", 1, "current");
        assert_eq!(
            evaluate_gate(&auth, &candidate),
            GateDisposition::Excluded {
                reason_code: ReasonCode::Unauthorized
            }
        );
    }

    // 4. sensitivity > max → Unauthorized
    #[test]
    fn fail_sensitivity_too_high() {
        let auth = default_auth(); // max_sensitivity = 2
        let candidate = make_candidate("ns-a", "scope-x", 3, "current");
        assert_eq!(
            evaluate_gate(&auth, &candidate),
            GateDisposition::Excluded {
                reason_code: ReasonCode::Unauthorized
            }
        );
    }

    // 5. truth_state="deleted" → HardExcluded
    #[test]
    fn deleted_always_excluded() {
        let auth = default_auth();
        let candidate = make_candidate("ns-a", "scope-x", 0, "deleted");
        assert_eq!(
            evaluate_gate(&auth, &candidate),
            GateDisposition::Excluded {
                reason_code: ReasonCode::HardExcluded
            }
        );
    }

    // 6. truth_state="forgotten" → HardExcluded
    #[test]
    fn forgotten_always_excluded() {
        let auth = default_auth();
        let candidate = make_candidate("ns-a", "scope-x", 0, "forgotten");
        assert_eq!(
            evaluate_gate(&auth, &candidate),
            GateDisposition::Excluded {
                reason_code: ReasonCode::HardExcluded
            }
        );
    }

    // 7. allowed_truth_states=["deleted"] → still HardExcluded (hard exclusion wins)
    #[test]
    fn deleted_excluded_even_if_in_allowlist() {
        let auth = caller_auth("ns-a", "scope-x", 2, vec!["deleted".to_string()]);
        let candidate = make_candidate("ns-a", "scope-x", 0, "deleted");
        assert_eq!(
            evaluate_gate(&auth, &candidate),
            GateDisposition::Excluded {
                reason_code: ReasonCode::HardExcluded
            }
        );
    }

    // 8. truth_state="superseded", allowlist empty → DefaultSuperseded
    #[test]
    fn superseded_excluded_by_default() {
        let auth = default_auth();
        let candidate = make_candidate("ns-a", "scope-x", 0, "superseded");
        assert_eq!(
            evaluate_gate(&auth, &candidate),
            GateDisposition::Excluded {
                reason_code: ReasonCode::DefaultSuperseded
            }
        );
    }

    // 9. allowed_truth_states=["superseded", "current"] → Pass
    #[test]
    fn superseded_included_when_explicitly_allowed() {
        let auth = caller_auth(
            "ns-a",
            "scope-x",
            2,
            vec!["superseded".to_string(), "current".to_string()],
        );
        let candidate = make_candidate("ns-a", "scope-x", 0, "superseded");
        assert_eq!(evaluate_gate(&auth, &candidate), GateDisposition::Pass);
    }

    // 10. stale, empty allowlist → Pass
    #[test]
    fn stale_included_by_default_empty_allowlist() {
        let auth = default_auth();
        let candidate = make_candidate("ns-a", "scope-x", 0, "stale");
        assert_eq!(evaluate_gate(&auth, &candidate), GateDisposition::Pass);
    }

    // 11. stale, allowed=["current"] → PolicyFiltered
    #[test]
    fn stale_excluded_when_allowlist_restricts() {
        let auth = caller_auth("ns-a", "scope-x", 2, vec!["current".to_string()]);
        let candidate = make_candidate("ns-a", "scope-x", 0, "stale");
        assert_eq!(
            evaluate_gate(&auth, &candidate),
            GateDisposition::Excluded {
                reason_code: ReasonCode::PolicyFiltered
            }
        );
    }

    // 12. unverified, empty allowlist → Pass
    #[test]
    fn unverified_included_by_default() {
        let auth = default_auth();
        let candidate = make_candidate("ns-a", "scope-x", 0, "unverified");
        assert_eq!(evaluate_gate(&auth, &candidate), GateDisposition::Pass);
    }

    // 13. contradicted, empty allowlist → Pass
    #[test]
    fn contradicted_included_by_default() {
        let auth = default_auth();
        let candidate = make_candidate("ns-a", "scope-x", 0, "contradicted");
        assert_eq!(evaluate_gate(&auth, &candidate), GateDisposition::Pass);
    }

    // 14. contradicted, allowed=["current"] → PolicyFiltered
    #[test]
    fn contradicted_excluded_when_not_in_allowlist() {
        let auth = caller_auth("ns-a", "scope-x", 2, vec!["current".to_string()]);
        let candidate = make_candidate("ns-a", "scope-x", 0, "contradicted");
        assert_eq!(
            evaluate_gate(&auth, &candidate),
            GateDisposition::Excluded {
                reason_code: ReasonCode::PolicyFiltered
            }
        );
    }

    // 15. batch: 3 candidates, verify 3 results
    #[test]
    fn evaluate_gates_batch_returns_one_per_candidate() {
        let auth = default_auth();
        let candidates = vec![
            make_candidate("ns-a", "scope-x", 0, "current"),
            make_candidate("ns-b", "scope-x", 0, "current"), // wrong namespace
            make_candidate("ns-a", "scope-x", 0, "deleted"),
        ];
        let results = evaluate_gates(&auth, &candidates);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0], GateDisposition::Pass);
        assert_eq!(
            results[1],
            GateDisposition::Excluded {
                reason_code: ReasonCode::Unauthorized
            }
        );
        assert_eq!(
            results[2],
            GateDisposition::Excluded {
                reason_code: ReasonCode::HardExcluded
            }
        );
    }

    // 16. all reason code strings are non-empty
    #[test]
    fn reason_codes_are_opaque_strings() {
        assert!(!ReasonCode::Unauthorized.as_str().is_empty());
        assert!(!ReasonCode::HardExcluded.as_str().is_empty());
        assert!(!ReasonCode::DefaultSuperseded.as_str().is_empty());
        assert!(!ReasonCode::PolicyFiltered.as_str().is_empty());
    }

    // 17. auth is checked before hard exclusion — wrong namespace with deleted state → Unauthorized
    #[test]
    fn gate_order_is_auth_first() {
        let auth = default_auth();
        let candidate = make_candidate("ns-WRONG", "scope-x", 0, "deleted");
        // Auth gate fires first → Unauthorized, NOT HardExcluded
        assert_eq!(
            evaluate_gate(&auth, &candidate),
            GateDisposition::Excluded {
                reason_code: ReasonCode::Unauthorized
            }
        );
    }
}
