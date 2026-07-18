//! Consent-gated cold start (memory-upgrade Task 35, R8).
//!
//! Privacy-first onboarding: KRIA must NOT scan/index the user's filesystem,
//! git history, workspace, or shell history until the user has explicitly
//! granted consent for that specific source. This is the backend enforcement
//! primitive — a durable, granular consent store plus a hard [`gate`] that any
//! cold-start scanner must call first. The desktop first-run screen is a thin
//! adapter over these methods (grant/revoke per source, preview, then scan).
//!
//! [`gate`]: ColdStartConsent::gate
//!
//! Consent is stored in the authority `preferences` table (durable, survives
//! restart). Default is deny-all: nothing is scanned before approval.

use std::sync::Arc;

use rusqlite::params;

use crate::memory::db::Database;
use crate::memory::error::{MemoryResult, PermissionError, StorageError};

/// A cold-start scan source the user consents to (granularly).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanSource {
    Filesystem,
    Git,
    Workspace,
    Shell,
}

impl ScanSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScanSource::Filesystem => "filesystem",
            ScanSource::Git => "git",
            ScanSource::Workspace => "workspace",
            ScanSource::Shell => "shell",
        }
    }

    /// All sources (for building the onboarding preview screen).
    pub fn all() -> [ScanSource; 4] {
        [
            ScanSource::Filesystem,
            ScanSource::Git,
            ScanSource::Workspace,
            ScanSource::Shell,
        ]
    }

    fn pref_key(&self) -> String {
        format!("coldstart_consent:{}", self.as_str())
    }
}

/// A previewable candidate the scanner proposes before any commit — shown to
/// the user in onboarding so scan results are previewable/deletable first (R8).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScanCandidate {
    pub source: String,
    pub path: String,
    pub detail: String,
}

/// Durable, granular cold-start consent store + scan gate.
#[derive(Clone)]
pub struct ColdStartConsent {
    db: Arc<Database>,
}

impl ColdStartConsent {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    fn set_pref(&self, key: &str, value: &str) -> MemoryResult<()> {
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "INSERT INTO preferences(key, value, vector_clock, updated_at, device_id) \
                 VALUES(?1,?2,'',?3,'local') \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                params![key, value, chrono::Utc::now().to_rfc3339()],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()
    }

    fn get_pref(&self, key: &str) -> MemoryResult<Option<String>> {
        self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare("SELECT value FROM preferences WHERE key = ?1")
                .map_err(StorageError::Sqlite)?;
            let mut rows = stmt.query(params![key]).map_err(StorageError::Sqlite)?;
            match rows.next().map_err(StorageError::Sqlite)? {
                Some(r) => Ok(Some(r.get(0).map_err(StorageError::Sqlite)?)),
                None => Ok(None),
            }
        })
    }

    /// Grant consent to scan a source.
    pub fn grant(&self, source: ScanSource) -> MemoryResult<()> {
        self.set_pref(&source.pref_key(), "granted")
    }

    /// Revoke consent for a source (subsequent scans are gated again).
    pub fn revoke(&self, source: ScanSource) -> MemoryResult<()> {
        self.set_pref(&source.pref_key(), "denied")
    }

    /// Whether a source is currently granted. Deny-by-default.
    pub fn is_granted(&self, source: ScanSource) -> MemoryResult<bool> {
        Ok(self.get_pref(&source.pref_key())?.as_deref() == Some("granted"))
    }

    /// The hard gate every cold-start scanner MUST call before scanning/indexing
    /// `source`. Errors (deny-by-default) unless the user granted consent — so
    /// no automatic indexing can occur before approval (R8).
    pub fn gate(&self, source: ScanSource) -> MemoryResult<()> {
        if self.is_granted(source)? {
            Ok(())
        } else {
            Err(PermissionError::Consent(format!(
                "cold-start scan of {} not consented",
                source.as_str()
            ))
            .into())
        }
    }

    /// Sources the user has granted (for the onboarding summary).
    pub fn granted_sources(&self) -> MemoryResult<Vec<ScanSource>> {
        let mut out = Vec::new();
        for s in ScanSource::all() {
            if self.is_granted(s)? {
                out.push(s);
            }
        }
        Ok(out)
    }

    /// Whether first-run onboarding has been completed (the consent screen was
    /// shown + acted on). Distinct from any individual grant.
    pub fn onboarding_complete(&self) -> MemoryResult<bool> {
        Ok(self.get_pref("coldstart_onboarding_complete")?.as_deref() == Some("1"))
    }

    /// Mark onboarding complete (called once the user finishes the first-run
    /// screen, regardless of which sources they granted).
    pub fn complete_onboarding(&self) -> MemoryResult<()> {
        self.set_pref("coldstart_onboarding_complete", "1")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn consent() -> ColdStartConsent {
        ColdStartConsent::new(Arc::new(Database::open_in_memory().unwrap()))
    }

    #[test]
    fn deny_by_default_then_grant_and_revoke() {
        let c = consent();
        // Nothing granted → gate refuses every source (no auto-index).
        for s in ScanSource::all() {
            assert!(!c.is_granted(s).unwrap());
            assert!(
                c.gate(s).is_err(),
                "{} must be gated by default",
                s.as_str()
            );
        }

        c.grant(ScanSource::Filesystem).unwrap();
        assert!(c.is_granted(ScanSource::Filesystem).unwrap());
        assert!(c.gate(ScanSource::Filesystem).is_ok());
        // Other sources remain gated (granular).
        assert!(c.gate(ScanSource::Shell).is_err());
        assert_eq!(c.granted_sources().unwrap(), vec![ScanSource::Filesystem]);

        c.revoke(ScanSource::Filesystem).unwrap();
        assert!(c.gate(ScanSource::Filesystem).is_err());
    }

    #[test]
    fn onboarding_flag_persists() {
        let c = consent();
        assert!(!c.onboarding_complete().unwrap());
        c.complete_onboarding().unwrap();
        assert!(c.onboarding_complete().unwrap());
    }
}
