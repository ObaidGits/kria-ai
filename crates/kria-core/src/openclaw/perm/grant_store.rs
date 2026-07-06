//! `GrantStore` — persistence layer over `capability_grants_scoped` (ICP §7.4 / §8.7).
//!
//! The `GrantStore` is a thin, deny-by-default persistence layer over the
//! `capability_grants_scoped` table that task 2.1 appended to the frozen
//! `MIGRATIONS` pipeline in [`crate::openclaw::registry`]. It lives inside the
//! SAME `skills.db` as [`ProductionSkillRegistry`] — there is **no second
//! database**. Grants are keyed by `skill_id` (served by `idx_grants_skill`) and
//! optionally partitioned by workspace via `scope_key`.
//!
//! # Deny-by-default
//!
//! Every "active grant" query filters out `revoked = 1` rows and rows whose
//! `expires_at` is in the past. An expired or revoked grant can therefore never
//! be reused — the permission engine (task 11.2) will treat the absence of an
//! active grant as "must (re)approve".
//!
//! # Source of truth
//!
//! `ProductionSkillRegistry` remains the single source of truth. This table is
//! additive persistence; the table/index are created only by the frozen
//! registry migrations (migration 5). This store never issues DDL and never
//! drops or rewrites the schema.
//!
//! [`ProductionSkillRegistry`]: crate::openclaw::registry::ProductionSkillRegistry

use crate::openclaw::cil::CilError;
use crate::safety::RiskLevel;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, Row};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// The lifetime/scope of a persisted grant (design §7.4 `scope_kind`).
///
/// Open string (de)serialization keeps the on-disk column stable and forward
/// compatible; an unknown string parses to an explicit error rather than being
/// silently coerced (honesty invariant).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ScopeKind {
    /// Explicit standing denial — the user said "never" for this skill+caps.
    Never,
    /// Approved for a single use; not reusable afterwards.
    Once,
    /// Approved for the current chat session (`scope_key` = session id).
    Session,
    /// Approved for the current workspace (`scope_key` = workspace id).
    Workspace,
    /// Standing approval until explicitly revoked.
    Persistent,
    /// Pre-authorized by policy; no prompt (`scope_key` typically null).
    Silent,
}

impl ScopeKind {
    /// Stable lower-case string used in the `scope_kind` column.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::Once => "once",
            Self::Session => "session",
            Self::Workspace => "workspace",
            Self::Persistent => "persistent",
            Self::Silent => "silent",
        }
    }

    /// Parse a `scope_kind` column value. Unknown values are surfaced as a
    /// persistence error rather than silently defaulted (deny-by-default).
    pub fn from_str(s: &str) -> Result<Self, CilError> {
        match s {
            "never" => Ok(Self::Never),
            "once" => Ok(Self::Once),
            "session" => Ok(Self::Session),
            "workspace" => Ok(Self::Workspace),
            "persistent" => Ok(Self::Persistent),
            "silent" => Ok(Self::Silent),
            other => Err(CilError::Io(format!("unknown scope_kind {other:?}"))),
        }
    }

    /// Whether this scope is partitioned by a `scope_key` (session/workspace id).
    pub fn is_partitioned(&self) -> bool {
        matches!(self, Self::Session | Self::Workspace)
    }
}

/// The decision recorded for a grant (design §7.4 `decision`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GrantDecision {
    /// The capability set was allowed under this scope.
    Allow,
    /// The capability set was explicitly denied under this scope.
    Deny,
}

impl GrantDecision {
    /// Stable lower-case string used in the `decision` column.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }

    /// Parse a `decision` column value.
    pub fn from_str(s: &str) -> Result<Self, CilError> {
        match s {
            "allow" => Ok(Self::Allow),
            "deny" => Ok(Self::Deny),
            other => Err(CilError::Io(format!("unknown grant decision {other:?}"))),
        }
    }
}

/// One row of `capability_grants_scoped`.
///
/// `granted_at`/`expires_at` are stored as RFC3339 like the rest of the
/// registry. `expires_at == None` means no expiry; `revoked` mirrors the
/// integer flag column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedGrant {
    /// Primary key — a caller-supplied unique id (e.g. UUID).
    pub grant_id: String,
    /// The skill this grant applies to (indexed by `idx_grants_skill`).
    pub skill_id: String,
    /// The lifetime/scope of the grant.
    pub scope_kind: ScopeKind,
    /// Session/workspace partition key; `None` for unpartitioned scopes.
    pub scope_key: Option<String>,
    /// The `ApprovalCache::compute_hash` payload of the approved capability set.
    pub caps_hash: String,
    /// The risk level classified for the grant.
    pub risk: RiskLevel,
    /// Whether the grant allows or denies.
    pub decision: GrantDecision,
    /// When the grant was created.
    pub granted_at: DateTime<Utc>,
    /// Optional expiry; `None` = never expires.
    pub expires_at: Option<DateTime<Utc>>,
    /// Whether the grant has been revoked.
    pub revoked: bool,
}

impl ScopedGrant {
    /// Is this grant currently active — neither revoked nor expired at `now`?
    ///
    /// Deny-by-default: this is the single predicate all "active" reads agree
    /// with, so an expired/revoked grant is never treated as usable.
    pub fn is_active_at(&self, now: DateTime<Utc>) -> bool {
        if self.revoked {
            return false;
        }
        match self.expires_at {
            Some(exp) => exp > now,
            None => true,
        }
    }
}

/// Persistent, scoped, revocable permission grants over `skills.db`.
///
/// Holds an `Arc<Mutex<Connection>>` to `skills.db` — the SAME database as
/// [`ProductionSkillRegistry`], following the registry's own connection pattern.
/// Use [`GrantStore::from_shared_connection`] to share the registry's live
/// connection, or [`GrantStore::open`] to open an additional connection to the
/// same file (WAL allows concurrent readers).
///
/// [`ProductionSkillRegistry`]: crate::openclaw::registry::ProductionSkillRegistry
pub struct GrantStore {
    db: Arc<Mutex<Connection>>,
}

impl GrantStore {
    /// Open an additional connection to `skills.db`.
    ///
    /// This opens the SAME database file the registry uses (never a second
    /// database) and enables WAL for concurrent reads, matching
    /// `ProductionSkillRegistry::new`. The `capability_grants_scoped` table and
    /// `idx_grants_skill` index are created by the frozen registry migrations
    /// (migration 5); this constructor issues no DDL. Construct the registry
    /// first (or otherwise run migrations) so the table exists.
    pub fn open(db_path: &Path) -> Result<Self, CilError> {
        let conn = Connection::open(db_path)
            .map_err(|e| CilError::Io(format!("open skills.db for grant store: {e}")))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| CilError::Io(format!("enable WAL for grant store: {e}")))?;
        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    /// Build a `GrantStore` over an already-open, shared `skills.db` connection.
    ///
    /// Preferred when the registry's connection is available: it keeps every
    /// writer on one connection and one source of truth.
    pub fn from_shared_connection(db: Arc<Mutex<Connection>>) -> Self {
        Self { db }
    }

    /// Persist a grant (insert or overwrite by `grant_id`).
    ///
    /// Uses `INSERT OR REPLACE` so re-persisting the same `grant_id` (e.g. after
    /// a scope refresh) is idempotent.
    pub fn insert(&self, grant: &ScopedGrant) -> Result<(), CilError> {
        let db = self
            .db
            .lock()
            .map_err(|_| CilError::Io("grant store connection poisoned".into()))?;
        db.execute(
            "INSERT OR REPLACE INTO capability_grants_scoped (
                grant_id, skill_id, scope_kind, scope_key, caps_hash,
                risk, decision, granted_at, expires_at, revoked
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                grant.grant_id,
                grant.skill_id,
                grant.scope_kind.as_str(),
                grant.scope_key,
                grant.caps_hash,
                grant.risk.as_str(),
                grant.decision.as_str(),
                grant.granted_at.to_rfc3339(),
                grant.expires_at.map(|t| t.to_rfc3339()),
                grant.revoked as i64,
            ],
        )
        .map_err(|e| CilError::Io(format!("persist grant {}: {e}", grant.grant_id)))?;
        Ok(())
    }

    /// Fetch a single grant by id (regardless of active state).
    pub fn get(&self, grant_id: &str) -> Result<Option<ScopedGrant>, CilError> {
        let db = self
            .db
            .lock()
            .map_err(|_| CilError::Io("grant store connection poisoned".into()))?;
        let mut stmt = db
            .prepare(
                "SELECT grant_id, skill_id, scope_kind, scope_key, caps_hash,
                        risk, decision, granted_at, expires_at, revoked
                 FROM capability_grants_scoped WHERE grant_id = ?1",
            )
            .map_err(|e| CilError::Io(format!("prepare grant get: {e}")))?;
        let mut rows = stmt
            .query(params![grant_id])
            .map_err(|e| CilError::Io(format!("query grant get: {e}")))?;
        match rows
            .next()
            .map_err(|e| CilError::Io(format!("read grant row: {e}")))?
        {
            Some(row) => Ok(Some(row_to_grant(row)?)),
            None => Ok(None),
        }
    }

    /// All currently-active (non-revoked, non-expired at `now`) grants for a
    /// skill, ordered newest-first. Backed by `idx_grants_skill`.
    pub fn active_grants_for_skill(
        &self,
        skill_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<ScopedGrant>, CilError> {
        let db = self
            .db
            .lock()
            .map_err(|_| CilError::Io("grant store connection poisoned".into()))?;
        let now_rfc = now.to_rfc3339();
        let mut stmt = db
            .prepare(
                "SELECT grant_id, skill_id, scope_kind, scope_key, caps_hash,
                        risk, decision, granted_at, expires_at, revoked
                 FROM capability_grants_scoped
                 WHERE skill_id = ?1
                   AND revoked = 0
                   AND (expires_at IS NULL OR expires_at > ?2)
                 ORDER BY granted_at DESC",
            )
            .map_err(|e| CilError::Io(format!("prepare active grants: {e}")))?;
        let rows = stmt
            .query_map(params![skill_id, now_rfc], |row| Ok(row_to_grant(row)))
            .map_err(|e| CilError::Io(format!("query active grants: {e}")))?;
        collect_grants(rows)
    }

    /// Active grants for a skill within a specific workspace partition.
    ///
    /// Returns workspace-scoped grants whose `scope_key` matches `workspace_id`.
    /// Supports workspace partitioning per the task requirement.
    pub fn active_grants_for_workspace(
        &self,
        skill_id: &str,
        workspace_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<ScopedGrant>, CilError> {
        let db = self
            .db
            .lock()
            .map_err(|_| CilError::Io("grant store connection poisoned".into()))?;
        let now_rfc = now.to_rfc3339();
        let mut stmt = db
            .prepare(
                "SELECT grant_id, skill_id, scope_kind, scope_key, caps_hash,
                        risk, decision, granted_at, expires_at, revoked
                 FROM capability_grants_scoped
                 WHERE skill_id = ?1
                   AND scope_kind = 'workspace'
                   AND scope_key = ?2
                   AND revoked = 0
                   AND (expires_at IS NULL OR expires_at > ?3)
                 ORDER BY granted_at DESC",
            )
            .map_err(|e| CilError::Io(format!("prepare workspace grants: {e}")))?;
        let rows = stmt
            .query_map(params![skill_id, workspace_id, now_rfc], |row| {
                Ok(row_to_grant(row))
            })
            .map_err(|e| CilError::Io(format!("query workspace grants: {e}")))?;
        collect_grants(rows)
    }

    /// Look up a single active grant reusable for `skill_id` under an exact
    /// `(scope_kind, scope_key, caps_hash)` match — the reuse primitive the
    /// permission engine (task 11.2) consults before prompting.
    ///
    /// Deny-by-default: only a non-revoked, non-expired grant is returned. The
    /// newest matching grant wins.
    pub fn find_reusable(
        &self,
        skill_id: &str,
        scope_kind: ScopeKind,
        scope_key: Option<&str>,
        caps_hash: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<ScopedGrant>, CilError> {
        let db = self
            .db
            .lock()
            .map_err(|_| CilError::Io("grant store connection poisoned".into()))?;
        let now_rfc = now.to_rfc3339();
        // `scope_key IS ?` matches NULL to NULL and value to value uniformly.
        let mut stmt = db
            .prepare(
                "SELECT grant_id, skill_id, scope_kind, scope_key, caps_hash,
                        risk, decision, granted_at, expires_at, revoked
                 FROM capability_grants_scoped
                 WHERE skill_id = ?1
                   AND scope_kind = ?2
                   AND scope_key IS ?3
                   AND caps_hash = ?4
                   AND revoked = 0
                   AND (expires_at IS NULL OR expires_at > ?5)
                 ORDER BY granted_at DESC
                 LIMIT 1",
            )
            .map_err(|e| CilError::Io(format!("prepare reusable grant: {e}")))?;
        let mut rows = stmt
            .query(params![
                skill_id,
                scope_kind.as_str(),
                scope_key,
                caps_hash,
                now_rfc
            ])
            .map_err(|e| CilError::Io(format!("query reusable grant: {e}")))?;
        match rows
            .next()
            .map_err(|e| CilError::Io(format!("read reusable row: {e}")))?
        {
            Some(row) => Ok(Some(row_to_grant(row)?)),
            None => Ok(None),
        }
    }

    /// Mark a grant revoked (`revoked = 1`), forcing fresh approval on next use.
    ///
    /// Returns `true` if a matching grant existed and was updated, `false` if no
    /// such `grant_id` was found. Idempotent — revoking an already-revoked grant
    /// still reports `true` (a row matched).
    pub fn revoke(&self, grant_id: &str) -> Result<bool, CilError> {
        let db = self
            .db
            .lock()
            .map_err(|_| CilError::Io("grant store connection poisoned".into()))?;
        let changed = db
            .execute(
                "UPDATE capability_grants_scoped SET revoked = 1 WHERE grant_id = ?1",
                params![grant_id],
            )
            .map_err(|e| CilError::Io(format!("revoke grant {grant_id}: {e}")))?;
        Ok(changed > 0)
    }
}

/// Map a result-set row to a [`ScopedGrant`], surfacing parse failures as
/// `CilError` rather than swallowing them.
fn row_to_grant(row: &Row) -> Result<ScopedGrant, CilError> {
    let scope_kind_s: String = row
        .get(2)
        .map_err(|e| CilError::Io(format!("read scope_kind: {e}")))?;
    let scope_key: Option<String> = row
        .get(3)
        .map_err(|e| CilError::Io(format!("read scope_key: {e}")))?;
    let caps_hash: String = row
        .get(4)
        .map_err(|e| CilError::Io(format!("read caps_hash: {e}")))?;
    let risk_s: String = row
        .get(5)
        .map_err(|e| CilError::Io(format!("read risk: {e}")))?;
    let decision_s: String = row
        .get(6)
        .map_err(|e| CilError::Io(format!("read decision: {e}")))?;
    let granted_at_s: String = row
        .get(7)
        .map_err(|e| CilError::Io(format!("read granted_at: {e}")))?;
    let expires_at_s: Option<String> = row
        .get(8)
        .map_err(|e| CilError::Io(format!("read expires_at: {e}")))?;
    let revoked_i: i64 = row
        .get(9)
        .map_err(|e| CilError::Io(format!("read revoked: {e}")))?;

    Ok(ScopedGrant {
        grant_id: row
            .get(0)
            .map_err(|e| CilError::Io(format!("read grant_id: {e}")))?,
        skill_id: row
            .get(1)
            .map_err(|e| CilError::Io(format!("read skill_id: {e}")))?,
        scope_kind: ScopeKind::from_str(&scope_kind_s)?,
        scope_key,
        caps_hash,
        risk: risk_from_str(&risk_s)?,
        decision: GrantDecision::from_str(&decision_s)?,
        granted_at: parse_rfc3339(&granted_at_s)?,
        expires_at: match expires_at_s {
            Some(s) => Some(parse_rfc3339(&s)?),
            None => None,
        },
        revoked: revoked_i != 0,
    })
}

/// Drain a `query_map` iterator of `Result<Result<ScopedGrant, CilError>, _>`
/// into a `Vec`, propagating the first error of either layer.
fn collect_grants<I>(rows: I) -> Result<Vec<ScopedGrant>, CilError>
where
    I: Iterator<Item = Result<Result<ScopedGrant, CilError>, rusqlite::Error>>,
{
    let mut out = Vec::new();
    for row in rows {
        let grant = row.map_err(|e| CilError::Io(format!("iterate grants: {e}")))??;
        out.push(grant);
    }
    Ok(out)
}

/// Parse the `risk` column back into a [`RiskLevel`] (mirrors `RiskLevel::as_str`).
fn risk_from_str(s: &str) -> Result<RiskLevel, CilError> {
    match s {
        "GREEN" => Ok(RiskLevel::Green),
        "YELLOW" => Ok(RiskLevel::Yellow),
        "RED" => Ok(RiskLevel::Red),
        "BLACK" => Ok(RiskLevel::Black),
        other => Err(CilError::Io(format!("unknown risk level {other:?}"))),
    }
}

/// Parse an RFC3339 timestamp into a UTC datetime, mapping failure to `CilError`.
fn parse_rfc3339(s: &str) -> Result<DateTime<Utc>, CilError> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| CilError::Io(format!("parse timestamp {s:?}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openclaw::registry::ProductionSkillRegistry;
    use chrono::Duration;

    /// Build a `GrantStore` over a fresh temp `skills.db` whose schema (incl.
    /// migration 5 `capability_grants_scoped` + `idx_grants_skill`) has been
    /// created by the frozen registry — the single source of truth.
    fn store_with_temp_db() -> (GrantStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("skills.db");
        // Constructing the registry runs the frozen migrations that create the
        // capability_grants_scoped table + idx_grants_skill index.
        let _registry = ProductionSkillRegistry::new(&db_path).expect("registry init");
        let store = GrantStore::open(&db_path).expect("grant store open");
        (store, dir)
    }

    fn sample_grant(grant_id: &str, skill_id: &str) -> ScopedGrant {
        ScopedGrant {
            grant_id: grant_id.to_string(),
            skill_id: skill_id.to_string(),
            scope_kind: ScopeKind::Persistent,
            scope_key: None,
            caps_hash: "hash-abc".to_string(),
            risk: RiskLevel::Yellow,
            decision: GrantDecision::Allow,
            granted_at: Utc::now(),
            expires_at: None,
            revoked: false,
        }
    }

    #[test]
    fn insert_then_lookup_roundtrips() {
        let (store, _dir) = store_with_temp_db();
        let grant = sample_grant("g1", "skill.a");
        store.insert(&grant).expect("insert");

        let fetched = store.get("g1").expect("get").expect("present");
        assert_eq!(fetched, grant);

        let active = store
            .active_grants_for_skill("skill.a", Utc::now())
            .expect("active");
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].grant_id, "g1");
    }

    #[test]
    fn find_reusable_matches_scope_and_caps_hash() {
        let (store, _dir) = store_with_temp_db();
        store
            .insert(&sample_grant("g1", "skill.a"))
            .expect("insert");

        let hit = store
            .find_reusable(
                "skill.a",
                ScopeKind::Persistent,
                None,
                "hash-abc",
                Utc::now(),
            )
            .expect("reusable query");
        assert!(hit.is_some(), "matching grant should be reusable");

        // Different caps_hash → no reuse (deny-by-default).
        let miss = store
            .find_reusable(
                "skill.a",
                ScopeKind::Persistent,
                None,
                "other-hash",
                Utc::now(),
            )
            .expect("reusable query");
        assert!(miss.is_none());
    }

    #[test]
    fn revoked_and_expired_grants_are_not_active() {
        let (store, _dir) = store_with_temp_db();

        // Expired grant.
        let mut expired = sample_grant("g-exp", "skill.b");
        expired.expires_at = Some(Utc::now() - Duration::hours(1));
        store.insert(&expired).expect("insert expired");

        // Revoked grant.
        let mut live = sample_grant("g-rev", "skill.b");
        live.expires_at = None;
        store.insert(&live).expect("insert live");
        assert!(store.revoke("g-rev").expect("revoke"));

        let active = store
            .active_grants_for_skill("skill.b", Utc::now())
            .expect("active");
        assert!(
            active.is_empty(),
            "expired + revoked grants must not be active"
        );

        // Reuse must also refuse a revoked grant.
        let reuse = store
            .find_reusable(
                "skill.b",
                ScopeKind::Persistent,
                None,
                "hash-abc",
                Utc::now(),
            )
            .expect("reuse");
        assert!(reuse.is_none());
    }

    #[test]
    fn workspace_partitioning_isolates_by_scope_key() {
        let (store, _dir) = store_with_temp_db();
        let mut ws_a = sample_grant("g-ws-a", "skill.c");
        ws_a.scope_kind = ScopeKind::Workspace;
        ws_a.scope_key = Some("ws-a".to_string());
        store.insert(&ws_a).expect("insert ws-a");

        let mut ws_b = sample_grant("g-ws-b", "skill.c");
        ws_b.scope_kind = ScopeKind::Workspace;
        ws_b.scope_key = Some("ws-b".to_string());
        store.insert(&ws_b).expect("insert ws-b");

        let a = store
            .active_grants_for_workspace("skill.c", "ws-a", Utc::now())
            .expect("ws-a grants");
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].grant_id, "g-ws-a");
    }

    #[test]
    fn revoke_missing_grant_reports_false() {
        let (store, _dir) = store_with_temp_db();
        assert!(!store.revoke("does-not-exist").expect("revoke"));
    }

    // ---- Task 11.8 additions: scope persistence, expiry, revoke, partitioning ----

    /// R6.5: each scope kind persists and is retrievable/active, and
    /// `find_reusable` matches ONLY on an exact `(scope_kind, scope_key,
    /// caps_hash)` tuple. Covers once/session/workspace/persistent.
    #[test]
    fn scope_persistence_across_scope_kinds() {
        let (store, _dir) = store_with_temp_db();
        let now = Utc::now();

        // Once — unpartitioned single-use grant.
        let mut once = sample_grant("g-once", "skill.scope");
        once.scope_kind = ScopeKind::Once;
        once.scope_key = None;
        store.insert(&once).expect("insert once");

        // Session — partitioned by a session id.
        let mut sess = sample_grant("g-sess", "skill.scope");
        sess.scope_kind = ScopeKind::Session;
        sess.scope_key = Some("sess-1".to_string());
        store.insert(&sess).expect("insert session");

        // Workspace — partitioned by a workspace id.
        let mut ws = sample_grant("g-ws", "skill.scope");
        ws.scope_kind = ScopeKind::Workspace;
        ws.scope_key = Some("ws-1".to_string());
        store.insert(&ws).expect("insert workspace");

        // Persistent — standing, unpartitioned.
        let mut persistent = sample_grant("g-persist", "skill.scope");
        persistent.scope_kind = ScopeKind::Persistent;
        persistent.scope_key = None;
        store.insert(&persistent).expect("insert persistent");

        // All four persist and read back active for the skill.
        let active = store
            .active_grants_for_skill("skill.scope", now)
            .expect("active");
        assert_eq!(active.len(), 4, "all four scope kinds should be active");

        // Each is reusable under its OWN exact tuple.
        assert!(store
            .find_reusable("skill.scope", ScopeKind::Once, None, "hash-abc", now)
            .expect("once reuse")
            .is_some());
        assert!(store
            .find_reusable(
                "skill.scope",
                ScopeKind::Session,
                Some("sess-1"),
                "hash-abc",
                now
            )
            .expect("session reuse")
            .is_some());
        assert!(store
            .find_reusable(
                "skill.scope",
                ScopeKind::Workspace,
                Some("ws-1"),
                "hash-abc",
                now
            )
            .expect("workspace reuse")
            .is_some());
        let persist_hit = store
            .find_reusable("skill.scope", ScopeKind::Persistent, None, "hash-abc", now)
            .expect("persistent reuse")
            .expect("persistent present");
        assert_eq!(persist_hit.grant_id, "g-persist");
        assert_eq!(persist_hit.scope_key, None);

        // A session grant is NOT reused for a DIFFERENT session key.
        assert!(store
            .find_reusable(
                "skill.scope",
                ScopeKind::Session,
                Some("sess-OTHER"),
                "hash-abc",
                now
            )
            .expect("other session reuse")
            .is_none());

        // Scope kind must match too: a session key does not satisfy a workspace query.
        assert!(store
            .find_reusable(
                "skill.scope",
                ScopeKind::Workspace,
                Some("sess-1"),
                "hash-abc",
                now
            )
            .expect("wrong-kind reuse")
            .is_none());

        // A persistent grant (scope_key=None) is not matched when a key is supplied.
        assert!(store
            .find_reusable(
                "skill.scope",
                ScopeKind::Persistent,
                Some("ws-1"),
                "hash-abc",
                now
            )
            .expect("persistent-with-key reuse")
            .is_none());
    }

    /// R6.5 (deny-by-default expiry): future expiry stays active/reusable; an
    /// expiry at-or-before `now` is inactive; `expires_at = None` never expires.
    #[test]
    fn expiry_active_future_and_boundary() {
        let (store, _dir) = store_with_temp_db();
        let now = Utc::now();

        // Future expiry — active and reusable.
        let mut future = sample_grant("g-future", "skill.exp");
        future.caps_hash = "hash-future".to_string();
        future.expires_at = Some(now + Duration::hours(1));
        store.insert(&future).expect("insert future");
        assert!(
            future.is_active_at(now),
            "future-dated grant is active in-memory"
        );
        assert!(store
            .find_reusable("skill.exp", ScopeKind::Persistent, None, "hash-future", now)
            .expect("future reuse")
            .is_some());

        // Boundary: expiry EXACTLY at `now` is expired (strict `expires_at > now`).
        let mut boundary = sample_grant("g-boundary", "skill.exp");
        boundary.caps_hash = "hash-boundary".to_string();
        boundary.expires_at = Some(now);
        store.insert(&boundary).expect("insert boundary");
        assert!(
            !boundary.is_active_at(now),
            "grant expiring exactly at now is not active"
        );
        assert!(store
            .find_reusable(
                "skill.exp",
                ScopeKind::Persistent,
                None,
                "hash-boundary",
                now
            )
            .expect("boundary reuse")
            .is_none());

        // No expiry — never expires, even far in the future.
        let mut forever = sample_grant("g-forever", "skill.exp");
        forever.caps_hash = "hash-forever".to_string();
        forever.expires_at = None;
        store.insert(&forever).expect("insert forever");
        assert!(forever.is_active_at(now + Duration::weeks(520)));

        // Only the future + forever grants are active for the skill at `now`.
        let active = store
            .active_grants_for_skill("skill.exp", now)
            .expect("active");
        let ids: Vec<&str> = active.iter().map(|g| g.grant_id.as_str()).collect();
        assert!(ids.contains(&"g-future"));
        assert!(ids.contains(&"g-forever"));
        assert!(!ids.contains(&"g-boundary"), "boundary must be excluded");
        assert_eq!(active.len(), 2);
    }

    /// R6.6: revoking an active grant forces fresh approval — it is no longer
    /// active or reusable, yet `get` still returns the row flagged `revoked`.
    #[test]
    fn revoke_forces_fresh_approval() {
        let (store, _dir) = store_with_temp_db();
        let now = Utc::now();

        let mut grant = sample_grant("g-rev-fresh", "skill.rev");
        grant.scope_kind = ScopeKind::Session;
        grant.scope_key = Some("sess-x".to_string());
        store.insert(&grant).expect("insert");

        // Reusable before revocation.
        assert!(store
            .find_reusable(
                "skill.rev",
                ScopeKind::Session,
                Some("sess-x"),
                "hash-abc",
                now
            )
            .expect("pre-revoke reuse")
            .is_some());

        // Revoke reports a row was updated.
        assert!(store.revoke("g-rev-fresh").expect("revoke"));

        // No longer active for the skill.
        assert!(store
            .active_grants_for_skill("skill.rev", now)
            .expect("active")
            .is_empty());

        // No longer reusable → fresh approval required next use.
        assert!(store
            .find_reusable(
                "skill.rev",
                ScopeKind::Session,
                Some("sess-x"),
                "hash-abc",
                now
            )
            .expect("post-revoke reuse")
            .is_none());

        // The row still exists, now flagged revoked (audit trail preserved).
        let row = store.get("g-rev-fresh").expect("get").expect("present");
        assert!(row.revoked);
        assert!(!row.is_active_at(now));
    }

    /// 11.4: workspace grants are isolated by `scope_key`; a workspace query
    /// returns only that workspace's grant and never an unpartitioned
    /// (persistent) grant for the same skill.
    #[test]
    fn workspace_partitioning_scope_key_isolation() {
        let (store, _dir) = store_with_temp_db();
        let now = Utc::now();

        let mut ws_a = sample_grant("g-part-a", "skill.part");
        ws_a.scope_kind = ScopeKind::Workspace;
        ws_a.scope_key = Some("ws-a".to_string());
        store.insert(&ws_a).expect("insert ws-a");

        let mut ws_b = sample_grant("g-part-b", "skill.part");
        ws_b.scope_kind = ScopeKind::Workspace;
        ws_b.scope_key = Some("ws-b".to_string());
        store.insert(&ws_b).expect("insert ws-b");

        // A persistent grant for the same skill must NOT leak into workspace queries.
        let mut persistent = sample_grant("g-part-persist", "skill.part");
        persistent.scope_kind = ScopeKind::Persistent;
        persistent.scope_key = None;
        store.insert(&persistent).expect("insert persistent");

        // ws-a query → only ws-a's grant.
        let a = store
            .active_grants_for_workspace("skill.part", "ws-a", now)
            .expect("ws-a");
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].grant_id, "g-part-a");

        // ws-b query → only ws-b's grant.
        let b = store
            .active_grants_for_workspace("skill.part", "ws-b", now)
            .expect("ws-b");
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].grant_id, "g-part-b");

        // Unknown workspace → nothing.
        assert!(store
            .active_grants_for_workspace("skill.part", "ws-unknown", now)
            .expect("ws-unknown")
            .is_empty());

        // The skill-wide query still sees all three (partitioning is only for the
        // workspace-scoped read path).
        assert_eq!(
            store
                .active_grants_for_skill("skill.part", now)
                .expect("all")
                .len(),
            3
        );
    }
}
