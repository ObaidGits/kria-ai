//! SQLite-backed skill registry.
//!
//! Persists installed skills in the `installed_skills` table.
//! Skills are stored as JSON-serialized `SkillDescriptor` blobs.

use super::types::*;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// SQLite-backed skill registry.
pub struct SkillRegistry {
    db: Arc<Mutex<Connection>>,
}

impl SkillRegistry {
    /// Open or create a skill registry at the given database path.
    pub fn open(db_path: &Path) -> Result<Self, RegistryError> {
        let conn = Connection::open(db_path).map_err(RegistryError::Db)?;

        // Enable WAL mode for concurrent reads
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(RegistryError::Db)?;

        // Create table if not exists
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS installed_skills (
                skill_id        TEXT PRIMARY KEY,
                name            TEXT NOT NULL,
                category        TEXT NOT NULL,
                trust_tier      TEXT NOT NULL DEFAULT 'local',
                status          TEXT NOT NULL DEFAULT 'active',
                descriptor_json TEXT NOT NULL,
                installed_at    TEXT NOT NULL,
                last_used_at    TEXT,
                use_count       INTEGER NOT NULL DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_skills_category
                ON installed_skills(category);
            CREATE INDEX IF NOT EXISTS idx_skills_status
                ON installed_skills(status);
            CREATE INDEX IF NOT EXISTS idx_skills_trust
                ON installed_skills(trust_tier);",
        )
        .map_err(RegistryError::Db)?;

        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    /// Install a skill (store its descriptor).
    pub fn install(&self, skill: &SkillDescriptor) -> Result<(), RegistryError> {
        let db = self.db.lock().unwrap();
        let descriptor_json =
            serde_json::to_string(skill).map_err(|e| RegistryError::Serialization(e.to_string()))?;

        db.execute(
            "INSERT OR REPLACE INTO installed_skills
             (skill_id, name, category, trust_tier, status, descriptor_json, installed_at, last_used_at, use_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                skill.skill_id,
                skill.name,
                skill.category,
                skill.trust_tier.as_str(),
                skill.status.as_str(),
                descriptor_json,
                skill.installed_at.to_rfc3339(),
                skill.last_used_at.map(|t| t.to_rfc3339()),
                skill.use_count as i64,
            ],
        )
        .map_err(RegistryError::Db)?;

        Ok(())
    }

    /// Uninstall a skill.
    pub fn uninstall(&self, skill_id: &str) -> Result<(), RegistryError> {
        let db = self.db.lock().unwrap();
        let rows = db
            .execute(
                "DELETE FROM installed_skills WHERE skill_id = ?1",
                params![skill_id],
            )
            .map_err(RegistryError::Db)?;

        if rows == 0 {
            return Err(RegistryError::NotFound(skill_id.to_string()));
        }

        Ok(())
    }

    /// Toggle a skill enabled/disabled.
    pub fn toggle(&self, skill_id: &str, enabled: bool) -> Result<(), RegistryError> {
        let status = if enabled {
            SkillStatus::Active.as_str()
        } else {
            SkillStatus::Disabled.as_str()
        };

        let db = self.db.lock().unwrap();
        let rows = db
            .execute(
                "UPDATE installed_skills SET status = ?1 WHERE skill_id = ?2",
                params![status, skill_id],
            )
            .map_err(RegistryError::Db)?;

        if rows == 0 {
            return Err(RegistryError::NotFound(skill_id.to_string()));
        }

        Ok(())
    }

    /// Get a skill by ID.
    pub fn get(&self, skill_id: &str) -> Result<SkillDescriptor, RegistryError> {
        let db = self.db.lock().unwrap();
        let descriptor_json: String = db
            .query_row(
                "SELECT descriptor_json FROM installed_skills WHERE skill_id = ?1",
                params![skill_id],
                |row| row.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    RegistryError::NotFound(skill_id.to_string())
                }
                other => RegistryError::Db(other),
            })?;

        serde_json::from_str(&descriptor_json)
            .map_err(|e| RegistryError::Serialization(e.to_string()))
    }

    /// List all installed skills.
    pub fn list_installed(&self) -> Result<Vec<SkillDescriptor>, RegistryError> {
        let db = self.db.lock().unwrap();
        let mut stmt = db
            .prepare("SELECT descriptor_json FROM installed_skills ORDER BY name")
            .map_err(RegistryError::Db)?;

        let skills = stmt
            .query_map([], |row| {
                let json: String = row.get(0)?;
                Ok(json)
            })
            .map_err(RegistryError::Db)?
            .filter_map(|r| r.ok())
            .filter_map(|json| serde_json::from_str(&json).ok())
            .collect();

        Ok(skills)
    }

    /// List only active (usable) skills.
    pub fn list_active(&self) -> Result<Vec<SkillDescriptor>, RegistryError> {
        let db = self.db.lock().unwrap();
        let mut stmt = db
            .prepare(
                "SELECT descriptor_json FROM installed_skills WHERE status = 'active' ORDER BY name",
            )
            .map_err(RegistryError::Db)?;

        let skills = stmt
            .query_map([], |row| {
                let json: String = row.get(0)?;
                Ok(json)
            })
            .map_err(RegistryError::Db)?
            .filter_map(|r| r.ok())
            .filter_map(|json| serde_json::from_str(&json).ok())
            .collect();

        Ok(skills)
    }

    /// Record a skill invocation (increment use_count, update last_used_at).
    pub fn record_invocation(&self, skill_id: &str) -> Result<(), RegistryError> {
        let db = self.db.lock().unwrap();
        let now = chrono::Utc::now().to_rfc3339();

        db.execute(
            "UPDATE installed_skills
             SET use_count = use_count + 1, last_used_at = ?1
             WHERE skill_id = ?2",
            params![now, skill_id],
        )
        .map_err(RegistryError::Db)?;

        Ok(())
    }

    /// Get all skills matching a given status.
    pub fn list_by_status(&self, status: SkillStatus) -> Result<Vec<SkillDescriptor>, RegistryError> {
        let db = self.db.lock().unwrap();
        let mut stmt = db
            .prepare("SELECT descriptor_json FROM installed_skills WHERE status = ?1 ORDER BY name")
            .map_err(RegistryError::Db)?;

        let skills = stmt
            .query_map(params![status.as_str()], |row| {
                let json: String = row.get(0)?;
                Ok(json)
            })
            .map_err(RegistryError::Db)?
            .filter_map(|r| r.ok())
            .filter_map(|json| serde_json::from_str(&json).ok())
            .collect();

        Ok(skills)
    }

    /// Run lifecycle maintenance: auto-disable stale skills.
    pub fn run_lifecycle_maintenance(
        &self,
        stale_days: i64,
        auto_disable_days: i64,
    ) -> Result<Vec<LifecycleAction>, RegistryError> {
        let now = chrono::Utc::now();
        let mut actions = Vec::new();

        let skills = self.list_installed()?;
        for skill in &skills {
            let days_since_use = skill
                .last_used_at
                .map(|t| (now - t).num_days())
                .unwrap_or(i64::MAX);

            if days_since_use > auto_disable_days && skill.status == SkillStatus::Active {
                self.toggle(&skill.skill_id, false)?;
                actions.push(LifecycleAction::AutoDisabled {
                    skill_id: skill.skill_id.clone(),
                    days_unused: days_since_use,
                });
            } else if days_since_use > stale_days && skill.status == SkillStatus::Active {
                actions.push(LifecycleAction::FlaggedStale {
                    skill_id: skill.skill_id.clone(),
                    days_unused: days_since_use,
                });
            }
        }

        Ok(actions)
    }
}

/// Errors from the skill registry.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("database error: {0}")]
    Db(#[source] rusqlite::Error),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("skill not found: {0}")]
    NotFound(String),
}
