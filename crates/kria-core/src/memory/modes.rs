//! Memory modes, enforced at the write gate (memory-upgrade design §23, ADR-013).
//!
//! The mode decision table is **deterministic** and applied on the Write Policy
//! fast path (L3): it is impossible to bypass. Mode is per-session and
//! user-switchable mid-session; a switch emits a `mode_switched` boundary event
//! (handled by the write policy), and the current mode is always surfaced to the
//! UI.

use dashmap::DashMap;
use uuid::Uuid;

use crate::memory::types::{MemoryMode, RejectReason};

/// Context the mode gate needs to decide a write.
#[derive(Clone, Copy, Debug)]
pub struct ModeWriteContext {
    /// Whether the candidate is personal-scoped (rejected in Workspace mode).
    pub is_personal_scope: bool,
    /// Whether the candidate is a library-ingestion write (the only kind
    /// allowed in Library-only mode).
    pub is_library_ingest: bool,
}

/// The fast-path decision for a write under a given mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModeWriteDecision {
    /// Persist normally (policy-governed).
    Allow,
    /// Persist but tag session-scoped (purged at session end — Temporary mode).
    AllowSessionScoped,
    /// Do not persist; return this reject reason.
    Reject(RejectReason),
}

/// Evaluate the mode decision table (design §23). Deterministic, LLM-free.
pub fn evaluate(mode: &MemoryMode, ctx: &ModeWriteContext) -> ModeWriteDecision {
    use MemoryMode::*;
    match mode {
        Permanent | Developer | Research => ModeWriteDecision::Allow,
        // Benchmark writes go to an isolated namespace (assigned by the write
        // policy); the gate itself allows them.
        Benchmark => ModeWriteDecision::Allow,
        // Safe mode allows deterministic writes; the "no LLM" constraint is
        // enforced in the slow path, not the gate.
        Safe => ModeWriteDecision::Allow,
        Temporary => ModeWriteDecision::AllowSessionScoped,
        Incognito => ModeWriteDecision::Reject(RejectReason::Mode(Incognito)),
        ReadOnly => ModeWriteDecision::Reject(RejectReason::Mode(ReadOnly)),
        Guest => ModeWriteDecision::Reject(RejectReason::Mode(Guest)),
        Workspace => {
            if ctx.is_personal_scope {
                ModeWriteDecision::Reject(RejectReason::NamespaceViolation)
            } else {
                ModeWriteDecision::Allow
            }
        }
        LibraryOnly => {
            if ctx.is_library_ingest {
                ModeWriteDecision::Allow
            } else {
                ModeWriteDecision::Reject(RejectReason::Mode(LibraryOnly))
            }
        }
        // Fail-safe: an unknown/forward-compat mode rejects writes rather than
        // silently persisting under an unrecognized policy.
        Other(name) => ModeWriteDecision::Reject(RejectReason::Mode(Other(name.clone()))),
    }
}

/// Per-session mode registry. In-RAM authoritative state (design §27 caching):
/// the current mode is transient and re-derived from the session row on resume.
#[derive(Debug)]
pub struct ModeManager {
    modes: DashMap<Uuid, MemoryMode>,
    default: MemoryMode,
}

impl ModeManager {
    pub fn new(default: MemoryMode) -> Self {
        Self {
            modes: DashMap::new(),
            default,
        }
    }

    /// Current mode for a session (defaults to the configured default).
    pub fn current(&self, session_id: Uuid) -> MemoryMode {
        self.modes
            .get(&session_id)
            .map(|m| m.clone())
            .unwrap_or_else(|| self.default.clone())
    }

    /// Switch a session's mode. Returns the previous mode (for the boundary
    /// event the caller emits).
    pub fn set_mode(&self, session_id: Uuid, mode: MemoryMode) -> MemoryMode {
        let prev = self.current(session_id);
        self.modes.insert(session_id, mode);
        prev
    }

    /// Whether writes in this session's mode are session-scoped (Temporary) so
    /// they can be purged at session end.
    pub fn is_session_scoped(&self, session_id: Uuid) -> bool {
        matches!(self.current(session_id), MemoryMode::Temporary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(personal: bool, library: bool) -> ModeWriteContext {
        ModeWriteContext {
            is_personal_scope: personal,
            is_library_ingest: library,
        }
    }

    #[test]
    fn incognito_and_readonly_reject_all() {
        assert!(matches!(
            evaluate(&MemoryMode::Incognito, &ctx(false, false)),
            ModeWriteDecision::Reject(RejectReason::Mode(MemoryMode::Incognito))
        ));
        assert!(matches!(
            evaluate(&MemoryMode::ReadOnly, &ctx(false, false)),
            ModeWriteDecision::Reject(_)
        ));
    }

    #[test]
    fn workspace_rejects_personal_scope() {
        assert_eq!(
            evaluate(&MemoryMode::Workspace, &ctx(true, false)),
            ModeWriteDecision::Reject(RejectReason::NamespaceViolation)
        );
        assert_eq!(
            evaluate(&MemoryMode::Workspace, &ctx(false, false)),
            ModeWriteDecision::Allow
        );
    }

    #[test]
    fn library_only_allows_only_ingest() {
        assert_eq!(
            evaluate(&MemoryMode::LibraryOnly, &ctx(false, true)),
            ModeWriteDecision::Allow
        );
        assert!(matches!(
            evaluate(&MemoryMode::LibraryOnly, &ctx(false, false)),
            ModeWriteDecision::Reject(_)
        ));
    }

    #[test]
    fn temporary_is_session_scoped() {
        assert_eq!(
            evaluate(&MemoryMode::Temporary, &ctx(false, false)),
            ModeWriteDecision::AllowSessionScoped
        );
    }

    #[test]
    fn unknown_mode_fails_safe() {
        assert!(matches!(
            evaluate(&MemoryMode::Other("weird".into()), &ctx(false, false)),
            ModeWriteDecision::Reject(_)
        ));
    }

    #[test]
    fn manager_tracks_and_switches() {
        let mgr = ModeManager::new(MemoryMode::Permanent);
        let s = Uuid::now_v7();
        assert_eq!(mgr.current(s), MemoryMode::Permanent);
        let prev = mgr.set_mode(s, MemoryMode::Incognito);
        assert_eq!(prev, MemoryMode::Permanent);
        assert_eq!(mgr.current(s), MemoryMode::Incognito);
        assert!(!mgr.is_session_scoped(s));
        mgr.set_mode(s, MemoryMode::Temporary);
        assert!(mgr.is_session_scoped(s));
    }
}
