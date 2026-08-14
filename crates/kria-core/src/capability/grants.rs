//! Durable, scoped, revocable permission grants for the CPP.
//!
//! A [`GrantStore`] persists the user's/policy's approval decisions so an
//! elevated capability is prompted only when genuinely necessary and can be
//! reused within its granted scope — and revoked at any time. It is
//! provider-neutral: grants are keyed by `(provider_id, capability_id)` and a
//! **set of granted effect classes**, so reuse honors *narrowing* (a request
//! using a subset of granted effects is covered) while *widening* (a new effect
//! class) forces fresh approval — the monotonicity property the permission
//! engine relies on.
//!
//! Storage is SQLite (durable across restarts, R6.4). The table is self-managed
//! and additive; a `:memory:` store is used in tests. This is the single CPP
//! grant owner (the legacy `openclaw::perm` grant store is removed at Milestone
//! 11 once CPP is the sole path).

use std::sync::Mutex;

use chrono::{DateTime, Utc};
use rusqlite::Connection;

use super::error::CapError;

/// The scope at which a grant applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    /// One-shot: consumed after a single use.
    Once,
    /// For the current chat session.
    Session,
    /// For the current workspace.
    Workspace,
    /// Standing until explicitly revoked.
    Persistent,
    /// Pre-authorized by policy (covers even AlwaysAsk capabilities).
    Silent,
}

impl ScopeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Once => "once",
            Self::Session => "session",
            Self::Workspace => "workspace",
            Self::Persistent => "persistent",
            Self::Silent => "silent",
        }
    }
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "once" => Some(Self::Once),
            "session" => Some(Self::Session),
            "workspace" => Some(Self::Workspace),
            "persistent" => Some(Self::Persistent),
            "silent" => Some(Self::Silent),
            _ => None,
        }
    }
}

/// Allow or deny.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantDecision {
    Allow,
    Deny,
}

impl GrantDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }
}

/// A persisted grant row.
#[derive(Debug, Clone)]
pub struct ScopedGrant {
    pub grant_id: String,
    pub provider_id: String,
    pub capability_id: String,
    pub scope_kind: ScopeKind,
    pub scope_key: Option<String>,
    /// Sorted set of granted effect-class strings.
    pub effects: Vec<String>,
    pub decision: GrantDecision,
    pub granted_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked: bool,
}

impl ScopedGrant {
    /// Whether this grant is active at `now` (not revoked, not expired).
    pub fn is_active_at(&self, now: DateTime<Utc>) -> bool {
        if self.revoked {
            return false;
        }
        match self.expires_at {
            Some(exp) => now < exp,
            None => true,
        }
    }

    /// Whether this grant *covers* a request for `requested` effect classes:
    /// every requested class is within the granted set (narrowing is covered;
    /// widening is not).
    pub fn covers(&self, requested: &[String]) -> bool {
        requested
            .iter()
            .all(|r| self.effects.iter().any(|g| g == r))
    }
}

/// Durable grant store backed by SQLite.
pub struct GrantStore {
    conn: Mutex<Connection>,
}

impl GrantStore {
    /// Open (or create) a durable grant store at `path`.
    pub fn open(path: &std::path::Path) -> Result<Self, CapError> {
        let conn =
            Connection::open(path).map_err(|e| CapError::Io(format!("grant db open: {e}")))?;
        Self::from_conn(conn)
    }

    /// An in-memory grant store (tests).
    pub fn in_memory() -> Result<Self, CapError> {
        let conn =
            Connection::open_in_memory().map_err(|e| CapError::Io(format!("grant db mem: {e}")))?;
        Self::from_conn(conn)
    }

    fn from_conn(conn: Connection) -> Result<Self, CapError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cpp_grants (
                grant_id      TEXT PRIMARY KEY,
                provider_id   TEXT NOT NULL,
                capability_id TEXT NOT NULL,
                scope_kind    TEXT NOT NULL,
                scope_key     TEXT,
                effects_json  TEXT NOT NULL,
                decision      TEXT NOT NULL,
                granted_at    TEXT NOT NULL,
                expires_at    TEXT,
                revoked       INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_cpp_grants_key
                ON cpp_grants(provider_id, capability_id);",
        )
        .map_err(|e| CapError::Io(format!("grant db migrate: {e}")))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Persist a grant.
    ///
    /// Native host-OS effects are refused (design §2.1, OSC-001): the extension
    /// grant store must never durably authorize a native OS mutation. Such
    /// authority exists only as an `ExecutionGate`-minted `OsActionGrant`, never
    /// as a persisted extension grant.
    pub fn insert(&self, grant: &ScopedGrant) -> Result<(), CapError> {
        if crate::agent::os_action_authority::effects_request_native_os(&grant.effects) {
            return Err(CapError::Permission(format!(
                "refusing to persist grant {}: native host-OS effects cannot be authorized by the \
                 extension grant store",
                grant.grant_id
            )));
        }
        let conn = self
            .conn
            .lock()
            .map_err(|e| CapError::Io(format!("grant lock: {e}")))?;
        let mut effects = grant.effects.clone();
        effects.sort();
        conn.execute(
            "INSERT OR REPLACE INTO cpp_grants
             (grant_id, provider_id, capability_id, scope_kind, scope_key, effects_json, decision, granted_at, expires_at, revoked)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            rusqlite::params![
                grant.grant_id,
                grant.provider_id,
                grant.capability_id,
                grant.scope_kind.as_str(),
                grant.scope_key,
                serde_json::to_string(&effects).unwrap_or_else(|_| "[]".into()),
                grant.decision.as_str(),
                grant.granted_at.to_rfc3339(),
                grant.expires_at.map(|e| e.to_rfc3339()),
                grant.revoked as i64,
            ],
        )
        .map_err(|e| CapError::Io(format!("grant insert: {e}")))?;
        Ok(())
    }

    /// Find an active grant that covers the requested effects at the given scope.
    /// Returns the first covering Allow, or a matching standing Deny, else None.
    pub fn find_covering(
        &self,
        provider_id: &str,
        capability_id: &str,
        scope_kind: ScopeKind,
        scope_key: Option<&str>,
        requested_effects: &[String],
        now: DateTime<Utc>,
    ) -> Result<Option<ScopedGrant>, CapError> {
        let candidates = self.grants_for(provider_id, capability_id)?;
        // Prefer an explicit standing Deny at this scope (safety-first).
        for g in &candidates {
            if g.scope_kind == scope_kind
                && g.scope_key.as_deref() == scope_key
                && g.decision == GrantDecision::Deny
                && g.is_active_at(now)
                && g.covers(requested_effects)
            {
                return Ok(Some(g.clone()));
            }
        }
        for g in &candidates {
            if g.scope_kind == scope_kind
                && g.scope_key.as_deref() == scope_key
                && g.decision == GrantDecision::Allow
                && g.is_active_at(now)
                && g.covers(requested_effects)
            {
                return Ok(Some(g.clone()));
            }
        }
        Ok(None)
    }

    /// A Silent (policy) grant covering the requested effects, if any.
    pub fn find_silent(
        &self,
        provider_id: &str,
        capability_id: &str,
        requested_effects: &[String],
        now: DateTime<Utc>,
    ) -> Result<Option<ScopedGrant>, CapError> {
        let candidates = self.grants_for(provider_id, capability_id)?;
        Ok(candidates.into_iter().find(|g| {
            g.scope_kind == ScopeKind::Silent
                && g.decision == GrantDecision::Allow
                && g.is_active_at(now)
                && g.covers(requested_effects)
        }))
    }

    /// All grants for a capability (any scope/state).
    pub fn grants_for(
        &self,
        provider_id: &str,
        capability_id: &str,
    ) -> Result<Vec<ScopedGrant>, CapError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CapError::Io(format!("grant lock: {e}")))?;
        let mut stmt = conn
            .prepare(
                "SELECT grant_id, provider_id, capability_id, scope_kind, scope_key, effects_json, decision, granted_at, expires_at, revoked
                 FROM cpp_grants WHERE provider_id=?1 AND capability_id=?2",
            )
            .map_err(|e| CapError::Io(format!("grant query: {e}")))?;
        let rows = stmt
            .query_map(
                rusqlite::params![provider_id, capability_id],
                Self::row_to_grant,
            )
            .map_err(|e| CapError::Io(format!("grant map: {e}")))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| CapError::Io(format!("grant row: {e}")))?);
        }
        Ok(out)
    }

    /// Revoke a grant by id. Returns whether a row was affected.
    pub fn revoke(&self, grant_id: &str) -> Result<bool, CapError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CapError::Io(format!("grant lock: {e}")))?;
        let n = conn
            .execute(
                "UPDATE cpp_grants SET revoked=1 WHERE grant_id=?1",
                rusqlite::params![grant_id],
            )
            .map_err(|e| CapError::Io(format!("grant revoke: {e}")))?;
        Ok(n > 0)
    }

    /// All active (non-revoked, non-expired) grants across all capabilities —
    /// for the desktop grant list.
    pub fn active_grants(&self, now: DateTime<Utc>) -> Result<Vec<ScopedGrant>, CapError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CapError::Io(format!("grant lock: {e}")))?;
        let mut stmt = conn
            .prepare(
                "SELECT grant_id, provider_id, capability_id, scope_kind, scope_key, effects_json, decision, granted_at, expires_at, revoked
                 FROM cpp_grants WHERE revoked=0",
            )
            .map_err(|e| CapError::Io(format!("grant query: {e}")))?;
        let rows = stmt
            .query_map([], Self::row_to_grant)
            .map_err(|e| CapError::Io(format!("grant map: {e}")))?;
        let mut out = Vec::new();
        for r in rows {
            let g = r.map_err(|e| CapError::Io(format!("grant row: {e}")))?;
            if g.is_active_at(now) {
                out.push(g);
            }
        }
        Ok(out)
    }

    fn row_to_grant(row: &rusqlite::Row) -> rusqlite::Result<ScopedGrant> {
        let effects_json: String = row.get(5)?;
        let granted_at: String = row.get(7)?;
        let expires_at: Option<String> = row.get(8)?;
        let scope_str: String = row.get(3)?;
        let decision_str: String = row.get(6)?;
        Ok(ScopedGrant {
            grant_id: row.get(0)?,
            provider_id: row.get(1)?,
            capability_id: row.get(2)?,
            scope_kind: ScopeKind::from_name(&scope_str).unwrap_or(ScopeKind::Once),
            scope_key: row.get(4)?,
            effects: serde_json::from_str(&effects_json).unwrap_or_default(),
            decision: if decision_str == "deny" {
                GrantDecision::Deny
            } else {
                GrantDecision::Allow
            },
            granted_at: DateTime::parse_from_rfc3339(&granted_at)
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            expires_at: expires_at
                .and_then(|e| DateTime::parse_from_rfc3339(&e).ok())
                .map(|d| d.with_timezone(&Utc)),
            revoked: row.get::<_, i64>(9)? != 0,
        })
    }
}
