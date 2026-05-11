//! Append-only HMAC-signed audit ledger.
//!
//! Records all OpenClaw tool invocations, installations, and security events.
//! Every entry is HMAC-SHA256 signed to detect tampering.
//! The ledger is append-only — entries are never modified or deleted.

use super::types::AuditEventType;
use crate::infra::ToolResult;
use hmac::{Hmac, Mac};
use rusqlite::{params, Connection};
use sha2::Sha256;
use std::sync::{Arc, Mutex};

type HmacSha256 = Hmac<Sha256>;

/// Append-only audit ledger for OpenClaw operations.
pub struct AuditLedger {
    db: Arc<Mutex<Connection>>,
    hmac_key: Vec<u8>,
}

/// A single audit entry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuditEntry {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub event_type: AuditEventType,
    pub skill_id: String,
    pub invocation_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub tool_name: String,
    pub risk_level: String,
    pub input_hash: String,
    pub output_hash: String,
    pub duration_ms: u64,
    pub success: bool,
    pub error_summary: Option<String>,
    pub resource_class: String,
    pub container_id: String,
    pub signature: String,
}

impl AuditLedger {
    /// Create or open an audit ledger.
    pub fn open(db_path: &std::path::Path, hmac_key: Vec<u8>) -> Result<Self, AuditError> {
        let conn = Connection::open(db_path).map_err(AuditError::Db)?;

        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(AuditError::Db)?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS audit_log (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp       TEXT NOT NULL,
                event_type      TEXT NOT NULL,
                skill_id        TEXT NOT NULL,
                invocation_id   TEXT NOT NULL,
                session_id      TEXT NOT NULL,
                turn_id         TEXT NOT NULL,
                tool_name       TEXT NOT NULL,
                risk_level      TEXT NOT NULL,
                input_hash      TEXT NOT NULL,
                output_hash     TEXT NOT NULL,
                duration_ms     INTEGER NOT NULL,
                success         INTEGER NOT NULL,
                error_summary   TEXT,
                resource_class  TEXT NOT NULL,
                container_id    TEXT NOT NULL,
                signature       TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_audit_timestamp
                ON audit_log(timestamp);
            CREATE INDEX IF NOT EXISTS idx_audit_skill
                ON audit_log(skill_id);
            CREATE INDEX IF NOT EXISTS idx_audit_event
                ON audit_log(event_type);",
        )
        .map_err(AuditError::Db)?;

        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
            hmac_key,
        })
    }

    /// Append an audit entry.
    pub fn append(&self, entry: &AuditEntry) -> Result<i64, AuditError> {
        let db = self.db.lock().unwrap();

        db.execute(
            "INSERT INTO audit_log
             (timestamp, event_type, skill_id, invocation_id, session_id, turn_id,
              tool_name, risk_level, input_hash, output_hash, duration_ms, success,
              error_summary, resource_class, container_id, signature)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                entry.timestamp.to_rfc3339(),
                entry.event_type.as_str(),
                entry.skill_id,
                entry.invocation_id,
                entry.session_id,
                entry.turn_id,
                entry.tool_name,
                entry.risk_level,
                entry.input_hash,
                entry.output_hash,
                entry.duration_ms as i64,
                entry.success as i32,
                entry.error_summary,
                entry.resource_class,
                entry.container_id,
                entry.signature,
            ],
        )
        .map_err(AuditError::Db)?;

        Ok(db.last_insert_rowid())
    }

    /// Sign an audit entry with HMAC-SHA256.
    pub fn sign_entry(&self, entry: &AuditEntry) -> String {
        let payload = format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            entry.timestamp.to_rfc3339(),
            entry.event_type.as_str(),
            entry.skill_id,
            entry.invocation_id,
            entry.session_id,
            entry.turn_id,
            entry.tool_name,
            entry.risk_level,
            entry.input_hash,
            entry.output_hash,
            entry.duration_ms,
            entry.success,
            entry.error_summary.as_deref().unwrap_or(""),
            entry.resource_class,
            entry.container_id,
        );

        let mut mac =
            HmacSha256::new_from_slice(&self.hmac_key).expect("HMAC key length is always valid");
        mac.update(payload.as_bytes());
        let result = mac.finalize();
        hex::encode(result.into_bytes())
    }

    /// Verify the integrity of the audit chain.
    /// Returns the ID of the first tampered entry, if any.
    pub fn verify_chain(&self) -> Result<Option<i64>, AuditError> {
        let db = self.db.lock().unwrap();
        let mut stmt = db
            .prepare(
                "SELECT id, timestamp, event_type, skill_id, invocation_id, session_id,
                 turn_id, tool_name, risk_level, input_hash, output_hash, duration_ms,
                 success, error_summary, resource_class, container_id, signature
                 FROM audit_log ORDER BY id",
            )
            .map_err(AuditError::Db)?;

        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, i64>(11)?,
                    row.get::<_, i32>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, String>(14)?,
                    row.get::<_, String>(15)?,
                    row.get::<_, String>(16)?,
                ))
            })
            .map_err(AuditError::Db)?;

        for row in rows {
            let (id, ts, et, sid, iid, sess, tid, tn, rl, ih, oh, dm, suc, es, rc, ci, sig) =
                row.map_err(AuditError::Db)?;

            let entry = AuditEntry {
                timestamp: chrono::DateTime::parse_from_rfc3339(&ts)
                    .map_err(|_| AuditError::VerificationFailed(id))?
                    .with_timezone(&chrono::Utc),
                event_type: et.parse().map_err(|_| AuditError::VerificationFailed(id))?,
                skill_id: sid,
                invocation_id: iid,
                session_id: sess,
                turn_id: tid,
                tool_name: tn,
                risk_level: rl,
                input_hash: ih,
                output_hash: oh,
                duration_ms: dm as u64,
                success: suc != 0,
                error_summary: es,
                resource_class: rc,
                container_id: ci,
                signature: sig.clone(),
            };

            let expected = self.sign_entry(&entry);
            if expected != sig {
                return Ok(Some(id));
            }
        }

        Ok(None) // Chain is intact
    }

    /// Helper to create a SkillInstalled audit entry.
    ///
    /// `source_url` is stored in the `container_id` column (repurposed as the
    /// install source, since no container is involved at install time).
    pub fn create_skill_install_entry(
        skill_id: &str,
        skill_name: &str,
        trust_tier: &str,
        source_url: &str,
    ) -> AuditEntry {
        use sha2::{Digest, Sha256};
        let input_hash = {
            let mut h = Sha256::new();
            h.update(source_url.as_bytes());
            hex::encode(h.finalize())
        };
        AuditEntry {
            timestamp: chrono::Utc::now(),
            event_type: AuditEventType::SkillInstalled,
            skill_id: skill_id.to_string(),
            invocation_id: uuid::Uuid::new_v4().to_string(),
            session_id: String::new(),
            turn_id: String::new(),
            tool_name: skill_name.to_string(),
            risk_level: trust_tier.to_string(),
            input_hash,
            output_hash: String::new(),
            duration_ms: 0,
            success: true,
            error_summary: None,
            resource_class: String::new(),
            container_id: source_url.to_string(),
            signature: String::new(),
        }
    }

    /// Helper to create a standard invocation entry.
    pub fn create_invocation_entry(
        event_type: AuditEventType,
        skill_id: &str,
        invocation_id: &str,
        session_id: &str,
        turn_id: &str,
        tool_name: &str,
        risk_level: &str,
        input_data: &serde_json::Value,
        output: &ToolResult,
        duration_ms: u64,
        resource_class: &str,
        container_id: &str,
    ) -> AuditEntry {
        use sha2::{Digest, Sha256};

        let input_hash = {
            let mut hasher = Sha256::new();
            hasher.update(input_data.to_string().as_bytes());
            hex::encode(hasher.finalize())
        };

        let output_hash = {
            let mut hasher = Sha256::new();
            hasher.update(output.data.to_string().as_bytes());
            hex::encode(hasher.finalize())
        };

        AuditEntry {
            timestamp: chrono::Utc::now(),
            event_type,
            skill_id: skill_id.to_string(),
            invocation_id: invocation_id.to_string(),
            session_id: session_id.to_string(),
            turn_id: turn_id.to_string(),
            tool_name: tool_name.to_string(),
            risk_level: risk_level.to_string(),
            input_hash,
            output_hash,
            duration_ms,
            success: output.success,
            error_summary: output.error.clone(),
            resource_class: resource_class.to_string(),
            container_id: container_id.to_string(),
            signature: String::new(), // Filled by sign_entry()
        }
    }
}

/// Errors from the audit ledger.
#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    #[error("database error: {0}")]
    Db(#[source] rusqlite::Error),
    #[error("audit chain verification failed at entry {0}")]
    VerificationFailed(i64),
}

/// Hex encoding helper (avoid adding another dependency).
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes
            .as_ref()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect()
    }
}
