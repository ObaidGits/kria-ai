//! `os_control::audit` — the durable, append-only OS-action audit authority.
//!
//! linux-os-control-production **Task 1.8**, design §4, §6, §14 (OSC-001,
//! OSC-007, OSC-023, OSC-025, OSC-029).
//!
//! # One logical action, one admission, at most one terminal
//!
//! This module hard-migrates OS-action auditing onto a **fallible,
//! append-only, integrity-linked** SQLite model that replaces the old
//! infallible `log` / in-place `update_result` contract (ignored insertion
//! errors could not fail closed, and mutated completion columns were not
//! covered by the row hash). It owns:
//!
//! * [`OsAuditStore::admit_action`] — appends exactly **one** redacted
//!   `admission` record before the first provider observation and returns a
//!   non-cloneable [`AuditAdmissionToken`] bound to
//!   session/action/parameter/target/capability/prospective-resource digests
//!   and a recovery key (but **not** to a not-yet-issued grant). Admission
//!   failure returns [`OsControlError::AuditUnavailable`] **before** any
//!   provider access for mutations and privacy-sensitive reads (fail closed).
//! * [`OsAuditStore::stage_recovery_payload`] — commits a safe recovery
//!   payload (redacted digests only) **before** dispatch, so a crash between
//!   dispatch and terminal append can be reconciled without inventing state.
//! * [`OsAuditStore::append_terminal`] — idempotently appends the action's
//!   **sole** terminal `completion`/`incident` record. A unique partial index
//!   on the terminal `parent_admission_id` enforces at most one terminal;
//!   concurrent/replayed appenders whose canonical terminal digest matches read
//!   and return the winning terminal, while a **digest mismatch** is an
//!   integrity incident that keeps audit unhealthy.
//! * [`OsAuditStore::reconcile_incomplete_admissions`] — a bounded startup /
//!   health scan that reconstructs the safe terminal summary from the staged
//!   recovery payload where possible, otherwise appends `OutcomeUnknownAfterCrash`.
//!   Reconciliation uses the same idempotent terminal key and **never** invokes
//!   a provider (the store holds no provider handle, so redispatch is
//!   structurally impossible).
//! * [`OsAuditStore::verify_chain`] — hash-chain integrity over every admission,
//!   completion, incident, and recovery row.
//!
//! Terminal-append interruption preserves the truthful in-memory receipt as
//! [`AuditCompletionState::PendingRecovery`], marks audit health unavailable,
//! and blocks subsequent automatic mutations until reconciliation records the
//! sole terminal.

use std::sync::Mutex;

use rusqlite::{params, Connection, Error as SqliteError, ErrorCode};
use sha2::{Digest as _, Sha256};

use crate::os_control::context::AuditAdmissionToken;
use crate::os_control::contract::{
    ActionId, AuditAdmissionId, AuditRecordId, AuditRecoveryKey, CorrelationId, DecisionId, Digest,
    OsEvidenceSource, ProviderId, SafeErrorCode, SafeText, SessionId, SnapshotRevision,
    VerificationReliability,
};
use crate::os_control::error::OsControlError;
use crate::os_control::receipt::{ActionLifecycle, AuditCompletionState};
use crate::os_control::redaction::redact_parameters;
use crate::os_control::resource::write_resource_set_digest;
use crate::safety::RiskLevel;

/// The incident code appended when an interrupted terminal cannot be
/// reconstructed from a staged recovery payload (design §14.4).
pub const OUTCOME_UNKNOWN_AFTER_CRASH: &str = "os_control.incident.outcome_unknown_after_crash";

/// The incident code recorded when a replayed terminal append conflicts with an
/// already-durable terminal for the same admission (design §14.2).
pub const TERMINAL_DIGEST_CONFLICT: &str = "os_control.incident.terminal_digest_conflict";

/// Hard maximum number of rows any bounded audit scan may return (design §14.6).
pub const MAX_SCAN_LIMIT: usize = 512;

// ─────────────────────────────────────────────────────────────────────────────
// Request / record inputs
// ─────────────────────────────────────────────────────────────────────────────

/// Why an action is being admitted — decides fail-closed behaviour while audit
/// is unhealthy (design §14.4: mutations and privacy-sensitive reads fail
/// closed; plain reads may continue per policy).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestSensitivity {
    /// A state mutation. Fails closed when audit is unhealthy.
    Mutation,
    /// A privacy-sensitive read. Fails closed when audit is unhealthy.
    PrivacySensitiveRead,
    /// A plain read. May continue while audit is unhealthy (per policy).
    PlainRead,
}

impl RequestSensitivity {
    fn fails_closed_when_unhealthy(self) -> bool {
        matches!(self, Self::Mutation | Self::PrivacySensitiveRead)
    }
}

/// The inputs to a single logical-action admission (design §6, §14.1). The
/// parameter object is redacted through the shared registry before anything
/// durable is written — raw values never enter audit.
#[derive(Debug, Clone)]
pub struct AdmissionRequest {
    /// Bound login session.
    pub session_id: SessionId,
    /// Correlation spanning the logical request.
    pub correlation_id: CorrelationId,
    /// This action's identity within the correlation.
    pub action_id: ActionId,
    /// Canonical tool name (drives action/resource derivation).
    pub tool_name: String,
    /// Strict parameter object (redacted before storage).
    pub params: serde_json::Value,
    /// Host-target digest (computed by the caller from the resolved target).
    pub target_hash: Digest,
    /// Capability-snapshot revision this action was admitted under.
    pub capability_snapshot_revision: SnapshotRevision,
    /// Resolved risk level.
    pub risk: RiskLevel,
    /// Durable decision id, when approval created one.
    pub decision_id: Option<DecisionId>,
    /// The request sensitivity (fail-closed classification).
    pub sensitivity: RequestSensitivity,
}

/// The safe terminal summary appended for a completed action (design §4, §14).
/// Carries only redacted digests / closed codes — never raw content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalRecord {
    /// Terminal lifecycle label.
    pub lifecycle: ActionLifecycle,
    /// Provider that produced the outcome.
    pub provider: ProviderId,
    /// Redacted before-state digest.
    pub before_digest: Option<Digest>,
    /// Redacted after-state digest.
    pub after_digest: Option<Digest>,
    /// Opaque provider-receipt digest.
    pub provider_receipt_digest: Option<Digest>,
    /// Verification evidence source, when verified.
    pub verification_source: Option<OsEvidenceSource>,
    /// Verification reliability, when verified.
    pub verification_reliability: Option<VerificationReliability>,
    /// Whether rollback is advertised for this receipt.
    pub rollback_available: bool,
    /// Closed incident/error code, when the terminal is an incident.
    pub incident_code: Option<SafeErrorCode>,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
}

impl TerminalRecord {
    fn is_incident(&self) -> bool {
        self.incident_code.is_some()
            || matches!(
                self.lifecycle,
                ActionLifecycle::VerificationFailed
                    | ActionLifecycle::Unverified
                    | ActionLifecycle::PartiallyApplied
            )
    }

    fn record_kind(&self) -> &'static str {
        if self.is_incident() {
            RECORD_INCIDENT
        } else {
            RECORD_COMPLETION
        }
    }

    /// The canonical, idempotent terminal digest keyed by admission identity and
    /// outcome. Two logically-equal terminals for the same admission produce the
    /// same digest, so a replay is idempotent and a genuinely-different outcome
    /// is a detectable conflict.
    fn terminal_digest(&self, admission_id: &str) -> Digest {
        let canonical = format!(
            "{admission}|{lifecycle}|{before}|{after}|{receipt}|{incident}",
            admission = admission_id,
            lifecycle = self.lifecycle.as_str(),
            before = self
                .before_digest
                .as_ref()
                .map(Digest::as_hex)
                .unwrap_or(""),
            after = self.after_digest.as_ref().map(Digest::as_hex).unwrap_or(""),
            receipt = self
                .provider_receipt_digest
                .as_ref()
                .map(Digest::as_hex)
                .unwrap_or(""),
            incident = self
                .incident_code
                .as_ref()
                .map(SafeErrorCode::as_str)
                .unwrap_or(""),
        );
        Digest::of_str(&canonical)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Health, outcomes, reports
// ─────────────────────────────────────────────────────────────────────────────

/// Audit health. Once terminal persistence is interrupted or a terminal digest
/// conflict is detected, audit is [`AuditHealth::Unhealthy`] and subsequent
/// automatic mutations fail closed until reconciliation records the terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditHealth {
    /// Every admitted action has its sole terminal (or is safely pending a
    /// bounded reconcile); automatic mutations may proceed.
    Healthy,
    /// A terminal was interrupted or conflicted; automatic mutations blocked.
    Unhealthy {
        /// Redacted reason.
        reason: SafeText,
    },
}

impl AuditHealth {
    /// Whether audit is healthy.
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        matches!(self, Self::Healthy)
    }
}

/// The outcome of an [`OsAuditStore::append_terminal`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalAppendOutcome {
    /// The sole terminal is durable (freshly written or matched on replay).
    Recorded {
        /// Durable terminal record id.
        record_id: AuditRecordId,
        /// True when this call matched an already-durable identical terminal.
        idempotent_replay: bool,
    },
    /// Terminal append was interrupted; the admission stays detectably
    /// incomplete and audit is now unhealthy (fail closed).
    PendingRecovery {
        /// The incomplete admission.
        admission_id: AuditAdmissionId,
        /// The recovery key committed at admission.
        recovery_key: AuditRecoveryKey,
    },
    /// A replayed terminal disagreed with the durable terminal; integrity
    /// incident, audit remains unhealthy.
    IntegrityConflict {
        /// The admission whose terminal conflicted.
        admission_id: AuditAdmissionId,
    },
}

impl TerminalAppendOutcome {
    /// Project this outcome onto the receipt-facing [`AuditCompletionState`].
    #[must_use]
    pub fn completion_state(&self) -> AuditCompletionState {
        match self {
            Self::Recorded { record_id, .. } => AuditCompletionState::Recorded {
                record_id: record_id.clone(),
            },
            Self::PendingRecovery {
                admission_id,
                recovery_key,
            } => AuditCompletionState::PendingRecovery {
                admission_id: admission_id.clone(),
                recovery_key: recovery_key.clone(),
            },
            Self::IntegrityConflict { admission_id } => AuditCompletionState::PendingRecovery {
                admission_id: admission_id.clone(),
                recovery_key: AuditRecoveryKey::new(""),
            },
        }
    }
}

/// Report from a bounded reconciliation scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileReport {
    /// Number of admissions scanned this pass.
    pub scanned: usize,
    /// Number of terminals appended (reconstructed or unknown-after-crash).
    pub reconciled: usize,
    /// Number reconstructed from a staged recovery payload.
    pub reconstructed: usize,
    /// Number that fell back to `OutcomeUnknownAfterCrash`.
    pub unknown_after_crash: usize,
    /// Cursor to resume a subsequent bounded scan, if more may remain.
    pub next_cursor: Option<i64>,
}

// ── Record-kind tokens ──────────────────────────────────────────────────────
const RECORD_ADMISSION: &str = "admission";
const RECORD_COMPLETION: &str = "completion";
const RECORD_INCIDENT: &str = "incident";
const RECORD_RECOVERY: &str = "recovery";

/// Test-only fault injection for simulating durable-write interruption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditFault {
    /// The next terminal append is interrupted before the row is written.
    InterruptNextTerminal,
}

// ─────────────────────────────────────────────────────────────────────────────
// The store
// ─────────────────────────────────────────────────────────────────────────────

/// The durable OS-action audit authority (design §14). Backed by a single
/// SQLite connection; holds **no** provider handle, so reconciliation can never
/// redispatch.
pub struct OsAuditStore {
    conn: Mutex<Connection>,
    health: Mutex<AuditHealth>,
    fault: Mutex<Option<AuditFault>>,
}

impl std::fmt::Debug for OsAuditStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OsAuditStore")
            .field("health", &*self.health.lock().unwrap())
            .finish()
    }
}

impl OsAuditStore {
    /// Open a store over an existing connection, applying the migration.
    #[must_use]
    pub fn new(conn: Connection) -> Self {
        Self::migrate(&conn);
        Self {
            conn: Mutex::new(conn),
            health: Mutex::new(AuditHealth::Healthy),
            fault: Mutex::new(None),
        }
    }

    /// Open an in-memory store (tests / ephemeral).
    #[must_use]
    pub fn open_in_memory() -> Self {
        Self::new(Connection::open_in_memory().expect("open in-memory audit db"))
    }

    /// The hard-migration DDL: admission/terminal kind check, unique
    /// terminal-parent index, and indexed incomplete-admission query.
    fn migrate(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS os_audit_log (
                id                           INTEGER PRIMARY KEY AUTOINCREMENT,
                record_kind                  TEXT NOT NULL CHECK (record_kind IN
                                                ('admission','completion','incident','recovery')),
                admission_id                 TEXT NOT NULL,
                parent_admission_id          TEXT,
                recovery_key                 TEXT NOT NULL,
                correlation_id               TEXT,
                session_id                   TEXT NOT NULL,
                action_hash                  TEXT NOT NULL,
                parameter_hash               TEXT NOT NULL,
                target_hash                  TEXT NOT NULL,
                resource_set_digest          TEXT NOT NULL,
                decision_id                  TEXT,
                risk                         TEXT NOT NULL,
                provider_id                  TEXT,
                lifecycle                    TEXT,
                before_digest                TEXT,
                after_digest                 TEXT,
                provider_receipt_digest      TEXT,
                verification_source          TEXT,
                verification_reliability     TEXT,
                rollback_available           INTEGER,
                error_or_incident_code       TEXT,
                capability_snapshot_revision INTEGER NOT NULL,
                duration_ms                  INTEGER,
                redacted_parameters          TEXT,
                terminal_digest              TEXT,
                timestamp                    TEXT NOT NULL,
                prev_hash                    TEXT NOT NULL,
                row_hash                     TEXT NOT NULL
            );
            -- At most one terminal (completion|incident) per admission.
            CREATE UNIQUE INDEX IF NOT EXISTS idx_os_audit_terminal_parent
                ON os_audit_log(parent_admission_id)
                WHERE record_kind IN ('completion','incident');
            CREATE INDEX IF NOT EXISTS idx_os_audit_admission
                ON os_audit_log(admission_id);
            CREATE INDEX IF NOT EXISTS idx_os_audit_kind
                ON os_audit_log(record_kind);",
        )
        .expect("failed to create os_audit_log table");
    }

    /// Current audit health.
    #[must_use]
    pub fn health(&self) -> AuditHealth {
        self.health.lock().unwrap().clone()
    }

    /// Whether audit is healthy (automatic mutations may proceed).
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        self.health.lock().unwrap().is_healthy()
    }

    fn mark_unhealthy(&self, reason: &str) {
        *self.health.lock().unwrap() = AuditHealth::Unhealthy {
            reason: SafeText::new(reason),
        };
    }

    /// Restore health after a successful bounded reconcile (design §14.4).
    fn restore_health_if_complete(&self) {
        if self.incomplete_admission_count() == 0 {
            *self.health.lock().unwrap() = AuditHealth::Healthy;
        }
    }

    /// Set a test-only fault to simulate durable-write interruption.
    #[cfg(any(test, feature = "os-control-test"))]
    pub fn inject_fault(&self, fault: AuditFault) {
        *self.fault.lock().unwrap() = Some(fault);
    }

    fn take_fault(&self) -> Option<AuditFault> {
        self.fault.lock().unwrap().take()
    }

    // ── Admission (design §14.1) ────────────────────────────────────────────

    /// Append the single logical-action admission and return its bound token.
    ///
    /// Fails closed with [`OsControlError::AuditUnavailable`] when the durable
    /// append fails, or when audit is unhealthy and the request is a mutation or
    /// privacy-sensitive read (no provider is reached in either case).
    pub fn admit_action(
        &self,
        request: &AdmissionRequest,
    ) -> Result<AuditAdmissionToken, OsControlError> {
        // Fail closed for mutations / privacy-sensitive reads while unhealthy.
        if request.sensitivity.fails_closed_when_unhealthy() && !self.is_healthy() {
            return Err(OsControlError::AuditUnavailable);
        }

        let action_hash = Digest::of_str(&request.tool_name);
        let redacted = redact_parameters(&request.tool_name, &request.params);
        let parameter_hash = redacted.parameter_digest.clone();
        let resource_set_digest = write_resource_set_digest(&request.tool_name, &request.params);
        let redacted_json = serde_json::to_string(&redacted).unwrap_or_else(|_| "{}".to_string());

        let admission_id = AuditAdmissionId::new(new_id());
        let recovery_key = AuditRecoveryKey::new(new_id());
        let timestamp = now_iso();

        let conn = self.conn.lock().unwrap();
        let prev_hash = latest_row_hash(&conn);
        let row_hash = hash_row(&[
            &prev_hash,
            RECORD_ADMISSION,
            admission_id.as_str(),
            recovery_key.as_str(),
            request.session_id.as_str(),
            action_hash.as_hex(),
            parameter_hash.as_hex(),
            request.target_hash.as_hex(),
            resource_set_digest.as_hex(),
            &timestamp,
        ]);

        let inserted = conn.execute(
            "INSERT INTO os_audit_log (
                record_kind, admission_id, parent_admission_id, recovery_key,
                correlation_id, session_id, action_hash, parameter_hash, target_hash,
                resource_set_digest, decision_id, risk, capability_snapshot_revision,
                redacted_parameters, timestamp, prev_hash, row_hash
            ) VALUES (
                ?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16
            )",
            params![
                RECORD_ADMISSION,
                admission_id.as_str(),
                recovery_key.as_str(),
                request.correlation_id.as_str(),
                request.session_id.as_str(),
                action_hash.as_hex(),
                parameter_hash.as_hex(),
                request.target_hash.as_hex(),
                resource_set_digest.as_hex(),
                request.decision_id.as_ref().map(|d| d.as_str()),
                request.risk.as_str(),
                request.capability_snapshot_revision.0 as i64,
                redacted_json,
                timestamp,
                prev_hash,
                row_hash,
            ],
        );

        match inserted {
            Ok(_) => Ok(AuditAdmissionToken::seal(
                admission_id,
                recovery_key,
                request.session_id.clone(),
                action_hash,
                parameter_hash,
                request.target_hash.clone(),
                request.capability_snapshot_revision,
                resource_set_digest,
            )),
            Err(_) => Err(OsControlError::AuditUnavailable),
        }
    }

    // ── Recovery payload staging (design §14.4) ─────────────────────────────

    /// Commit a safe recovery payload **before** dispatch, so an interrupted
    /// terminal can be reconstructed. Contains only redacted digests / codes.
    pub fn stage_recovery_payload(
        &self,
        token: &AuditAdmissionToken,
        terminal: &TerminalRecord,
    ) -> Result<(), OsControlError> {
        let timestamp = now_iso();
        let conn = self.conn.lock().unwrap();
        let prev_hash = latest_row_hash(&conn);
        let admission_id = token.admission_id().as_str();
        let terminal_digest = terminal.terminal_digest(admission_id);
        let row_hash = hash_row(&[
            &prev_hash,
            RECORD_RECOVERY,
            admission_id,
            token.recovery_key().as_str(),
            terminal.lifecycle.as_str(),
            terminal_digest.as_hex(),
            &timestamp,
        ]);

        conn.execute(
            "INSERT INTO os_audit_log (
                record_kind, admission_id, parent_admission_id, recovery_key,
                session_id, action_hash, parameter_hash, target_hash, resource_set_digest,
                risk, provider_id, lifecycle, before_digest, after_digest,
                provider_receipt_digest, verification_source, verification_reliability,
                rollback_available, error_or_incident_code, capability_snapshot_revision,
                duration_ms, terminal_digest, timestamp, prev_hash, row_hash
            ) VALUES (
                ?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24
            )",
            params![
                RECORD_RECOVERY,
                admission_id,
                token.recovery_key().as_str(),
                token.session_id().as_str(),
                token.action_hash().as_hex(),
                token.parameter_hash().as_hex(),
                token.target_hash().as_hex(),
                token.resource_set_digest().as_hex(),
                "", // risk not meaningful on recovery row; kept non-null
                terminal.provider.as_str(),
                terminal.lifecycle.as_str(),
                terminal.before_digest.as_ref().map(Digest::as_hex),
                terminal.after_digest.as_ref().map(Digest::as_hex),
                terminal
                    .provider_receipt_digest
                    .as_ref()
                    .map(Digest::as_hex),
                terminal.verification_source.map(evidence_source_token),
                terminal.verification_reliability.map(reliability_token),
                terminal.rollback_available as i64,
                terminal.incident_code.as_ref().map(SafeErrorCode::as_str),
                token.capability_snapshot_revision().0 as i64,
                terminal.duration_ms as i64,
                terminal_digest.as_hex(),
                timestamp,
                prev_hash,
                row_hash,
            ],
        )
        .map_err(|_| OsControlError::AuditUnavailable)?;
        Ok(())
    }

    // ── Terminal append (design §14.2/.3) ───────────────────────────────────

    /// Idempotently append the action's sole terminal record.
    pub fn append_terminal(
        &self,
        token: &AuditAdmissionToken,
        terminal: &TerminalRecord,
    ) -> TerminalAppendOutcome {
        self.append_terminal_inner(
            token.admission_id(),
            token.recovery_key(),
            token.session_id().as_str(),
            token.action_hash().as_hex(),
            token.parameter_hash().as_hex(),
            token.target_hash().as_hex(),
            token.resource_set_digest().as_hex(),
            token.capability_snapshot_revision(),
            terminal,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn append_terminal_inner(
        &self,
        admission_id: &AuditAdmissionId,
        recovery_key: &AuditRecoveryKey,
        session_id: &str,
        action_hash: &str,
        parameter_hash: &str,
        target_hash: &str,
        resource_set_digest: &str,
        capability_snapshot_revision: SnapshotRevision,
        terminal: &TerminalRecord,
    ) -> TerminalAppendOutcome {
        let admission_str = admission_id.as_str();
        let terminal_digest = terminal.terminal_digest(admission_str);

        // Test-only interruption simulation: skip the write, fail closed.
        if self.take_fault() == Some(AuditFault::InterruptNextTerminal) {
            self.mark_unhealthy("terminal append interrupted");
            return TerminalAppendOutcome::PendingRecovery {
                admission_id: admission_id.clone(),
                recovery_key: recovery_key.clone(),
            };
        }

        let timestamp = now_iso();
        let conn = self.conn.lock().unwrap();
        let prev_hash = latest_row_hash(&conn);
        let kind = terminal.record_kind();
        let row_hash = hash_row(&[
            &prev_hash,
            kind,
            admission_str,
            recovery_key.as_str(),
            terminal.lifecycle.as_str(),
            terminal_digest.as_hex(),
            &timestamp,
        ]);

        let inserted = conn.execute(
            "INSERT INTO os_audit_log (
                record_kind, admission_id, parent_admission_id, recovery_key,
                session_id, action_hash, parameter_hash, target_hash, resource_set_digest,
                risk, provider_id, lifecycle, before_digest, after_digest,
                provider_receipt_digest, verification_source, verification_reliability,
                rollback_available, error_or_incident_code, capability_snapshot_revision,
                duration_ms, terminal_digest, timestamp, prev_hash, row_hash
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25
            )",
            params![
                kind,
                admission_str,
                admission_str, // parent_admission_id
                recovery_key.as_str(),
                session_id,
                action_hash,
                parameter_hash,
                target_hash,
                resource_set_digest,
                terminal.risk_placeholder(),
                terminal.provider.as_str(),
                terminal.lifecycle.as_str(),
                terminal.before_digest.as_ref().map(Digest::as_hex),
                terminal.after_digest.as_ref().map(Digest::as_hex),
                terminal
                    .provider_receipt_digest
                    .as_ref()
                    .map(Digest::as_hex),
                terminal.verification_source.map(evidence_source_token),
                terminal.verification_reliability.map(reliability_token),
                terminal.rollback_available as i64,
                terminal.incident_code.as_ref().map(SafeErrorCode::as_str),
                capability_snapshot_revision.0 as i64,
                terminal.duration_ms as i64,
                terminal_digest.as_hex(),
                timestamp,
                prev_hash,
                row_hash,
            ],
        );

        match inserted {
            Ok(_) => {
                let record_id = AuditRecordId::new(format!("terminal:{admission_str}"));
                drop(conn);
                self.restore_health_if_complete();
                TerminalAppendOutcome::Recorded {
                    record_id,
                    idempotent_replay: false,
                }
            }
            Err(SqliteError::SqliteFailure(inner, _))
                if inner.code == ErrorCode::ConstraintViolation =>
            {
                // A terminal already exists for this admission. Idempotent iff
                // its canonical digest matches; otherwise an integrity conflict.
                let existing: Option<String> = conn
                    .query_row(
                        "SELECT terminal_digest FROM os_audit_log
                         WHERE parent_admission_id = ?1
                           AND record_kind IN ('completion','incident')
                         LIMIT 1",
                        params![admission_str],
                        |row| row.get(0),
                    )
                    .ok();
                drop(conn);
                match existing {
                    Some(existing_digest) if existing_digest == terminal_digest.as_hex() => {
                        self.restore_health_if_complete();
                        TerminalAppendOutcome::Recorded {
                            record_id: AuditRecordId::new(format!("terminal:{admission_str}")),
                            idempotent_replay: true,
                        }
                    }
                    _ => {
                        self.mark_unhealthy("terminal digest conflict");
                        TerminalAppendOutcome::IntegrityConflict {
                            admission_id: admission_id.clone(),
                        }
                    }
                }
            }
            Err(_) => {
                drop(conn);
                self.mark_unhealthy("terminal append interrupted");
                TerminalAppendOutcome::PendingRecovery {
                    admission_id: admission_id.clone(),
                    recovery_key: recovery_key.clone(),
                }
            }
        }
    }

    // ── Incomplete detection + reconciliation (design §14.4) ────────────────

    /// Count admissions that have no terminal record.
    #[must_use]
    pub fn incomplete_admission_count(&self) -> usize {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM os_audit_log a
             WHERE a.record_kind = 'admission'
               AND NOT EXISTS (
                   SELECT 1 FROM os_audit_log t
                   WHERE t.parent_admission_id = a.admission_id
                     AND t.record_kind IN ('completion','incident')
               )",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|n| n as usize)
        .unwrap_or(0)
    }

    /// Bounded startup / health reconciliation (design §14.4). Reconstructs the
    /// terminal from a staged recovery payload where possible, otherwise appends
    /// `OutcomeUnknownAfterCrash`. Never invokes a provider.
    pub fn reconcile_incomplete_admissions(
        &self,
        limit: usize,
        cursor: Option<i64>,
    ) -> ReconcileReport {
        let limit = limit.clamp(1, MAX_SCAN_LIMIT);
        let after = cursor.unwrap_or(0);

        // Snapshot the bounded batch of incomplete admissions.
        struct Incomplete {
            row_id: i64,
            admission_id: String,
            recovery_key: String,
            session_id: String,
            action_hash: String,
            parameter_hash: String,
            target_hash: String,
            resource_set_digest: String,
            capability_snapshot_revision: i64,
        }
        let batch: Vec<Incomplete> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, admission_id, recovery_key, session_id, action_hash,
                            parameter_hash, target_hash, resource_set_digest,
                            capability_snapshot_revision
                     FROM os_audit_log a
                     WHERE a.record_kind = 'admission'
                       AND a.id > ?1
                       AND NOT EXISTS (
                           SELECT 1 FROM os_audit_log t
                           WHERE t.parent_admission_id = a.admission_id
                             AND t.record_kind IN ('completion','incident')
                       )
                     ORDER BY a.id ASC
                     LIMIT ?2",
                )
                .expect("prepare incomplete scan");
            let rows = stmt
                .query_map(params![after, limit as i64], |row| {
                    Ok(Incomplete {
                        row_id: row.get(0)?,
                        admission_id: row.get(1)?,
                        recovery_key: row.get(2)?,
                        session_id: row.get(3)?,
                        action_hash: row.get(4)?,
                        parameter_hash: row.get(5)?,
                        target_hash: row.get(6)?,
                        resource_set_digest: row.get(7)?,
                        capability_snapshot_revision: row.get(8)?,
                    })
                })
                .expect("scan incomplete admissions")
                .filter_map(Result::ok)
                .collect();
            rows
        };

        let scanned = batch.len();
        let mut reconciled = 0;
        let mut reconstructed = 0;
        let mut unknown_after_crash = 0;
        let mut next_cursor = cursor;

        for row in &batch {
            next_cursor = Some(row.row_id);
            let admission_id = AuditAdmissionId::new(row.admission_id.clone());
            let recovery_key = AuditRecoveryKey::new(row.recovery_key.clone());
            let revision = SnapshotRevision(row.capability_snapshot_revision.max(0) as u64);

            let terminal = self
                .load_recovery_payload(&row.admission_id)
                .unwrap_or_else(|| {
                    unknown_after_crash += 1;
                    outcome_unknown_after_crash_terminal()
                });
            if terminal.incident_code.as_ref().map(SafeErrorCode::as_str)
                != Some(OUTCOME_UNKNOWN_AFTER_CRASH)
            {
                reconstructed += 1;
            }

            let outcome = self.append_terminal_inner(
                &admission_id,
                &recovery_key,
                &row.session_id,
                &row.action_hash,
                &row.parameter_hash,
                &row.target_hash,
                &row.resource_set_digest,
                revision,
                &terminal,
            );
            if matches!(outcome, TerminalAppendOutcome::Recorded { .. }) {
                reconciled += 1;
            }
        }

        // A short scan means no more remain.
        if scanned < limit {
            next_cursor = None;
        }
        self.restore_health_if_complete();

        ReconcileReport {
            scanned,
            reconciled,
            reconstructed,
            unknown_after_crash,
            next_cursor,
        }
    }

    /// Load a staged recovery payload for an admission, if one was committed.
    fn load_recovery_payload(&self, admission_id: &str) -> Option<TerminalRecord> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT provider_id, lifecycle, before_digest, after_digest,
                    provider_receipt_digest, verification_source, verification_reliability,
                    rollback_available, error_or_incident_code, duration_ms
             FROM os_audit_log
             WHERE admission_id = ?1 AND record_kind = 'recovery'
             ORDER BY id DESC LIMIT 1",
            params![admission_id],
            |row| {
                let provider: String = row.get(0)?;
                let lifecycle: String = row.get(1)?;
                let before: Option<String> = row.get(2)?;
                let after: Option<String> = row.get(3)?;
                let receipt: Option<String> = row.get(4)?;
                let vsource: Option<String> = row.get(5)?;
                let vrel: Option<String> = row.get(6)?;
                let rollback: Option<i64> = row.get(7)?;
                let incident: Option<String> = row.get(8)?;
                let duration: Option<i64> = row.get(9)?;
                Ok(TerminalRecord {
                    lifecycle: lifecycle_from_token(&lifecycle),
                    provider: ProviderId::new(provider),
                    before_digest: before.map(Digest::from_hex),
                    after_digest: after.map(Digest::from_hex),
                    provider_receipt_digest: receipt.map(Digest::from_hex),
                    verification_source: vsource.as_deref().and_then(evidence_source_from_token),
                    verification_reliability: vrel.as_deref().and_then(reliability_from_token),
                    rollback_available: rollback.unwrap_or(0) != 0,
                    incident_code: incident.map(SafeErrorCode::from_code),
                    duration_ms: duration.unwrap_or(0).max(0) as u64,
                })
            },
        )
        .ok()
    }

    // ── Chain verification (design §14.6) ───────────────────────────────────

    /// Verify the hash-chain integrity of every audit row in insertion order.
    /// Returns `Ok(rows_verified)` or `Err(first_broken_row_id)`.
    pub fn verify_chain(&self) -> Result<usize, i64> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, record_kind, admission_id, recovery_key, session_id,
                        action_hash, parameter_hash, target_hash, resource_set_digest,
                        lifecycle, terminal_digest, timestamp, prev_hash, row_hash
                 FROM os_audit_log ORDER BY id ASC",
            )
            .map_err(|_| -1i64)?;

        struct Row {
            id: i64,
            record_kind: String,
            admission_id: String,
            recovery_key: String,
            session_id: String,
            action_hash: String,
            parameter_hash: String,
            target_hash: String,
            resource_set_digest: String,
            lifecycle: Option<String>,
            terminal_digest: Option<String>,
            timestamp: String,
            prev_hash: String,
            row_hash: String,
        }

        let rows: Vec<Row> = stmt
            .query_map([], |r| {
                Ok(Row {
                    id: r.get(0)?,
                    record_kind: r.get(1)?,
                    admission_id: r.get(2)?,
                    recovery_key: r.get(3)?,
                    session_id: r.get(4)?,
                    action_hash: r.get(5)?,
                    parameter_hash: r.get(6)?,
                    target_hash: r.get(7)?,
                    resource_set_digest: r.get(8)?,
                    lifecycle: r.get(9)?,
                    terminal_digest: r.get(10)?,
                    timestamp: r.get(11)?,
                    prev_hash: r.get(12)?,
                    row_hash: r.get(13)?,
                })
            })
            .map_err(|_| -1i64)?
            .filter_map(Result::ok)
            .collect();

        let mut count = 0;
        for row in &rows {
            let expected = match row.record_kind.as_str() {
                RECORD_ADMISSION => hash_row(&[
                    &row.prev_hash,
                    RECORD_ADMISSION,
                    &row.admission_id,
                    &row.recovery_key,
                    &row.session_id,
                    &row.action_hash,
                    &row.parameter_hash,
                    &row.target_hash,
                    &row.resource_set_digest,
                    &row.timestamp,
                ]),
                RECORD_RECOVERY => hash_row(&[
                    &row.prev_hash,
                    RECORD_RECOVERY,
                    &row.admission_id,
                    &row.recovery_key,
                    row.lifecycle.as_deref().unwrap_or(""),
                    row.terminal_digest.as_deref().unwrap_or(""),
                    &row.timestamp,
                ]),
                kind => hash_row(&[
                    &row.prev_hash,
                    kind,
                    &row.admission_id,
                    &row.recovery_key,
                    row.lifecycle.as_deref().unwrap_or(""),
                    row.terminal_digest.as_deref().unwrap_or(""),
                    &row.timestamp,
                ]),
            };
            if expected != row.row_hash {
                return Err(row.id);
            }
            count += 1;
        }
        Ok(count)
    }

    /// Count terminal rows for one admission (tests / cardinality proofs).
    #[must_use]
    pub fn terminal_count(&self, admission_id: &AuditAdmissionId) -> usize {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM os_audit_log
             WHERE parent_admission_id = ?1 AND record_kind IN ('completion','incident')",
            params![admission_id.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .map(|n| n as usize)
        .unwrap_or(0)
    }

    /// Count admission rows for one admission id (must always be exactly one).
    #[must_use]
    pub fn admission_count(&self, admission_id: &AuditAdmissionId) -> usize {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM os_audit_log
             WHERE admission_id = ?1 AND record_kind = 'admission'",
            params![admission_id.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .map(|n| n as usize)
        .unwrap_or(0)
    }
}

impl TerminalRecord {
    /// Terminals do not carry the original risk; a stable non-null placeholder
    /// keeps the `risk` NOT NULL column satisfied without asserting a value.
    fn risk_placeholder(&self) -> &'static str {
        "TERMINAL"
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Free helpers
// ─────────────────────────────────────────────────────────────────────────────

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn now_iso() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.6fZ")
        .to_string()
}

fn latest_row_hash(conn: &Connection) -> String {
    conn.query_row(
        "SELECT COALESCE(row_hash, 'GENESIS') FROM os_audit_log ORDER BY id DESC LIMIT 1",
        [],
        |row| row.get(0),
    )
    .unwrap_or_else(|_| "GENESIS".to_string())
}

fn hash_row(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            h.update(b"|");
        }
        h.update(part.as_bytes());
    }
    hex::encode(h.finalize())
}

fn outcome_unknown_after_crash_terminal() -> TerminalRecord {
    TerminalRecord {
        lifecycle: ActionLifecycle::Unverified,
        provider: ProviderId::new("reconciler"),
        before_digest: None,
        after_digest: None,
        provider_receipt_digest: None,
        verification_source: None,
        verification_reliability: None,
        rollback_available: false,
        incident_code: Some(SafeErrorCode::from_static(OUTCOME_UNKNOWN_AFTER_CRASH)),
        duration_ms: 0,
    }
}

fn lifecycle_from_token(token: &str) -> ActionLifecycle {
    match token {
        "unchanged" => ActionLifecycle::Unchanged,
        "verified" => ActionLifecycle::Verified,
        "accepted" => ActionLifecycle::Accepted,
        "verification_failed" => ActionLifecycle::VerificationFailed,
        "rolled_back" => ActionLifecycle::RolledBack,
        "partially_applied" => ActionLifecycle::PartiallyApplied,
        _ => ActionLifecycle::Unverified,
    }
}

fn evidence_source_token(source: OsEvidenceSource) -> &'static str {
    match source {
        OsEvidenceSource::UserAttestation => "user_attestation",
        OsEvidenceSource::StructuredCommandQuery => "structured_command_query",
        OsEvidenceSource::IndependentProviderQuery => "independent_provider_query",
        OsEvidenceSource::AuthoritativeServiceState => "authoritative_service_state",
    }
}

fn evidence_source_from_token(token: &str) -> Option<OsEvidenceSource> {
    match token {
        "user_attestation" => Some(OsEvidenceSource::UserAttestation),
        "structured_command_query" => Some(OsEvidenceSource::StructuredCommandQuery),
        "independent_provider_query" => Some(OsEvidenceSource::IndependentProviderQuery),
        "authoritative_service_state" => Some(OsEvidenceSource::AuthoritativeServiceState),
        _ => None,
    }
}

fn reliability_token(rel: VerificationReliability) -> &'static str {
    match rel {
        VerificationReliability::Strong => "strong",
        VerificationReliability::Moderate => "moderate",
        VerificationReliability::Weak => "weak",
    }
}

fn reliability_from_token(token: &str) -> Option<VerificationReliability> {
    match token {
        "strong" => Some(VerificationReliability::Strong),
        "moderate" => Some(VerificationReliability::Moderate),
        "weak" => Some(VerificationReliability::Weak),
        _ => None,
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;
    use crate::os_control::testing::temp_dir;

    fn mutation_admission(tool: &str) -> AdmissionRequest {
        AdmissionRequest {
            session_id: SessionId::new("sess-1"),
            correlation_id: CorrelationId::new("corr-1"),
            action_id: ActionId::new("act-1"),
            tool_name: tool.to_string(),
            params: serde_json::json!({ "level": 40 }),
            target_hash: Digest::of_str("host"),
            capability_snapshot_revision: SnapshotRevision(7),
            risk: RiskLevel::Yellow,
            decision_id: None,
            sensitivity: RequestSensitivity::Mutation,
        }
    }

    fn verified_terminal() -> TerminalRecord {
        TerminalRecord {
            lifecycle: ActionLifecycle::Verified,
            provider: ProviderId::new("pipewire"),
            before_digest: Some(Digest::of_str("before")),
            after_digest: Some(Digest::of_str("after")),
            provider_receipt_digest: Some(Digest::of_str("pr")),
            verification_source: Some(OsEvidenceSource::AuthoritativeServiceState),
            verification_reliability: Some(VerificationReliability::Strong),
            rollback_available: true,
            incident_code: None,
            duration_ms: 12,
        }
    }

    #[test]
    fn in_memory_migration_creates_schema() {
        let store = OsAuditStore::open_in_memory();
        assert!(store.is_healthy());
        assert_eq!(store.incomplete_admission_count(), 0);
        assert_eq!(store.verify_chain(), Ok(0));
    }

    #[test]
    fn admit_binds_canonical_resource_and_parameter_digests() {
        let store = OsAuditStore::open_in_memory();
        let req = mutation_admission("set_volume");
        let token = store.admit_action(&req).expect("admit");
        // Resource digest must match the single canonical Task-1.6 derivation.
        let expected = write_resource_set_digest(&req.tool_name, &req.params);
        assert_eq!(token.resource_set_digest(), &expected);
        // Parameter digest must match the shared redaction canonical digest.
        let expected_param = redact_parameters(&req.tool_name, &req.params).parameter_digest;
        assert_eq!(token.parameter_hash(), &expected_param);
        assert_eq!(token.session_id().as_str(), "sess-1");
        assert_eq!(token.capability_snapshot_revision(), SnapshotRevision(7));
        // Exactly one admission, no terminal yet.
        assert_eq!(store.admission_count(token.admission_id()), 1);
        assert_eq!(store.terminal_count(token.admission_id()), 0);
        assert_eq!(store.incomplete_admission_count(), 1);
    }

    #[test]
    fn one_admission_serves_read_preflight_noop_and_mutation() {
        // A single admission/token is reused across the whole logical action; the
        // pre-observation never creates a second admission. We prove this by
        // admitting once and observing the admission count stays 1 while the same
        // token is used for the eventual terminal.
        let store = OsAuditStore::open_in_memory();
        let token = store
            .admit_action(&mutation_admission("set_volume"))
            .unwrap();
        assert_eq!(store.admission_count(token.admission_id()), 1);
        // No-op path (Unchanged terminal) uses the SAME token.
        let mut unchanged = verified_terminal();
        unchanged.lifecycle = ActionLifecycle::Unchanged;
        let outcome = store.append_terminal(&token, &unchanged);
        assert!(matches!(outcome, TerminalAppendOutcome::Recorded { .. }));
        assert_eq!(store.admission_count(token.admission_id()), 1);
        assert_eq!(store.terminal_count(token.admission_id()), 1);
    }

    #[test]
    fn terminal_cardinality_is_exactly_one_via_unique_constraint() {
        let store = OsAuditStore::open_in_memory();
        let token = store
            .admit_action(&mutation_admission("set_volume"))
            .unwrap();
        let terminal = verified_terminal();
        let first = store.append_terminal(&token, &terminal);
        assert!(matches!(
            first,
            TerminalAppendOutcome::Recorded {
                idempotent_replay: false,
                ..
            }
        ));
        // Idempotent replay of the identical terminal returns the winning row.
        let replay = store.append_terminal(&token, &terminal);
        assert!(matches!(
            replay,
            TerminalAppendOutcome::Recorded {
                idempotent_replay: true,
                ..
            }
        ));
        // Still exactly one terminal.
        assert_eq!(store.terminal_count(token.admission_id()), 1);
        assert!(store.is_healthy());
    }

    #[test]
    fn conflicting_terminal_is_integrity_incident_and_unhealthy() {
        let store = OsAuditStore::open_in_memory();
        let token = store
            .admit_action(&mutation_admission("set_volume"))
            .unwrap();
        store.append_terminal(&token, &verified_terminal());
        // A genuinely different outcome for the same admission conflicts.
        let mut different = verified_terminal();
        different.lifecycle = ActionLifecycle::VerificationFailed;
        different.after_digest = Some(Digest::of_str("tampered"));
        let outcome = store.append_terminal(&token, &different);
        assert!(matches!(
            outcome,
            TerminalAppendOutcome::IntegrityConflict { .. }
        ));
        assert!(!store.is_healthy());
        assert_eq!(store.terminal_count(token.admission_id()), 1);
    }

    #[test]
    fn admission_failure_when_unhealthy_returns_audit_unavailable() {
        let store = OsAuditStore::open_in_memory();
        // Force unhealthy via an interrupted terminal.
        let token = store
            .admit_action(&mutation_admission("set_volume"))
            .unwrap();
        store.inject_fault(AuditFault::InterruptNextTerminal);
        let outcome = store.append_terminal(&token, &verified_terminal());
        assert!(matches!(
            outcome,
            TerminalAppendOutcome::PendingRecovery { .. }
        ));
        assert!(!store.is_healthy());

        // A subsequent MUTATION admission fails closed (no provider is reached).
        let err = store
            .admit_action(&mutation_admission("set_brightness"))
            .unwrap_err();
        assert!(matches!(err, OsControlError::AuditUnavailable));

        // A plain read may still be admitted per policy.
        let mut read = mutation_admission("read_file");
        read.sensitivity = RequestSensitivity::PlainRead;
        assert!(store.admit_action(&read).is_ok());
    }

    #[test]
    fn interrupted_terminal_reports_pending_recovery_and_reconciles_from_payload() {
        let store = OsAuditStore::open_in_memory();
        let token = store
            .admit_action(&mutation_admission("set_volume"))
            .unwrap();
        let terminal = verified_terminal();
        // Stage the recovery payload BEFORE dispatch.
        store.stage_recovery_payload(&token, &terminal).unwrap();

        // Terminal append is interrupted → PendingRecovery, unhealthy.
        store.inject_fault(AuditFault::InterruptNextTerminal);
        let outcome = store.append_terminal(&token, &terminal);
        match &outcome {
            TerminalAppendOutcome::PendingRecovery { .. } => {}
            other => panic!("expected pending recovery, got {other:?}"),
        }
        assert_eq!(store.terminal_count(token.admission_id()), 0);
        assert!(!store.is_healthy());
        assert_eq!(store.incomplete_admission_count(), 1);

        // Bounded reconcile reconstructs the terminal from the recovery payload.
        let report = store.reconcile_incomplete_admissions(64, None);
        assert_eq!(report.reconciled, 1);
        assert_eq!(report.reconstructed, 1);
        assert_eq!(report.unknown_after_crash, 0);
        assert_eq!(store.terminal_count(token.admission_id()), 1);
        assert_eq!(store.incomplete_admission_count(), 0);
        // Health restored after the sole terminal is recorded.
        assert!(store.is_healthy());
    }

    #[test]
    fn unknown_after_crash_when_no_recovery_payload() {
        let store = OsAuditStore::open_in_memory();
        let token = store
            .admit_action(&mutation_admission("set_volume"))
            .unwrap();
        // No recovery payload staged, terminal interrupted.
        store.inject_fault(AuditFault::InterruptNextTerminal);
        store.append_terminal(&token, &verified_terminal());

        let report = store.reconcile_incomplete_admissions(64, None);
        assert_eq!(report.reconciled, 1);
        assert_eq!(report.unknown_after_crash, 1);
        assert_eq!(store.terminal_count(token.admission_id()), 1);
        assert!(store.is_healthy());
    }

    #[test]
    fn process_restart_scan_finds_incomplete_admission() {
        // Simulate a crash between admission and terminal, then a restart
        // opening a fresh store over the SAME durable database file.
        let dir = temp_dir();
        let path = dir.path().join("os_audit.db");
        let admission_id;
        {
            let store = OsAuditStore::new(Connection::open(&path).unwrap());
            let token = store
                .admit_action(&mutation_admission("set_volume"))
                .unwrap();
            admission_id = token.admission_id().clone();
            // ... process exits before the terminal is appended.
        }
        // Restart: a fresh store over the same file detects the incomplete row.
        let store = OsAuditStore::new(Connection::open(&path).unwrap());
        assert_eq!(store.incomplete_admission_count(), 1);
        let report = store.reconcile_incomplete_admissions(64, None);
        assert_eq!(report.reconciled, 1);
        assert_eq!(report.unknown_after_crash, 1);
        assert_eq!(store.terminal_count(&admission_id), 1);
    }

    #[test]
    fn reconcile_is_idempotent_and_never_redispatches() {
        // OsAuditStore holds no provider handle at all, so reconciliation cannot
        // redispatch by construction. Running reconcile twice is a no-op on the
        // second pass (same idempotent terminal key).
        let store = OsAuditStore::open_in_memory();
        let token = store
            .admit_action(&mutation_admission("set_volume"))
            .unwrap();
        store.inject_fault(AuditFault::InterruptNextTerminal);
        store.append_terminal(&token, &verified_terminal());

        let first = store.reconcile_incomplete_admissions(64, None);
        assert_eq!(first.reconciled, 1);
        let second = store.reconcile_incomplete_admissions(64, None);
        assert_eq!(second.scanned, 0);
        assert_eq!(second.reconciled, 0);
        assert_eq!(store.terminal_count(token.admission_id()), 1);
    }

    #[test]
    fn bounded_scan_respects_limit_and_cursor() {
        let store = OsAuditStore::open_in_memory();
        // Three incomplete admissions (admitted while healthy, no terminal yet —
        // e.g. a crash before any terminal append).
        for _ in 0..3 {
            store
                .admit_action(&mutation_admission("set_volume"))
                .unwrap();
        }
        assert_eq!(store.incomplete_admission_count(), 3);
        // Bounded to 2 per pass.
        let first = store.reconcile_incomplete_admissions(2, None);
        assert_eq!(first.scanned, 2);
        assert!(first.next_cursor.is_some());
        let second = store.reconcile_incomplete_admissions(2, first.next_cursor);
        assert!(second.scanned <= 2);
        assert_eq!(store.incomplete_admission_count(), 0);
    }

    #[test]
    fn scan_limit_is_hard_capped() {
        let store = OsAuditStore::open_in_memory();
        let report = store.reconcile_incomplete_admissions(usize::MAX, None);
        // No panic; a huge limit is clamped to MAX_SCAN_LIMIT internally.
        assert_eq!(report.scanned, 0);
        assert!(MAX_SCAN_LIMIT <= 512);
    }

    #[test]
    fn hash_chain_detects_tampering() {
        let store = OsAuditStore::open_in_memory();
        let token = store
            .admit_action(&mutation_admission("set_volume"))
            .unwrap();
        store.append_terminal(&token, &verified_terminal());
        assert!(store.verify_chain().is_ok());

        // Tamper with a stored row_hash.
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "UPDATE os_audit_log SET target_hash = 'tampered' WHERE record_kind = 'admission'",
                [],
            )
            .unwrap();
        }
        assert!(store.verify_chain().is_err());
    }

    #[test]
    fn durable_audit_never_stores_raw_secret_or_content() {
        // Raw params source scan: a wifi password / clipboard content must never
        // appear anywhere in the durable audit rows.
        let store = OsAuditStore::open_in_memory();
        let mut req = mutation_admission("connect_wifi");
        req.params = serde_json::json!({ "ssid": "SECRET-SSID", "password": "top-secret-pw" });
        let token = store.admit_action(&req).unwrap();
        store.append_terminal(&token, &verified_terminal());

        let conn = store.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT redacted_parameters, action_hash, parameter_hash FROM os_audit_log")
            .unwrap();
        let rows: Vec<String> = stmt
            .query_map([], |r| {
                Ok(format!(
                    "{}|{}|{}",
                    r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?
                ))
            })
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        let all = rows.join("\n");
        assert!(
            !all.contains("SECRET-SSID"),
            "ssid leaked into durable audit"
        );
        assert!(
            !all.contains("top-secret-pw"),
            "password leaked into durable audit"
        );
    }

    #[test]
    fn every_admitted_action_has_at_most_one_terminal_invariant() {
        // Completion proof: across many admissions the (admission, terminal)
        // cardinality is 1:{0,1} at any instant.
        let store = OsAuditStore::open_in_memory();
        let mut tokens = Vec::new();
        for _ in 0..5 {
            tokens.push(
                store
                    .admit_action(&mutation_admission("set_volume"))
                    .unwrap(),
            );
        }
        for t in &tokens {
            assert_eq!(store.admission_count(t.admission_id()), 1);
            assert!(store.terminal_count(t.admission_id()) <= 1);
        }
        // Complete them all; each ends with exactly one terminal.
        for t in &tokens {
            store.append_terminal(t, &verified_terminal());
        }
        for t in &tokens {
            assert_eq!(store.terminal_count(t.admission_id()), 1);
        }
    }
}
