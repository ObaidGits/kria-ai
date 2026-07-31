//! Source lifecycle state types for the `sources` authority table (design §4.3,
//! task F2.6.1 / MGR-046).
//!
//! This module defines the typed state enums and validators for the
//! consent-gated source ingestion lifecycle.  Every type here maps directly to
//! a column in `sources` and replaces the raw `Option<String>` placeholders in
//! [`crate::memory::model::SourceRecord`].
//!
//! ## Key behavioral rules (MGR-046)
//! 1. [`ConsentState::Approved`] is the only consent state that permits
//!    ingestion — `Pending`, `Excluded`, and `Revoked` all block it.
//! 2. [`SourceLifecycleState::Registered`] and [`SourceLifecycleState::Paused`]
//!    are the only lifecycle states from which ingestion may start.
//! 3. [`SourceLifecycleState::Deleted`] and [`SourceLifecycleState::Purged`] are
//!    terminal — no further transitions are possible.
//! 4. [`SourceCursor`] tracks resumable progress; [`SourceCursor::advance`]
//!    creates a new cursor snapshot while preserving the previous one.

use serde::{Deserialize, Serialize};

// ── SourceKind ─────────────────────────────────────────────────────────────

/// The kind of source, extending the authority `CHECK` set with three
/// additional source kinds needed for consent-gated ingestion (design §4.3,
/// task F2.6.1).
///
/// The authority boundary `CHECK` in the SQL schema covers
/// `native/mcp/openclaw/sidecar/import/library/conversation`; the three new
/// variants (`filesystem`, `repository`, `shell_history`) are added here and
/// must be included in the next migration's `CHECK` expansion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// In-process native tool / core subsystem.
    Native,
    /// A Model Context Protocol server.
    Mcp,
    /// A sandboxed OpenClaw skill.
    OpenClaw,
    /// A local sidecar process.
    Sidecar,
    /// An interchange / bulk import.
    Import,
    /// A library / document corpus ingestion.
    Library,
    /// A conversation turn.
    Conversation,
    /// A local filesystem path scan.
    Filesystem,
    /// A version-control repository scan.
    Repository,
    /// A shell history ingestion.
    ShellHistory,
}

impl SourceKind {
    /// The canonical text form stored in `source_kind` columns.
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceKind::Native => "native",
            SourceKind::Mcp => "mcp",
            SourceKind::OpenClaw => "openclaw",
            SourceKind::Sidecar => "sidecar",
            SourceKind::Import => "import",
            SourceKind::Library => "library",
            SourceKind::Conversation => "conversation",
            SourceKind::Filesystem => "filesystem",
            SourceKind::Repository => "repository",
            SourceKind::ShellHistory => "shell_history",
        }
    }
}

impl std::str::FromStr for SourceKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "native" => SourceKind::Native,
            "mcp" => SourceKind::Mcp,
            "openclaw" => SourceKind::OpenClaw,
            "sidecar" => SourceKind::Sidecar,
            "import" => SourceKind::Import,
            "library" => SourceKind::Library,
            "conversation" => SourceKind::Conversation,
            "filesystem" => SourceKind::Filesystem,
            "repository" => SourceKind::Repository,
            "shell_history" => SourceKind::ShellHistory,
            other => return Err(format!("unknown source_kind {other:?}")),
        })
    }
}

impl std::fmt::Display for SourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── SourceTrustClass ───────────────────────────────────────────────────────

/// The trust classification of a source (design §4.3 `sources.trust_class`).
///
/// Ordered from most trusted to least trusted (`Native > Verified > ThirdParty
/// > External > Unknown`) so the effective trust for a composed source is
/// `min(contributors)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceTrustClass {
    /// Highest trust: KRIA's own built-in native tools.
    Native,
    /// Verified MCP server registered with KRIA.
    Verified,
    /// Third-party OpenClaw skill or sidecar.
    ThirdParty,
    /// External import or library source.
    External,
    /// Unknown / unclassified trust.
    Unknown,
}

impl SourceTrustClass {
    /// The canonical text form stored in `trust_class` columns.
    pub fn as_str(self) -> &'static str {
        match self {
            SourceTrustClass::Native => "native",
            SourceTrustClass::Verified => "verified",
            SourceTrustClass::ThirdParty => "third_party",
            SourceTrustClass::External => "external",
            SourceTrustClass::Unknown => "unknown",
        }
    }
}

impl std::str::FromStr for SourceTrustClass {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "native" => SourceTrustClass::Native,
            "verified" => SourceTrustClass::Verified,
            "third_party" => SourceTrustClass::ThirdParty,
            "external" => SourceTrustClass::External,
            "unknown" => SourceTrustClass::Unknown,
            other => return Err(format!("unknown trust_class {other:?}")),
        })
    }
}

impl std::fmt::Display for SourceTrustClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── ConsentState ───────────────────────────────────────────────────────────

/// The consent state for a source (design §4.3 `sources.consent_state`).
///
/// MGR-046: explicit consent must be obtained before any scan or ingestion.
/// Only [`ConsentState::Approved`] permits ingestion; all other states block it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsentState {
    /// Consent has not been requested yet.
    Pending,
    /// User approved ingestion of this source.
    Approved,
    /// User explicitly excluded this source.
    Excluded,
    /// Consent was revoked after initial approval.
    Revoked,
}

impl ConsentState {
    /// Whether ingestion is permitted in this consent state.
    ///
    /// Only [`ConsentState::Approved`] returns `true`; `Pending`, `Excluded`,
    /// and `Revoked` all return `false` (MGR-046).
    pub fn permits_ingestion(self) -> bool {
        matches!(self, ConsentState::Approved)
    }

    /// The canonical text form stored in `consent_state` columns.
    pub fn as_str(self) -> &'static str {
        match self {
            ConsentState::Pending => "pending",
            ConsentState::Approved => "approved",
            ConsentState::Excluded => "excluded",
            ConsentState::Revoked => "revoked",
        }
    }
}

impl std::str::FromStr for ConsentState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "pending" => ConsentState::Pending,
            "approved" => ConsentState::Approved,
            "excluded" => ConsentState::Excluded,
            "revoked" => ConsentState::Revoked,
            other => return Err(format!("unknown consent_state {other:?}")),
        })
    }
}

impl std::fmt::Display for ConsentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── SourceLifecycleState ───────────────────────────────────────────────────

/// The lifecycle state of a source (design §4.3 `sources.lifecycle_state`).
///
/// Valid ingestion-start states are [`SourceLifecycleState::Registered`] and
/// [`SourceLifecycleState::Paused`].  [`SourceLifecycleState::Deleted`] and
/// [`SourceLifecycleState::Purged`] are terminal and block all transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceLifecycleState {
    /// The source has been registered but not yet scanned.
    Registered,
    /// The source is actively being scanned / ingested.
    Ingesting,
    /// Ingestion is paused (e.g. resource pressure or explicit pause).
    Paused,
    /// Ingestion completed successfully.
    Completed,
    /// Ingestion failed.
    Failed,
    /// The source has been deleted (deletion lifecycle started).
    Deleted,
    /// The source's ingested content has been fully purged.
    Purged,
}

impl SourceLifecycleState {
    /// Whether ingestion may be started (or resumed) from this state.
    ///
    /// Only [`SourceLifecycleState::Registered`] and
    /// [`SourceLifecycleState::Paused`] return `true`.
    pub fn can_ingest(self) -> bool {
        matches!(
            self,
            SourceLifecycleState::Registered | SourceLifecycleState::Paused
        )
    }

    /// Whether this is a terminal state (no further transitions possible).
    ///
    /// [`SourceLifecycleState::Deleted`] and [`SourceLifecycleState::Purged`]
    /// are terminal.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            SourceLifecycleState::Deleted | SourceLifecycleState::Purged
        )
    }

    /// The canonical text form stored in `lifecycle_state` columns.
    pub fn as_str(self) -> &'static str {
        match self {
            SourceLifecycleState::Registered => "registered",
            SourceLifecycleState::Ingesting => "ingesting",
            SourceLifecycleState::Paused => "paused",
            SourceLifecycleState::Completed => "completed",
            SourceLifecycleState::Failed => "failed",
            SourceLifecycleState::Deleted => "deleted",
            SourceLifecycleState::Purged => "purged",
        }
    }
}

impl std::str::FromStr for SourceLifecycleState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "registered" => SourceLifecycleState::Registered,
            "ingesting" => SourceLifecycleState::Ingesting,
            "paused" => SourceLifecycleState::Paused,
            "completed" => SourceLifecycleState::Completed,
            "failed" => SourceLifecycleState::Failed,
            "deleted" => SourceLifecycleState::Deleted,
            "purged" => SourceLifecycleState::Purged,
            other => return Err(format!("unknown lifecycle_state {other:?}")),
        })
    }
}

impl std::fmt::Display for SourceLifecycleState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── SourceCursor ───────────────────────────────────────────────────────────

/// The ingestion progress cursor for a source (design §4.3
/// `sources.cursor_json`).
///
/// Allows resuming interrupted ingestion from the last checkpoint.  The cursor
/// is opaque to the authority store (stored as JSON in `cursor_json`) and
/// interpreted only by the ingestion worker.
///
/// [`SourceCursor::advance`] creates a **new** cursor snapshot from the current
/// one; the previous cursor is preserved by the caller for history / rollback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceCursor {
    /// The item offset or checkpoint identifier (opaque to the store).
    pub position: String,
    /// The content hash of the last successfully processed item, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_processed_hash: Option<String>,
    /// RFC 3339 UTC timestamp of when the cursor was last updated.
    pub updated_at: String,
    /// Total items estimated (`None` if unknown).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_items_estimate: Option<u64>,
    /// Items processed so far.
    pub items_processed: u64,
}

impl SourceCursor {
    /// Create a new cursor at the start of ingestion.
    ///
    /// `position` is the initial checkpoint identifier (e.g. `"0"` for an
    /// offset-based source or a path for a filesystem source).
    pub fn start(position: impl Into<String>) -> Self {
        Self {
            position: position.into(),
            last_processed_hash: None,
            updated_at: chrono::Utc::now().to_rfc3339(),
            total_items_estimate: None,
            items_processed: 0,
        }
    }

    /// Advance the cursor to the next position, recording the content hash of
    /// the item just processed.
    ///
    /// Returns a **new** cursor; the receiver is unchanged so the caller can
    /// keep the previous snapshot if needed.
    pub fn advance(&self, next_position: impl Into<String>, content_hash: Option<String>) -> Self {
        Self {
            position: next_position.into(),
            last_processed_hash: content_hash,
            updated_at: chrono::Utc::now().to_rfc3339(),
            total_items_estimate: self.total_items_estimate,
            items_processed: self.items_processed.saturating_add(1),
        }
    }

    /// Estimated fractional progress (`0.0` = no progress, `1.0` = complete).
    ///
    /// Returns `None` when [`SourceCursor::total_items_estimate`] is unknown,
    /// or when the estimate is zero (to avoid division by zero).
    pub fn estimated_progress(&self) -> Option<f64> {
        let total = self.total_items_estimate?;
        if total == 0 {
            return None;
        }
        Some((self.items_processed as f64 / total as f64).clamp(0.0, 1.0))
    }
}

// ── SourceStateTransitionError ─────────────────────────────────────────────

/// Errors produced by [`SourceStateValidator::can_ingest`] when the current
/// state combination does not permit ingestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceStateTransitionError {
    /// Cannot ingest: consent has not been approved.
    ConsentNotApproved {
        /// The actual consent state that blocked ingestion.
        current: ConsentState,
    },
    /// Cannot ingest: lifecycle state does not permit ingestion.
    LifecycleNotIngesting {
        /// The actual lifecycle state that blocked ingestion.
        current: SourceLifecycleState,
    },
    /// Cannot transition: source is in a terminal lifecycle state.
    TerminalState {
        /// The terminal lifecycle state.
        current: SourceLifecycleState,
    },
}

impl std::fmt::Display for SourceStateTransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceStateTransitionError::ConsentNotApproved { current } => {
                write!(
                    f,
                    "ingestion blocked: consent state is {current} (must be approved)"
                )
            }
            SourceStateTransitionError::LifecycleNotIngesting { current } => {
                write!(
                    f,
                    "ingestion blocked: lifecycle state is {current} \
                     (must be registered or paused)"
                )
            }
            SourceStateTransitionError::TerminalState { current } => {
                write!(
                    f,
                    "transition blocked: source is in terminal state {current}"
                )
            }
        }
    }
}

impl std::error::Error for SourceStateTransitionError {}

// ── SourceStateValidator ───────────────────────────────────────────────────

/// Validates that a consent + lifecycle state combination permits ingestion
/// (MGR-046).
///
/// This is a stateless validator: it takes the two state values and returns
/// either `Ok(())` or the first applicable [`SourceStateTransitionError`].
/// Terminal-state check runs before the lifecycle-ingestion check so that
/// a terminal source always returns [`SourceStateTransitionError::TerminalState`]
/// regardless of consent.
pub struct SourceStateValidator;

impl SourceStateValidator {
    /// Validate that ingestion is permitted given the current consent and
    /// lifecycle states.
    ///
    /// Returns `Ok(())` if and only if:
    /// * `consent` is [`ConsentState::Approved`], AND
    /// * `lifecycle` is [`SourceLifecycleState::Registered`] or
    ///   [`SourceLifecycleState::Paused`] (and therefore not terminal).
    ///
    /// Error precedence:
    /// 1. [`SourceStateTransitionError::TerminalState`] — lifecycle is terminal.
    /// 2. [`SourceStateTransitionError::ConsentNotApproved`] — consent is not
    ///    approved.
    /// 3. [`SourceStateTransitionError::LifecycleNotIngesting`] — lifecycle is
    ///    non-terminal but does not permit ingestion.
    pub fn can_ingest(
        consent: ConsentState,
        lifecycle: SourceLifecycleState,
    ) -> Result<(), SourceStateTransitionError> {
        // Terminal states block everything, regardless of consent.
        if lifecycle.is_terminal() {
            return Err(SourceStateTransitionError::TerminalState { current: lifecycle });
        }
        // Consent must be approved.
        if !consent.permits_ingestion() {
            return Err(SourceStateTransitionError::ConsentNotApproved { current: consent });
        }
        // Lifecycle must be in an ingestion-permitting state.
        if !lifecycle.can_ingest() {
            return Err(SourceStateTransitionError::LifecycleNotIngesting { current: lifecycle });
        }
        Ok(())
    }
}

// ── Unit tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── ConsentState::permits_ingestion ─────────────────────────────────

    #[test]
    fn consent_approved_permits_ingestion() {
        assert!(ConsentState::Approved.permits_ingestion());
    }

    #[test]
    fn consent_non_approved_blocks_ingestion() {
        assert!(!ConsentState::Pending.permits_ingestion());
        assert!(!ConsentState::Excluded.permits_ingestion());
        assert!(!ConsentState::Revoked.permits_ingestion());
    }

    // ── SourceLifecycleState::can_ingest ────────────────────────────────

    #[test]
    fn lifecycle_registered_and_paused_can_ingest() {
        assert!(SourceLifecycleState::Registered.can_ingest());
        assert!(SourceLifecycleState::Paused.can_ingest());
    }

    #[test]
    fn lifecycle_other_states_cannot_ingest() {
        assert!(!SourceLifecycleState::Ingesting.can_ingest());
        assert!(!SourceLifecycleState::Completed.can_ingest());
        assert!(!SourceLifecycleState::Failed.can_ingest());
        assert!(!SourceLifecycleState::Deleted.can_ingest());
        assert!(!SourceLifecycleState::Purged.can_ingest());
    }

    // ── SourceLifecycleState::is_terminal ───────────────────────────────

    #[test]
    fn lifecycle_deleted_and_purged_are_terminal() {
        assert!(SourceLifecycleState::Deleted.is_terminal());
        assert!(SourceLifecycleState::Purged.is_terminal());
    }

    #[test]
    fn lifecycle_non_terminal_states() {
        assert!(!SourceLifecycleState::Registered.is_terminal());
        assert!(!SourceLifecycleState::Ingesting.is_terminal());
        assert!(!SourceLifecycleState::Paused.is_terminal());
        assert!(!SourceLifecycleState::Completed.is_terminal());
        assert!(!SourceLifecycleState::Failed.is_terminal());
    }

    // ── SourceStateValidator::can_ingest ────────────────────────────────

    #[test]
    fn validator_permits_approved_registered() {
        assert!(SourceStateValidator::can_ingest(
            ConsentState::Approved,
            SourceLifecycleState::Registered
        )
        .is_ok());
    }

    #[test]
    fn validator_permits_approved_paused() {
        assert!(SourceStateValidator::can_ingest(
            ConsentState::Approved,
            SourceLifecycleState::Paused
        )
        .is_ok());
    }

    #[test]
    fn validator_blocks_when_consent_not_approved() {
        for consent in [
            ConsentState::Pending,
            ConsentState::Excluded,
            ConsentState::Revoked,
        ] {
            let err = SourceStateValidator::can_ingest(consent, SourceLifecycleState::Registered)
                .unwrap_err();
            assert!(
                matches!(err, SourceStateTransitionError::ConsentNotApproved { .. }),
                "expected ConsentNotApproved for {consent:?}, got {err:?}"
            );
        }
    }

    #[test]
    fn validator_blocks_when_lifecycle_not_ingesting() {
        for lifecycle in [
            SourceLifecycleState::Ingesting,
            SourceLifecycleState::Completed,
            SourceLifecycleState::Failed,
        ] {
            let err =
                SourceStateValidator::can_ingest(ConsentState::Approved, lifecycle).unwrap_err();
            assert!(
                matches!(
                    err,
                    SourceStateTransitionError::LifecycleNotIngesting { .. }
                ),
                "expected LifecycleNotIngesting for {lifecycle:?}, got {err:?}"
            );
        }
    }

    #[test]
    fn validator_returns_terminal_state_error_before_consent_check() {
        // Even with Approved consent, terminal lifecycle returns TerminalState.
        for lifecycle in [SourceLifecycleState::Deleted, SourceLifecycleState::Purged] {
            let err =
                SourceStateValidator::can_ingest(ConsentState::Approved, lifecycle).unwrap_err();
            assert!(
                matches!(err, SourceStateTransitionError::TerminalState { .. }),
                "expected TerminalState for {lifecycle:?}, got {err:?}"
            );
        }
    }

    // ── SourceCursor::start and advance ─────────────────────────────────

    #[test]
    fn cursor_start_creates_zero_progress_cursor() {
        let c = SourceCursor::start("0");
        assert_eq!(c.position, "0");
        assert_eq!(c.items_processed, 0);
        assert!(c.last_processed_hash.is_none());
        assert!(c.total_items_estimate.is_none());
    }

    #[test]
    fn cursor_advance_increments_items_processed() {
        let c = SourceCursor::start("0");
        let c2 = c.advance("1", Some("hash-abc".into()));
        assert_eq!(c2.position, "1");
        assert_eq!(c2.items_processed, 1);
        assert_eq!(c2.last_processed_hash.as_deref(), Some("hash-abc"));
        // Original cursor is unchanged.
        assert_eq!(c.position, "0");
        assert_eq!(c.items_processed, 0);
    }

    #[test]
    fn cursor_advance_preserves_total_estimate() {
        let mut c = SourceCursor::start("0");
        c.total_items_estimate = Some(100);
        let c2 = c.advance("1", None);
        assert_eq!(c2.total_items_estimate, Some(100));
    }

    // ── SourceCursor::estimated_progress ────────────────────────────────

    #[test]
    fn cursor_estimated_progress_none_when_total_unknown() {
        let c = SourceCursor::start("0");
        assert!(c.estimated_progress().is_none());
    }

    #[test]
    fn cursor_estimated_progress_none_when_total_zero() {
        let mut c = SourceCursor::start("0");
        c.total_items_estimate = Some(0);
        assert!(c.estimated_progress().is_none());
    }

    #[test]
    fn cursor_estimated_progress_fraction() {
        let mut c = SourceCursor::start("0");
        c.total_items_estimate = Some(10);
        c.items_processed = 5;
        let p = c.estimated_progress().unwrap();
        assert!((p - 0.5).abs() < f64::EPSILON, "expected 0.5, got {p}");
    }

    #[test]
    fn cursor_estimated_progress_clamped_to_one() {
        let mut c = SourceCursor::start("0");
        c.total_items_estimate = Some(5);
        c.items_processed = 10; // processed more than estimated
        let p = c.estimated_progress().unwrap();
        assert!((p - 1.0).abs() < f64::EPSILON, "expected 1.0, got {p}");
    }

    // ── Round-trip serde ─────────────────────────────────────────────────

    #[test]
    fn source_kind_serde_roundtrip() {
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
        for k in &kinds {
            let json = serde_json::to_string(k).unwrap();
            let back: SourceKind = serde_json::from_str(&json).unwrap();
            assert_eq!(&back, k, "serde roundtrip failed for {k:?}");
        }
    }

    #[test]
    fn source_kind_as_str_parses_back() {
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
        for k in &kinds {
            let s = k.as_str();
            let back: SourceKind = s.parse().unwrap();
            assert_eq!(&back, k);
        }
    }

    #[test]
    fn trust_class_ordering() {
        // Native is most trusted (lowest ordinal), Unknown is least trusted.
        assert!(SourceTrustClass::Native < SourceTrustClass::Verified);
        assert!(SourceTrustClass::Verified < SourceTrustClass::ThirdParty);
        assert!(SourceTrustClass::ThirdParty < SourceTrustClass::External);
        assert!(SourceTrustClass::External < SourceTrustClass::Unknown);
    }

    #[test]
    fn cursor_serde_roundtrip() {
        let c = SourceCursor::start("checkpoint-42");
        let json = serde_json::to_string(&c).unwrap();
        let back: SourceCursor = serde_json::from_str(&json).unwrap();
        assert_eq!(back.position, c.position);
        assert_eq!(back.items_processed, c.items_processed);
    }
}
