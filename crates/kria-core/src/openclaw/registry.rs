//! Production Skill Registry - Single authoritative source for all skills.
//!
//! A5 transforms OpenClaw from "runtime capable" into "production skill platform".
//! This registry is the SINGLE source of truth for ALL skills in the system.
//! Every runtime, router, installer, marketplace, and future AI workflow MUST
//! depend ONLY on this registry.
//!
//! NO duplicated registries. NO filesystem scanning. NO multiple sources of truth.

use super::types::*;
use crate::safety::RiskLevel;
use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

/// One additive, forward-only schema migration (task 8/R19). `apply` MUST be
/// additive only (`ALTER TABLE ... ADD COLUMN`, new `CREATE TABLE IF NOT
/// EXISTS`, new `CREATE INDEX IF NOT EXISTS`) — never a drop, rename, or
/// destructive rewrite, per the frozen A0-A9 no-redesign rule. Runs inside
/// its own transaction; `PRAGMA user_version` is bumped only after success.
struct Migration {
    version: i64,
    description: &'static str,
    apply: fn(&Connection) -> Result<(), rusqlite::Error>,
}

/// The current schema version. Bump this and add a new `Migration` entry to
/// `MIGRATIONS` whenever a column/table is added — never edit an already-
/// shipped migration's `apply` fn once it has run on any real database.
const SCHEMA_VERSION: i64 = 6;

/// Ordered list of every migration since the unversioned (`user_version=0`)
/// schema. A fresh database created by `create_base_schema` already has
/// every base column, so migration 1 is a no-op there (`ADD COLUMN` is
/// skipped if the column already exists) — but is REQUIRED to bring an
/// existing pre-migration-system database (any real user's `skills.db`
/// today) forward safely. This is the real fix proven necessary by task 19's
/// `finding_r19_2_no_migration_for_older_schema`.
static MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        description: "add skills.granted_capabilities (authoritative CapabilityGrant JSON; task: capability-grant wiring)",
        apply: |conn| {
            let has_column: bool = conn
                .prepare("SELECT granted_capabilities FROM skills LIMIT 1")
                .is_ok();
            if !has_column {
                conn.execute(
                    "ALTER TABLE skills ADD COLUMN granted_capabilities TEXT NOT NULL DEFAULT '[]'",
                    [],
                )?;
            }
            Ok(())
        },
    },
    Migration {
        version: 2,
        description: "add skills.input_schema (authoritative MCP inputSchema JSON; RC1 schema-driven argument generation + RC2 registry sync)",
        apply: |conn| {
            let has_column: bool = conn
                .prepare("SELECT input_schema FROM skills LIMIT 1")
                .is_ok();
            if !has_column {
                conn.execute(
                    "ALTER TABLE skills ADD COLUMN input_schema TEXT",
                    [],
                )?;
            }
            Ok(())
        },
    },
    Migration {
        version: 3,
        description: "add capability_profiles derived view (OpenClaw ICP §7.4; rebuildable from ProductionSkillRegistry, keyed by skill_id — never a second source of truth)",
        apply: |conn| {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS capability_profiles (
    skill_id TEXT PRIMARY KEY REFERENCES skills(skill_id) ON DELETE CASCADE,
    provides_json TEXT NOT NULL DEFAULT '[]',
    consumes_json TEXT NOT NULL DEFAULT '[]',
    inputs_json   TEXT NOT NULL DEFAULT '[]',
    outputs_json  TEXT NOT NULL DEFAULT '[]',
    embedding     BLOB,
    profile_epoch INTEGER NOT NULL DEFAULT 0
)",
                [],
            )?;
            Ok(())
        },
    },
    Migration {
        version: 4,
        description: "add market_catalog derived view (OpenClaw ICP §7.4; offline-embedded marketplace cache, rebuildable from marketplace fetch)",
        apply: |conn| {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS market_catalog (
    provider_id TEXT NOT NULL,
    slug        TEXT NOT NULL,
    manifest_json TEXT NOT NULL,
    version     TEXT NOT NULL,
    embedding   BLOB,
    trust_hint  TEXT,
    quality     REAL,
    popularity  REAL,
    deprecated  INTEGER NOT NULL DEFAULT 0,
    fetched_at  TEXT NOT NULL,
    PRIMARY KEY (provider_id, slug)
)",
                [],
            )?;
            Ok(())
        },
    },
    Migration {
        version: 5,
        description: "add capability_grants_scoped + idx_grants_skill (OpenClaw ICP §7.4; scoped permission grants keyed by skill_id)",
        apply: |conn| {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS capability_grants_scoped (
    grant_id     TEXT PRIMARY KEY,
    skill_id     TEXT NOT NULL,
    scope_kind   TEXT NOT NULL,
    scope_key    TEXT,
    caps_hash    TEXT NOT NULL,
    risk         TEXT NOT NULL,
    decision     TEXT NOT NULL,
    granted_at   TEXT NOT NULL,
    expires_at   TEXT,
    revoked      INTEGER NOT NULL DEFAULT 0
)",
                [],
            )?;
            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_grants_skill ON capability_grants_scoped(skill_id)",
                [],
            )?;
            Ok(())
        },
    },
    Migration {
        version: 6,
        description: "add capability_edges derived view (OpenClaw ICP §7.4; capability graph edges rebuildable from SkillMetadata + capability profiles)",
        apply: |conn| {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS capability_edges (
    from_skill TEXT NOT NULL,
    to_skill   TEXT NOT NULL,
    edge_kind  TEXT NOT NULL,
    weight     REAL NOT NULL DEFAULT 1.0,
    PRIMARY KEY (from_skill, to_skill, edge_kind)
)",
                [],
            )?;
            Ok(())
        },
    },
];

/// Single authoritative skill state (A5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillState {
    /// Discovered but not verified yet
    Discovered,
    /// Verified and ready for installation  
    Verified,
    /// Installed and ready to use
    Installed,
    /// Enabled and active
    Enabled,
    /// Disabled by user or policy
    Disabled,
    /// Deprecated - should not be used
    Deprecated,
    /// Removed from system
    Removed,
    /// Broken - runtime/dependency failure
    Broken,
    /// Recovering from broken state
    Recovering,
}

impl SkillState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Discovered => "discovered",
            Self::Verified => "verified",
            Self::Installed => "installed",
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
            Self::Deprecated => "deprecated",
            Self::Removed => "removed",
            Self::Broken => "broken",
            Self::Recovering => "recovering",
        }
    }

    pub fn is_usable(&self) -> bool {
        matches!(self, Self::Enabled)
    }
}

/// Skill discovery source (A5.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DiscoverySource {
    /// Bundled skills shipped with KRIA
    Bundled { path: String },
    /// Installed bundle files
    InstalledBundle { bundle_path: String },
    /// Future ClawHub marketplace  
    ClawHub { repository_url: String },
    /// Generated skills from AI workflows
    Generated { workflow_id: String },
    /// Local workspace skills
    Workspace { workspace_path: String },
    /// Developer skills in development
    Developer { dev_path: String },
}

/// Comprehensive skill metadata for production registry (A5.3 + A5.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMetadata {
    // Core identity
    pub skill_id: String,
    pub name: String,
    pub description: String,
    pub publisher: String,
    pub version: String,
    pub category: String,

    // Discovery
    pub discovery_source: DiscoverySource,
    pub discovered_at: chrono::DateTime<chrono::Utc>,

    // Capabilities & Runtime
    pub capabilities: SkillCapabilities,
    pub runtime_requirements: String,
    pub risk_level: RiskLevel,
    pub resource_class: ResourceClass,

    // Organization
    pub tags: Vec<String>,
    pub categories: Vec<String>,

    // Version management (A5.5)
    pub semantic_version: String,
    pub dependencies: Vec<SkillDependency>,
    pub compatibility_requirements: Vec<String>,

    // Security & Trust
    pub trust_tier: TrustTier,
    pub content_hash: String,
    pub signature: Option<String>,

    /// Authoritative granted capabilities (A3/capability-grant wiring, schema
    /// migration 1). This is the SOLE source of truth the runtime uses to
    /// materialize a container at execution time — never `SkillCapabilities`
    /// (that's the legacy display-only flag view). Empty for GREEN/no-grant
    /// skills. `#[serde(default)]` so any pre-migration-1 JSON blob (there
    /// shouldn't be any once migrations run, but stay defensive) still parses.
    #[serde(default)]
    pub granted_capabilities: Vec<crate::openclaw::capability::CapabilityGrant>,

    // Bundle information
    pub bundle_path: Option<String>,
    pub manifest_toml: Option<String>,

    /// The skill's authoritative JSON input schema (MCP `inputSchema`), the
    /// SAME schema the skill handler validates against inside the container.
    /// Populated by the container `tools/list` registry sync (RC2) and used by
    /// schema-driven argument generation (RC1) to turn a natural-language
    /// request into typed, schema-valid arguments. `#[serde(default)]` +
    /// schema migration 2 keep older DBs/JSON blobs parsing.
    #[serde(default)]
    pub input_schema: Option<serde_json::Value>,

    // State tracking (A5.1)
    pub state: SkillState,
    pub state_changed_at: chrono::DateTime<chrono::Utc>,
}

/// Interpret a `SkillDescriptor.parameters` value as a JSON input schema.
/// Returns `Some(schema)` only when it actually looks like a JSON Schema
/// (`type` or `properties` present) — an empty `{}` stub becomes `None` so we
/// never persist a meaningless schema. General; no per-skill knowledge.
fn schema_from_descriptor_params(params: &serde_json::Value) -> Option<serde_json::Value> {
    if params.get("type").is_some() || params.get("properties").is_some() {
        Some(params.clone())
    } else {
        None
    }
}

/// Skill dependency for version management (A5.5 + A5.10).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDependency {
    pub skill_id: String,
    pub version_requirement: String,
    pub optional: bool,
}

/// Health status tracking (A5.6).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillHealth {
    pub skill_id: String,
    pub status: HealthStatus,
    pub last_check: chrono::DateTime<chrono::Utc>,
    pub failure_count: u32,
    pub failure_reasons: Vec<String>,
    pub recovery_attempts: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    Healthy,
    Broken,
    Deprecated,
    Disabled,
    VerificationFailed,
    MissingDependency,
    RuntimeUnavailable,
}

/// Usage statistics (A5.7).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillStatistics {
    pub skill_id: String,
    pub usage_count: u64,
    pub last_execution: Option<chrono::DateTime<chrono::Utc>>,
    pub success_rate: f64,
    pub failure_rate: f64,
    pub average_latency_ms: f64,
    pub average_resource_usage: f64,
    pub installation_date: chrono::DateTime<chrono::Utc>,
    pub publisher_trust_score: f64,
}

/// Registry events (A5.8).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RegistryEvent {
    // Installation events
    Installed {
        skill_id: String,
        version: String,
    },
    Updated {
        skill_id: String,
        old_version: String,
        new_version: String,
    },
    Enabled {
        skill_id: String,
    },
    Disabled {
        skill_id: String,
    },

    // Health events
    Broken {
        skill_id: String,
        reason: String,
    },
    Recovered {
        skill_id: String,
    },

    // Lifecycle events
    Removed {
        skill_id: String,
    },
    Deprecated {
        skill_id: String,
    },

    // Discovery events
    Verified {
        skill_id: String,
    },
    Rejected {
        skill_id: String,
        reason: String,
    },

    // Usage events
    ExecutionStarted {
        skill_id: String,
        invocation_id: String,
    },
    ExecutionCompleted {
        skill_id: String,
        invocation_id: String,
        success: bool,
        latency_ms: u64,
    },
}

/// Search query for production search API (A5.9).
#[derive(Debug, Clone)]
pub struct SkillQuery {
    pub slug: Option<String>,
    pub publisher: Option<String>,
    pub description_contains: Option<String>,
    pub tags: Vec<String>,
    pub categories: Vec<String>,
    pub capabilities: Vec<String>,
    pub runtime_requirements: Option<String>,
    pub risk_level: Option<RiskLevel>,
    pub state: Option<SkillState>,
    pub enabled_only: bool,
}

/// Dependency conflict information (A5.10).
#[derive(Debug, Clone)]
pub struct DependencyConflict {
    pub skill_id: String,
    pub conflicting_skill: String,
    pub conflict_type: ConflictType,
    pub details: String,
}

#[derive(Debug, Clone)]
pub enum ConflictType {
    VersionIncompatible,
    CyclicDependency,
    MissingDependency,
    ResourceConflict,
}

/// Bundle provenance stored alongside the derived descriptor (single registry, single row).
#[derive(Debug, Clone, Default)]
pub struct BundleProvenance {
    pub publisher: String,
    pub version: String,
    pub content_hash: String,
    pub signature: String,
    pub manifest_toml: String,
    pub bundle_path: String,
}

/// Production Skill Registry - Single authoritative source (A5).
///
/// This is THE registry. All skills exist exactly once here.
/// No filesystem scanning. No duplicate registries. No multiple sources.
pub struct ProductionSkillRegistry {
    db: Arc<Mutex<Connection>>,
    event_sender: broadcast::Sender<RegistryEvent>,
}

impl ProductionSkillRegistry {
    /// Create new production skill registry (A5.1).
    pub fn new(db_path: &Path) -> Result<Self, RegistryError> {
        let conn = Connection::open(db_path).map_err(RegistryError::Db)?;

        // Enable WAL mode for concurrent reads
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(RegistryError::Db)?;

        let registry = Self {
            db: Arc::new(Mutex::new(conn)),
            event_sender: broadcast::channel(1000).0,
        };

        registry.initialize_schema()?;
        Ok(registry)
    }

    /// Initialize complete database schema (A5.1 + A5.3 + A5.4), then run any
    /// pending versioned migrations (task 8/R19 — see `SCHEMA_VERSION` and
    /// `run_migrations` below). `CREATE TABLE IF NOT EXISTS` alone is a no-op
    /// against an existing older-schema database; migrations are what
    /// actually bring an upgrading user's real DB forward safely.
    fn initialize_schema(&self) -> Result<(), RegistryError> {
        {
            let db = self.db.lock().unwrap();
            Self::create_base_schema(&db)?;
        }
        self.run_migrations()?;
        Ok(())
    }

    fn create_base_schema(db: &Connection) -> Result<(), RegistryError> {
        db.execute_batch(
            r#"
            -- Core skills table - authoritative metadata (A5.3 + A5.4)
            CREATE TABLE IF NOT EXISTS skills (
                skill_id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT NOT NULL,
                publisher TEXT NOT NULL,
                version TEXT NOT NULL,
                category TEXT NOT NULL,
                
                -- Discovery (A5.2)
                discovery_source TEXT NOT NULL,
                discovered_at TEXT NOT NULL,
                
                -- Capabilities & Runtime
                capabilities TEXT NOT NULL, -- JSON SkillCapabilities
                runtime_requirements TEXT NOT NULL,
                risk_level TEXT NOT NULL,
                resource_class TEXT NOT NULL,
                
                -- Organization (A5.3)
                tags TEXT NOT NULL, -- JSON array
                categories TEXT NOT NULL, -- JSON array
                
                -- Version Management (A5.5)
                semantic_version TEXT NOT NULL,
                dependencies TEXT NOT NULL, -- JSON array of SkillDependency
                compatibility_requirements TEXT NOT NULL, -- JSON array
                
                -- Security & Trust
                trust_tier TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                signature TEXT,

                -- Authoritative granted capabilities (A3/capability-grant wiring;
                -- schema migration 1). JSON array of CapabilityGrant. The runtime
                -- materializes containers SOLELY from this column at execution time.
                granted_capabilities TEXT NOT NULL DEFAULT '[]',

                -- Bundle information
                bundle_path TEXT,
                manifest_toml TEXT,

                -- Authoritative MCP inputSchema JSON (RC1/RC2). Nullable — a
                -- fresh base schema includes it; migration 2 backfills older DBs.
                input_schema TEXT,
                
                -- Current state (A5.1)
                state TEXT NOT NULL DEFAULT 'discovered',
                state_changed_at TEXT NOT NULL,
                
                -- Metadata
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            
            -- Health tracking (A5.6)
            CREATE TABLE IF NOT EXISTS skill_health (
                skill_id TEXT PRIMARY KEY,
                status TEXT NOT NULL,
                last_check TEXT NOT NULL,
                failure_count INTEGER NOT NULL DEFAULT 0,
                failure_reasons TEXT NOT NULL, -- JSON array
                recovery_attempts INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY(skill_id) REFERENCES skills(skill_id) ON DELETE CASCADE
            );
            
            -- Statistics tracking (A5.7)
            CREATE TABLE IF NOT EXISTS skill_statistics (
                skill_id TEXT PRIMARY KEY,
                usage_count INTEGER NOT NULL DEFAULT 0,
                last_execution TEXT,
                success_rate REAL NOT NULL DEFAULT 0.0,
                failure_rate REAL NOT NULL DEFAULT 0.0,
                average_latency_ms REAL NOT NULL DEFAULT 0.0,
                average_resource_usage REAL NOT NULL DEFAULT 0.0,
                installation_date TEXT NOT NULL,
                publisher_trust_score REAL NOT NULL DEFAULT 0.0,
                FOREIGN KEY(skill_id) REFERENCES skills(skill_id) ON DELETE CASCADE
            );
            
            -- Dependency graph (A5.10)
            CREATE TABLE IF NOT EXISTS skill_dependencies (
                skill_id TEXT NOT NULL,
                dependency_skill_id TEXT NOT NULL,
                version_requirement TEXT NOT NULL,
                optional BOOLEAN NOT NULL DEFAULT FALSE,
                PRIMARY KEY(skill_id, dependency_skill_id),
                FOREIGN KEY(skill_id) REFERENCES skills(skill_id) ON DELETE CASCADE
            );
            
            -- Registry events log (A5.8)
            CREATE TABLE IF NOT EXISTS registry_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_type TEXT NOT NULL,
                skill_id TEXT,
                event_data TEXT NOT NULL, -- JSON
                timestamp TEXT NOT NULL
            );
            
            -- Indexes for fast searches (A5.9)
            CREATE INDEX IF NOT EXISTS idx_skills_state ON skills(state);
            CREATE INDEX IF NOT EXISTS idx_skills_publisher ON skills(publisher);
            CREATE INDEX IF NOT EXISTS idx_skills_category ON skills(category);
            CREATE INDEX IF NOT EXISTS idx_skills_risk_level ON skills(risk_level);
            CREATE INDEX IF NOT EXISTS idx_skills_trust_tier ON skills(trust_tier);
            CREATE INDEX IF NOT EXISTS idx_skills_resource_class ON skills(resource_class);
            CREATE INDEX IF NOT EXISTS idx_skills_version ON skills(version);
            CREATE INDEX IF NOT EXISTS idx_health_status ON skill_health(status);
            CREATE INDEX IF NOT EXISTS idx_stats_usage_count ON skill_statistics(usage_count);
            CREATE INDEX IF NOT EXISTS idx_events_skill_id ON registry_events(skill_id);
            CREATE INDEX IF NOT EXISTS idx_events_timestamp ON registry_events(timestamp);
            "#,
        )
        .map_err(RegistryError::Db)?;

        Ok(())
    }

    /// Run every migration between the DB's current `PRAGMA user_version`
    /// and `SCHEMA_VERSION`, in order, each inside its own transaction. This
    /// is the real fix for the task-19/R19 finding: `CREATE TABLE IF NOT
    /// EXISTS` never adds columns to an existing older-schema database — a
    /// versioned, additive `ALTER TABLE` migration does.
    ///
    /// Safety: every migration here is additive only (`ALTER TABLE ... ADD
    /// COLUMN` with a default), matching the frozen A0-A9 no-redesign rule —
    /// no column is ever dropped or renamed, no table is ever recreated.
    fn run_migrations(&self) -> Result<(), RegistryError> {
        let db = self.db.lock().unwrap();

        let current_version: i64 = db
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(RegistryError::Db)?;

        if current_version >= SCHEMA_VERSION {
            return Ok(());
        }

        for migration in MIGRATIONS.iter() {
            if migration.version <= current_version {
                continue;
            }
            let tx = db.unchecked_transaction().map_err(RegistryError::Db)?;
            (migration.apply)(&tx).map_err(RegistryError::Db)?;
            tx.pragma_update(None, "user_version", migration.version)
                .map_err(RegistryError::Db)?;
            tx.commit().map_err(RegistryError::Db)?;
            tracing::info!(
                version = migration.version,
                description = migration.description,
                "[OpenClaw registry] schema migration applied"
            );
        }

        Ok(())
    }

    /// Subscribe to registry events (A5.8).
    pub fn subscribe_events(&self) -> broadcast::Receiver<RegistryEvent> {
        self.event_sender.subscribe()
    }

    /// Install skill with complete metadata (A5.1 + A5.4).
    pub fn install_skill(&self, metadata: &SkillMetadata) -> Result<(), RegistryError> {
        let db = self.db.lock().unwrap();
        let tx = db.unchecked_transaction().map_err(RegistryError::Db)?;
        let now = chrono::Utc::now().to_rfc3339();

        // Installing a skill means it is (at least) Installed. Pre-install discovery
        // states are bumped to Installed; already-advanced states (Enabled, etc.) are
        // honored so callers can seed enabled skills directly.
        let effective_state = match metadata.state {
            SkillState::Discovered | SkillState::Verified => SkillState::Installed,
            other => other,
        };

        // Insert core metadata
        tx.execute(
            r#"
            INSERT OR REPLACE INTO skills (
                skill_id, name, description, publisher, version, category,
                discovery_source, discovered_at, capabilities, runtime_requirements,
                risk_level, resource_class, tags, categories, semantic_version,
                dependencies, compatibility_requirements, trust_tier, content_hash,
                signature, granted_capabilities, bundle_path, manifest_toml, state,
                state_changed_at, created_at, updated_at, input_schema
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28
            )
            "#,
            params![
                metadata.skill_id,
                metadata.name,
                metadata.description,
                metadata.publisher,
                metadata.version,
                metadata.category,
                serde_json::to_string(&metadata.discovery_source).unwrap(),
                metadata.discovered_at.to_rfc3339(),
                serde_json::to_string(&metadata.capabilities).unwrap(),
                metadata.runtime_requirements,
                metadata.risk_level.as_str(),
                metadata.resource_class.as_str(),
                serde_json::to_string(&metadata.tags).unwrap(),
                serde_json::to_string(&metadata.categories).unwrap(),
                metadata.semantic_version,
                serde_json::to_string(&metadata.dependencies).unwrap(),
                serde_json::to_string(&metadata.compatibility_requirements).unwrap(),
                metadata.trust_tier.as_str(),
                metadata.content_hash,
                metadata.signature,
                serde_json::to_string(&metadata.granted_capabilities).unwrap(),
                metadata.bundle_path,
                metadata.manifest_toml,
                effective_state.as_str(),
                metadata.state_changed_at.to_rfc3339(),
                now,
                now,
                metadata.input_schema.as_ref().map(|s| s.to_string()),
            ],
        )
        .map_err(RegistryError::Db)?;

        // Initialize health tracking (A5.6)
        tx.execute(
            "INSERT OR REPLACE INTO skill_health (skill_id, status, last_check, failure_reasons)
             VALUES (?1, ?2, ?3, ?4)",
            params![metadata.skill_id, HealthStatus::Healthy.as_str(), now, "[]"],
        )
        .map_err(RegistryError::Db)?;

        // Initialize statistics (A5.7)
        tx.execute(
            "INSERT OR REPLACE INTO skill_statistics (skill_id, installation_date, publisher_trust_score)
             VALUES (?1, ?2, ?3)",
            params![
                metadata.skill_id,
                now,
                0.5 // TODO: Calculate publisher trust
            ],
        ).map_err(RegistryError::Db)?;

        // Insert dependencies (A5.10)
        for dep in &metadata.dependencies {
            tx.execute(
                "INSERT OR REPLACE INTO skill_dependencies (skill_id, dependency_skill_id, version_requirement, optional)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    metadata.skill_id,
                    dep.skill_id,
                    dep.version_requirement,
                    dep.optional
                ],
            ).map_err(RegistryError::Db)?;
        }

        tx.commit().map_err(RegistryError::Db)?;

        // Emit event (A5.8)
        let event = RegistryEvent::Installed {
            skill_id: metadata.skill_id.clone(),
            version: metadata.version.clone(),
        };
        let _ = self.event_sender.send(event);

        Ok(())
    }

    /// Production search API (A5.9).
    pub fn search_skills(&self, query: &SkillQuery) -> Result<Vec<SkillMetadata>, RegistryError> {
        let db = self.db.lock().unwrap();

        let mut sql = "SELECT * FROM skills WHERE 1=1".to_string();
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        // Build dynamic query
        if let Some(slug) = &query.slug {
            sql.push_str(" AND skill_id = ?");
            params.push(Box::new(slug.clone()));
        }

        if let Some(publisher) = &query.publisher {
            sql.push_str(" AND publisher = ?");
            params.push(Box::new(publisher.clone()));
        }

        if let Some(desc) = &query.description_contains {
            sql.push_str(" AND description LIKE ?");
            params.push(Box::new(format!("%{}%", desc)));
        }

        if let Some(risk) = &query.risk_level {
            sql.push_str(" AND risk_level = ?");
            params.push(Box::new(risk.as_str().to_string()));
        }

        if let Some(state) = &query.state {
            sql.push_str(" AND state = ?");
            params.push(Box::new(state.as_str().to_string()));
        }

        if query.enabled_only {
            sql.push_str(" AND state = 'enabled'");
        }

        // Execute query
        let mut stmt = db.prepare(&sql).map_err(RegistryError::Db)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

        let rows = stmt
            .query_map(param_refs.as_slice(), |row| self.row_to_metadata(row))
            .map_err(RegistryError::Db)?;

        let mut results = Vec::new();
        for row in rows {
            match row {
                Ok(metadata) => results.push(metadata),
                // Do NOT silently swallow parse failures. A dropped row here
                // previously hid the registry-empty bug (see `row_to_metadata`).
                // Surface it so a schema/parse regression is observable instead
                // of manifesting as a mysteriously empty registry.
                Err(e) => tracing::warn!(
                    error = %e,
                    "[OpenClaw registry] dropping a skills row that failed to parse — \
                     registry results may be incomplete"
                ),
            }
        }

        Ok(results)
    }

    /// Get all enabled skills for router (A5.9 + A5.11).
    pub fn get_enabled_skills(&self) -> Result<Vec<SkillMetadata>, RegistryError> {
        let query = SkillQuery {
            slug: None,
            publisher: None,
            description_contains: None,
            tags: Vec::new(),
            categories: Vec::new(),
            capabilities: Vec::new(),
            runtime_requirements: None,
            risk_level: None,
            state: Some(SkillState::Enabled),
            enabled_only: true,
        };

        self.search_skills(&query)
    }
    /// Backfill/update a skill's authoritative input schema (RC2 registry sync).
    /// Used to bring pre-`input_schema`-column rows (e.g. an existing user's
    /// curated calculator) in line with what the container advertises, so
    /// schema-driven argument generation has the real schema.
    pub fn set_input_schema(
        &self,
        skill_id: &str,
        schema: &serde_json::Value,
    ) -> Result<(), RegistryError> {
        let db = self.db.lock().unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        let rows = db
            .execute(
                "UPDATE skills SET input_schema = ?1, updated_at = ?2 WHERE skill_id = ?3",
                params![schema.to_string(), now, skill_id],
            )
            .map_err(RegistryError::Db)?;
        if rows == 0 {
            return Err(RegistryError::NotFound(skill_id.to_string()));
        }
        Ok(())
    }

    /// Transition skill state (A5.1).
    pub fn set_skill_state(
        &self,
        skill_id: &str,
        new_state: SkillState,
    ) -> Result<(), RegistryError> {
        let db = self.db.lock().unwrap();
        let now = chrono::Utc::now().to_rfc3339();

        let rows = db.execute(
            "UPDATE skills SET state = ?1, state_changed_at = ?2, updated_at = ?3 WHERE skill_id = ?4",
            params![new_state.as_str(), now, now, skill_id],
        ).map_err(RegistryError::Db)?;

        if rows == 0 {
            return Err(RegistryError::NotFound(skill_id.to_string()));
        }

        // Emit appropriate event
        let event = match new_state {
            SkillState::Enabled => RegistryEvent::Enabled {
                skill_id: skill_id.to_string(),
            },
            SkillState::Disabled => RegistryEvent::Disabled {
                skill_id: skill_id.to_string(),
            },
            SkillState::Broken => RegistryEvent::Broken {
                skill_id: skill_id.to_string(),
                reason: "state transition".to_string(),
            },
            SkillState::Recovering => RegistryEvent::Recovered {
                skill_id: skill_id.to_string(),
            },
            SkillState::Deprecated => RegistryEvent::Deprecated {
                skill_id: skill_id.to_string(),
            },
            SkillState::Removed => RegistryEvent::Removed {
                skill_id: skill_id.to_string(),
            },
            _ => return Ok(()),
        };
        let _ = self.event_sender.send(event);

        Ok(())
    }

    /// Get skill metadata by ID.
    pub fn get_skill(&self, skill_id: &str) -> Result<SkillMetadata, RegistryError> {
        let db = self.db.lock().unwrap();

        let metadata = db
            .query_row(
                "SELECT * FROM skills WHERE skill_id = ?1",
                params![skill_id],
                |row| self.row_to_metadata(row),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    RegistryError::NotFound(skill_id.to_string())
                }
                other => RegistryError::Db(other),
            })?;

        Ok(metadata)
    }

    /// Helper method to convert row to metadata.
    ///
    // (see `schema_from_descriptor_params` free fn below for the params→schema helper)
    ///
    /// PRODUCTION BUG FIX (registry-empty / "no enabled skills found in
    /// registry"): columns are read by NAME, never by positional index.
    /// `SELECT *` returns columns in the table's PHYSICAL order, and
    /// `ALTER TABLE ADD COLUMN` (used by schema migration 1 to add
    /// `granted_capabilities`) APPENDS the new column at the END. So an
    /// existing user's upgraded `skills.db` has `granted_capabilities` at the
    /// last index, while a freshly-created DB has it mid-table — the two
    /// physical orders differ. The previous index-based parser hard-coded the
    /// fresh-schema order, so on any upgraded DB `row.get::<_,String>(20)` read
    /// `bundle_path` (NULL) and returned `InvalidColumnType`, causing
    /// `row_to_metadata` to fail for EVERY row. Those failures were then
    /// silently swallowed by `search_skills`' `if let Ok(..)`, so every
    /// enabled curated skill vanished and the router reported an empty
    /// registry. Reading by name is order-independent and fixes both fresh
    /// and migrated databases.
    fn row_to_metadata(&self, row: &Row) -> Result<SkillMetadata, rusqlite::Error> {
        Ok(SkillMetadata {
            skill_id: row.get("skill_id")?,
            name: row.get("name")?,
            description: row.get("description")?,
            publisher: row.get("publisher")?,
            version: row.get("version")?,
            category: row.get("category")?,
            discovery_source: serde_json::from_str(&row.get::<_, String>("discovery_source")?)
                .unwrap(),
            discovered_at: chrono::DateTime::parse_from_rfc3339(
                &row.get::<_, String>("discovered_at")?,
            )
            .unwrap()
            .with_timezone(&chrono::Utc),
            capabilities: serde_json::from_str(&row.get::<_, String>("capabilities")?).unwrap(),
            runtime_requirements: row.get("runtime_requirements")?,
            risk_level: match row.get::<_, String>("risk_level")?.as_str() {
                "green" => RiskLevel::Green,
                "yellow" => RiskLevel::Yellow,
                "red" => RiskLevel::Red,
                "black" => RiskLevel::Black,
                _ => RiskLevel::Green,
            },
            resource_class: row.get::<_, String>("resource_class")?.parse().unwrap(),
            tags: serde_json::from_str(&row.get::<_, String>("tags")?).unwrap(),
            categories: serde_json::from_str(&row.get::<_, String>("categories")?).unwrap(),
            semantic_version: row.get("semantic_version")?,
            dependencies: serde_json::from_str(&row.get::<_, String>("dependencies")?).unwrap(),
            compatibility_requirements: serde_json::from_str(
                &row.get::<_, String>("compatibility_requirements")?,
            )
            .unwrap(),
            trust_tier: row.get::<_, String>("trust_tier")?.parse().unwrap(),
            content_hash: row.get("content_hash")?,
            signature: row.get("signature")?,
            granted_capabilities: serde_json::from_str(
                &row.get::<_, String>("granted_capabilities")?,
            )
            .unwrap_or_default(),
            bundle_path: row.get("bundle_path")?,
            manifest_toml: row.get("manifest_toml")?,
            input_schema: row
                .get::<_, Option<String>>("input_schema")?
                .and_then(|s| serde_json::from_str(&s).ok()),
            state: match row.get::<_, String>("state")?.as_str() {
                "discovered" => SkillState::Discovered,
                "verified" => SkillState::Verified,
                "installed" => SkillState::Installed,
                "enabled" => SkillState::Enabled,
                "disabled" => SkillState::Disabled,
                "deprecated" => SkillState::Deprecated,
                "removed" => SkillState::Removed,
                "broken" => SkillState::Broken,
                "recovering" => SkillState::Recovering,
                _ => SkillState::Discovered,
            },
            state_changed_at: chrono::DateTime::parse_from_rfc3339(
                &row.get::<_, String>("state_changed_at")?,
            )
            .unwrap()
            .with_timezone(&chrono::Utc),
        })
    }
}

impl HealthStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Broken => "broken",
            Self::Deprecated => "deprecated",
            Self::Disabled => "disabled",
            Self::VerificationFailed => "verification_failed",
            Self::MissingDependency => "missing_dependency",
            Self::RuntimeUnavailable => "runtime_unavailable",
        }
    }
}

/// Legacy compatibility layer - redirect to ProductionSkillRegistry.
pub type SkillRegistry = ProductionSkillRegistry;

impl ProductionSkillRegistry {
    /// Legacy compatibility: redirect to new().
    pub fn open(db_path: &Path) -> Result<Self, RegistryError> {
        Self::new(db_path)
    }

    /// Legacy compatibility: install via metadata conversion.
    pub fn install(&self, skill: &SkillDescriptor) -> Result<(), RegistryError> {
        let metadata = SkillMetadata {
            skill_id: skill.skill_id.clone(),
            name: skill.name.clone(),
            description: skill.description.clone(),
            publisher: "legacy".to_string(),
            version: "1.0.0".to_string(),
            category: skill.category.clone(),
            discovery_source: DiscoverySource::Bundled {
                path: "legacy".to_string(),
            },
            discovered_at: skill.installed_at,
            capabilities: skill.capabilities.clone(),
            runtime_requirements: skill.resource_profile.resource_class.as_str().to_string(),
            risk_level: skill.risk_level,
            resource_class: skill.resource_profile.resource_class,
            tags: Vec::new(),
            categories: vec![skill.category.clone()],
            semantic_version: "1.0.0".to_string(),
            dependencies: Vec::new(),
            compatibility_requirements: Vec::new(),
            trust_tier: skill.trust_tier,
            content_hash: "legacy".to_string(),
            signature: None,
            granted_capabilities: skill.granted.clone(),
            bundle_path: None,
            manifest_toml: None,
            input_schema: schema_from_descriptor_params(&skill.parameters),
            state: SkillState::Installed,
            state_changed_at: chrono::Utc::now(),
        };

        self.install_skill(&metadata)?;

        // Set appropriate state based on old status
        let state = match skill.status {
            SkillStatus::Active => SkillState::Enabled,
            SkillStatus::Disabled => SkillState::Disabled,
            SkillStatus::StaleDisabled => SkillState::Disabled,
            SkillStatus::PendingApproval => SkillState::Verified,
            SkillStatus::Quarantined => SkillState::Broken,
        };

        self.set_skill_state(&skill.skill_id, state)?;

        Ok(())
    }

    /// Legacy compatibility: get via search.
    /// REAL BUG FOUND + FIXED (task 5, R6.2/R6.5 validation): this previously
    /// returned `Ok(..)` for a `Removed`-state skill (it did a plain row
    /// lookup with NO state filter, and hardcoded `status: SkillStatus::Active`
    /// regardless of the real state). That broke the "uninstall/rollback
    /// removes the skill" contract callers rely on — e.g.
    /// `BundleInstaller::uninstall`/`rollback` transition a skill to
    /// `Removed` via `set_skill_state`, and callers (including the crate's own
    /// tests, `uninstall_removes_everything` / `activation_failure_triggers_rollback`
    /// in `openclaw_bundle_tests.rs`) then assert `registry.get(slug).is_err()` —
    /// which FAILED before this fix, confirmed by running those exact tests.
    /// Now maps the real `state` and treats `Removed` as not-found, matching
    /// the actual semantics `uninstall()`/`rollback()` rely on.
    pub fn get(&self, skill_id: &str) -> Result<SkillDescriptor, RegistryError> {
        let metadata = self.get_skill(skill_id)?;

        if matches!(metadata.state, SkillState::Removed) {
            return Err(RegistryError::NotFound(skill_id.to_string()));
        }

        let status = match metadata.state {
            SkillState::Enabled => SkillStatus::Active,
            SkillState::Disabled | SkillState::Deprecated => SkillStatus::Disabled,
            SkillState::Broken | SkillState::Recovering => SkillStatus::Quarantined,
            _ => SkillStatus::Active,
        };

        Ok(SkillDescriptor {
            skill_id: metadata.skill_id,
            name: metadata.name,
            description: metadata.description,
            category: metadata.category.clone(),
            parameters: metadata
                .input_schema
                .clone()
                .unwrap_or_else(|| serde_json::json!({})),
            risk_level: metadata.risk_level,
            network_policy: metadata.capabilities.to_network_policy(),
            resource_profile: ResourceProfile::for_category(&metadata.category),
            capabilities: metadata.capabilities,
            granted: metadata.granted_capabilities.clone(),
            trust_tier: metadata.trust_tier,
            source: SkillSource::Bundled,
            installed_at: metadata.discovered_at,
            last_used_at: None, // TODO: Extract from statistics
            use_count: 0,       // TODO: Extract from statistics
            status,
        })
    }

    /// Legacy compatibility: list active skills.
    pub fn list_active(&self) -> Result<Vec<SkillDescriptor>, RegistryError> {
        let metadata_list = self.get_enabled_skills()?;
        let mut skills = Vec::new();

        for metadata in metadata_list {
            let skill = SkillDescriptor {
                skill_id: metadata.skill_id,
                name: metadata.name,
                description: metadata.description,
                category: metadata.category.clone(),
                parameters: metadata
                    .input_schema
                    .clone()
                    .unwrap_or_else(|| serde_json::json!({})),
                risk_level: metadata.risk_level,
                network_policy: metadata.capabilities.to_network_policy(),
                resource_profile: ResourceProfile::for_category(&metadata.category),
                capabilities: metadata.capabilities,
                granted: Vec::new(),
                trust_tier: metadata.trust_tier,
                source: SkillSource::Bundled,
                installed_at: metadata.discovered_at,
                last_used_at: None,
                use_count: 0,
                status: SkillStatus::Active,
            };
            skills.push(skill);
        }

        Ok(skills)
    }

    /// Legacy compatibility: bundle provenance.
    pub fn get_provenance(
        &self,
        skill_id: &str,
    ) -> Result<Option<BundleProvenance>, RegistryError> {
        let metadata = self.get_skill(skill_id)?;

        Ok(Some(BundleProvenance {
            publisher: metadata.publisher,
            version: metadata.version,
            content_hash: metadata.content_hash,
            signature: metadata.signature.unwrap_or_default(),
            manifest_toml: metadata.manifest_toml.unwrap_or_default(),
            bundle_path: metadata.bundle_path.unwrap_or_default(),
        }))
    }

    /// Legacy compatibility: install bundle.
    pub fn install_bundle(
        &self,
        skill: &SkillDescriptor,
        prov: &BundleProvenance,
    ) -> Result<(), RegistryError> {
        let metadata = SkillMetadata {
            skill_id: skill.skill_id.clone(),
            name: skill.name.clone(),
            description: skill.description.clone(),
            publisher: prov.publisher.clone(),
            version: prov.version.clone(),
            category: skill.category.clone(),
            discovery_source: DiscoverySource::InstalledBundle {
                bundle_path: prov.bundle_path.clone(),
            },
            discovered_at: skill.installed_at,
            capabilities: skill.capabilities.clone(),
            runtime_requirements: skill.resource_profile.resource_class.as_str().to_string(),
            risk_level: skill.risk_level,
            resource_class: skill.resource_profile.resource_class,
            tags: Vec::new(),
            categories: vec![skill.category.clone()],
            semantic_version: prov.version.clone(),
            dependencies: Vec::new(),
            compatibility_requirements: Vec::new(),
            trust_tier: skill.trust_tier,
            content_hash: prov.content_hash.clone(),
            signature: Some(prov.signature.clone()),
            granted_capabilities: skill.granted.clone(),
            bundle_path: Some(prov.bundle_path.clone()),
            manifest_toml: Some(prov.manifest_toml.clone()),
            input_schema: schema_from_descriptor_params(&skill.parameters),
            state: SkillState::Installed,
            state_changed_at: chrono::Utc::now(),
        };

        self.install_skill(&metadata)
    }

    /// Legacy compatibility: uninstall skill.
    pub fn uninstall(&self, skill_id: &str) -> Result<(), RegistryError> {
        self.set_skill_state(skill_id, SkillState::Removed)
    }

    /// Legacy compatibility: toggle enabled/disabled.
    pub fn toggle(&self, skill_id: &str, enabled: bool) -> Result<(), RegistryError> {
        let new_state = if enabled {
            SkillState::Enabled
        } else {
            SkillState::Disabled
        };
        self.set_skill_state(skill_id, new_state)
    }

    /// Legacy compatibility: installed refs.
    pub fn installed_refs(&self) -> Result<Vec<(String, String, String)>, RegistryError> {
        let db = self.db.lock().unwrap();
        let mut stmt = db
            .prepare("SELECT skill_id, version, publisher FROM skills")
            .map_err(RegistryError::Db)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(RegistryError::Db)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Legacy compatibility: list all installed skills.
    pub fn list_installed(&self) -> Result<Vec<SkillDescriptor>, RegistryError> {
        let query = SkillQuery {
            slug: None,
            publisher: None,
            description_contains: None,
            tags: vec![],
            categories: vec![],
            capabilities: vec![],
            runtime_requirements: None,
            risk_level: None,
            state: None,
            enabled_only: false,
        };

        let metadata_list = self.search_skills(&query)?;
        let mut skills = Vec::new();

        for metadata in metadata_list {
            let skill = SkillDescriptor {
                skill_id: metadata.skill_id,
                name: metadata.name,
                description: metadata.description,
                category: metadata.category.clone(),
                parameters: metadata
                    .input_schema
                    .clone()
                    .unwrap_or_else(|| serde_json::json!({})),
                risk_level: metadata.risk_level,
                network_policy: metadata.capabilities.to_network_policy(),
                resource_profile: ResourceProfile::for_category(&metadata.category),
                capabilities: metadata.capabilities,
                granted: Vec::new(),
                trust_tier: metadata.trust_tier,
                source: SkillSource::Bundled,
                installed_at: metadata.discovered_at,
                last_used_at: None,
                use_count: 0,
                status: SkillStatus::Active,
            };
            skills.push(skill);
        }

        Ok(skills)
    }

    /// Legacy compatibility: record invocation.
    pub fn record_invocation(&self, skill_id: &str) -> Result<(), RegistryError> {
        self.record_execution(skill_id, true, 100, 0.5)
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
    #[error("dependency conflict: {0}")]
    DependencyConflict(String),
    #[error("version conflict: {0}")]
    VersionConflict(String),
    #[error("discovery error: {0}")]
    Discovery(String),
}

impl ProductionSkillRegistry {
    /// Discovery Engine Implementation (A5.2)
    /// Discovers skills from all configured sources automatically.
    pub fn discover_all_skills(&self) -> Result<Vec<SkillMetadata>, RegistryError> {
        let mut discovered = Vec::new();

        // Bundled skills
        discovered.extend(self.discover_bundled_skills()?);

        // Installed bundles
        discovered.extend(self.discover_installed_bundles()?);

        // Workspace skills
        discovered.extend(self.discover_workspace_skills()?);

        // Developer skills
        discovered.extend(self.discover_developer_skills()?);

        // Store discovered skills in DB
        for skill in &discovered {
            // Only install if not already discovered
            if self.get_skill(&skill.skill_id).is_err() {
                let metadata = skill.clone();
                self.install_skill(&metadata)?;
                self.set_skill_state(&skill.skill_id, SkillState::Discovered)?;
            }
        }

        Ok(discovered)
    }

    fn discover_bundled_skills(&self) -> Result<Vec<SkillMetadata>, RegistryError> {
        // TODO: Scan bundled skills from embedded resources or known paths
        Ok(vec![])
    }

    fn discover_installed_bundles(&self) -> Result<Vec<SkillMetadata>, RegistryError> {
        use std::path::Path;
        let bundles_dir = Path::new("./bundles");
        if !bundles_dir.exists() {
            return Ok(vec![]);
        }

        let mut skills = Vec::new();

        // Scan .ocskill bundles
        if let Ok(entries) = std::fs::read_dir(bundles_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("ocskill") {
                    // TODO: Parse bundle and extract skill metadata
                    // For now, create placeholder
                    let skill_id = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown")
                        .to_string();

                    skills.push(SkillMetadata {
                        skill_id: format!("bundle_{}", skill_id),
                        name: skill_id.clone(),
                        description: format!("Skill from bundle {}", skill_id),
                        publisher: "bundle".to_string(),
                        version: "1.0.0".to_string(),
                        category: "general".to_string(),
                        discovery_source: DiscoverySource::InstalledBundle {
                            bundle_path: path.to_string_lossy().to_string(),
                        },
                        discovered_at: chrono::Utc::now(),
                        capabilities: SkillCapabilities::default(),
                        runtime_requirements: "docker".to_string(),
                        risk_level: RiskLevel::Yellow,
                        resource_class: ResourceClass::Medium,
                        tags: vec!["bundled".to_string()],
                        categories: vec!["general".to_string()],
                        semantic_version: "1.0.0".to_string(),
                        dependencies: vec![],
                        compatibility_requirements: vec![],
                        trust_tier: TrustTier::Local,
                        content_hash: "bundle_hash".to_string(),
                        signature: None,
                        granted_capabilities: Vec::new(),
                        bundle_path: Some(path.to_string_lossy().to_string()),
                        manifest_toml: None,
                        input_schema: None,
                        state: SkillState::Discovered,
                        state_changed_at: chrono::Utc::now(),
                    });
                }
            }
        }

        Ok(skills)
    }

    fn discover_workspace_skills(&self) -> Result<Vec<SkillMetadata>, RegistryError> {
        use std::path::Path;
        let workspace_skills = Path::new("./workspace-skills");
        if !workspace_skills.exists() {
            return Ok(vec![]);
        }

        // TODO: Scan workspace for SKILL.md files
        Ok(vec![])
    }

    fn discover_developer_skills(&self) -> Result<Vec<SkillMetadata>, RegistryError> {
        use std::path::Path;
        let dev_skills = Path::new("./dev-skills");
        if !dev_skills.exists() {
            return Ok(vec![]);
        }

        // TODO: Scan developer skill directories
        Ok(vec![])
    }

    /// Version Management (A5.5)
    /// Upgrade skill to new version with compatibility checks.
    pub fn upgrade_skill(&self, skill_id: &str, new_version: &str) -> Result<(), RegistryError> {
        let current = self.get_skill(skill_id)?;

        // Check version compatibility
        if !self.is_version_compatible(&current.semantic_version, new_version) {
            return Err(RegistryError::VersionConflict(format!(
                "Cannot upgrade {} from {} to {}",
                skill_id, current.semantic_version, new_version
            )));
        }

        // Check dependency compatibility
        self.validate_dependencies_for_version(skill_id, new_version)?;

        // Update version
        let db = self.db.lock().unwrap();
        let now = chrono::Utc::now().to_rfc3339();

        db.execute(
            "UPDATE skills SET version = ?1, semantic_version = ?2, updated_at = ?3 WHERE skill_id = ?4",
            params![new_version, new_version, now, skill_id],
        ).map_err(RegistryError::Db)?;

        // Emit event
        let event = RegistryEvent::Updated {
            skill_id: skill_id.to_string(),
            old_version: current.version,
            new_version: new_version.to_string(),
        };
        let _ = self.event_sender.send(event);

        Ok(())
    }

    /// Downgrade skill to previous version.
    pub fn downgrade_skill(
        &self,
        skill_id: &str,
        target_version: &str,
    ) -> Result<(), RegistryError> {
        // Similar to upgrade but with downgrade validation
        self.upgrade_skill(skill_id, target_version)
    }

    /// Rollback to last known good version.
    pub fn rollback_skill(&self, _skill_id: &str) -> Result<(), RegistryError> {
        // TODO: Implement version history tracking and rollback
        Err(RegistryError::VersionConflict(
            "Rollback not implemented".to_string(),
        ))
    }

    /// Real semver compatibility for a version change (upgrade/downgrade).
    /// Both endpoints must be valid semver and must differ (a "change" to the
    /// same version is a no-op and rejected). Uses the `semver` crate — no more
    /// always-true stub. Bidirectional by design: `downgrade_skill` reuses this,
    /// so we do not require target > current here (direction is the caller's
    /// intent); we only reject unparseable or identical versions.
    fn is_version_compatible(&self, current: &str, target: &str) -> bool {
        let (Ok(cur), Ok(tgt)) = (
            semver::Version::parse(current.trim()),
            semver::Version::parse(target.trim()),
        ) else {
            return false;
        };
        cur != tgt
    }

    fn validate_dependencies_for_version(
        &self,
        _skill_id: &str,
        _version: &str,
    ) -> Result<(), RegistryError> {
        // TODO: Check if all dependencies are satisfied for this version
        Ok(())
    }

    /// Health Tracking (A5.6)
    /// Update skill health status with automatic detection.
    pub fn update_skill_health(
        &self,
        skill_id: &str,
        status: HealthStatus,
        reason: Option<String>,
    ) -> Result<(), RegistryError> {
        let now = chrono::Utc::now().to_rfc3339();

        // Scope the DB lock so it is released before we call set_skill_state/get_skill,
        // which re-lock self.db (std Mutex is NOT re-entrant → would deadlock).
        {
            let db = self.db.lock().unwrap();

            // Get current health
            let current_health: Result<(HealthStatus, u32), rusqlite::Error> = db.query_row(
                "SELECT status, failure_count FROM skill_health WHERE skill_id = ?1",
                params![skill_id],
                |row| {
                    let status_str: String = row.get(0)?;
                    let status = match status_str.as_str() {
                        "healthy" => HealthStatus::Healthy,
                        "broken" => HealthStatus::Broken,
                        _ => HealthStatus::Healthy,
                    };
                    Ok((status, row.get(1)?))
                },
            );

            let (failure_count, recovery_attempts) = match current_health {
                Ok((current_status, count)) => {
                    if status == HealthStatus::Broken && current_status != HealthStatus::Broken {
                        (count + 1, 0)
                    } else if status == HealthStatus::Healthy
                        && current_status == HealthStatus::Broken
                    {
                        (0, 0)
                    } else {
                        (count, 0)
                    }
                }
                Err(_) => (0, 0),
            };

            let failure_reasons = if let Some(reason) = reason {
                serde_json::to_string(&vec![reason]).unwrap()
            } else {
                "[]".to_string()
            };

            db.execute(
                "INSERT OR REPLACE INTO skill_health (skill_id, status, last_check, failure_count, failure_reasons, recovery_attempts)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    skill_id,
                    status.as_str(),
                    now,
                    failure_count,
                    failure_reasons,
                    recovery_attempts,
                ],
            ).map_err(RegistryError::Db)?;
        } // db lock released here

        // Auto-transition skill state based on health (re-locks self.db internally)
        match status {
            HealthStatus::Broken => {
                self.set_skill_state(skill_id, SkillState::Broken)?;
            }
            HealthStatus::Healthy => {
                // Recover from broken state if currently broken
                let current_skill = self.get_skill(skill_id)?;
                if matches!(current_skill.state, SkillState::Broken) {
                    self.set_skill_state(skill_id, SkillState::Enabled)?;
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// Perform health check on all skills.
    pub fn health_check_all(&self) -> Result<(), RegistryError> {
        let skills = self.search_skills(&SkillQuery {
            slug: None,
            publisher: None,
            description_contains: None,
            tags: vec![],
            categories: vec![],
            capabilities: vec![],
            runtime_requirements: None,
            risk_level: None,
            state: None,
            enabled_only: false,
        })?;

        for skill in skills {
            // Basic health check - verify skill files exist
            if let Some(bundle_path) = &skill.bundle_path {
                if !std::path::Path::new(bundle_path).exists() {
                    self.update_skill_health(
                        &skill.skill_id,
                        HealthStatus::Broken,
                        Some("Bundle file missing".to_string()),
                    )?;
                } else {
                    self.update_skill_health(&skill.skill_id, HealthStatus::Healthy, None)?;
                }
            }
        }

        Ok(())
    }

    /// Statistics Tracking (A5.7)
    /// Record skill execution statistics.
    pub fn record_execution(
        &self,
        skill_id: &str,
        success: bool,
        latency_ms: u64,
        resource_usage: f64,
    ) -> Result<(), RegistryError> {
        let db = self.db.lock().unwrap();
        let now = chrono::Utc::now().to_rfc3339();

        // Get current stats
        let current_stats = db.query_row(
            "SELECT usage_count, success_rate, failure_rate, average_latency_ms, average_resource_usage FROM skill_statistics WHERE skill_id = ?1",
            params![skill_id],
            |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, f64>(3)?,
                    row.get::<_, f64>(4)?,
                ))
            }
        ).unwrap_or((0, 0.0, 0.0, 0.0, 0.0));

        let (usage_count, old_success_rate, _old_failure_rate, old_latency, old_resource) =
            current_stats;
        let new_usage_count = usage_count + 1;

        // Calculate rolling averages
        let new_success_rate = if success {
            (old_success_rate * usage_count as f64 + 1.0) / new_usage_count as f64
        } else {
            (old_success_rate * usage_count as f64) / new_usage_count as f64
        };

        let new_failure_rate = 1.0 - new_success_rate;

        let new_latency =
            (old_latency * usage_count as f64 + latency_ms as f64) / new_usage_count as f64;
        let new_resource =
            (old_resource * usage_count as f64 + resource_usage) / new_usage_count as f64;

        db.execute(
            "UPDATE skill_statistics SET 
             usage_count = ?1, last_execution = ?2, success_rate = ?3, failure_rate = ?4,
             average_latency_ms = ?5, average_resource_usage = ?6
             WHERE skill_id = ?7",
            params![
                new_usage_count,
                now,
                new_success_rate,
                new_failure_rate,
                new_latency,
                new_resource,
                skill_id,
            ],
        )
        .map_err(RegistryError::Db)?;

        // Emit execution event
        let event = RegistryEvent::ExecutionCompleted {
            skill_id: skill_id.to_string(),
            invocation_id: uuid::Uuid::new_v4().to_string(),
            success,
            latency_ms,
        };
        let _ = self.event_sender.send(event);

        Ok(())
    }

    /// Get skill statistics.
    pub fn get_skill_statistics(&self, skill_id: &str) -> Result<SkillStatistics, RegistryError> {
        let db = self.db.lock().unwrap();

        let stats = db
            .query_row(
                "SELECT * FROM skill_statistics WHERE skill_id = ?1",
                params![skill_id],
                |row| {
                    Ok(SkillStatistics {
                        skill_id: row.get(0)?,
                        usage_count: row.get(1)?,
                        last_execution: row
                            .get::<_, Option<String>>(2)?
                            .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                            .map(|dt| dt.with_timezone(&chrono::Utc)),
                        success_rate: row.get(3)?,
                        failure_rate: row.get(4)?,
                        average_latency_ms: row.get(5)?,
                        average_resource_usage: row.get(6)?,
                        installation_date: chrono::DateTime::parse_from_rfc3339(
                            &row.get::<_, String>(7)?,
                        )
                        .unwrap()
                        .with_timezone(&chrono::Utc),
                        publisher_trust_score: row.get(8)?,
                    })
                },
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    RegistryError::NotFound(skill_id.to_string())
                }
                other => RegistryError::Db(other),
            })?;

        Ok(stats)
    }

    /// Dependency Graph Management (A5.10)  
    /// Check for dependency conflicts before installation.
    pub fn check_dependency_conflicts(
        &self,
        skill_metadata: &SkillMetadata,
    ) -> Result<Vec<DependencyConflict>, RegistryError> {
        let mut conflicts = Vec::new();

        // Check for cyclic dependencies
        if self.would_create_cycle(&skill_metadata.skill_id, &skill_metadata.dependencies)? {
            conflicts.push(DependencyConflict {
                skill_id: skill_metadata.skill_id.clone(),
                conflicting_skill: "self".to_string(),
                conflict_type: ConflictType::CyclicDependency,
                details: "Would create dependency cycle".to_string(),
            });
        }

        // Check for missing dependencies
        for dep in &skill_metadata.dependencies {
            if self.get_skill(&dep.skill_id).is_err() && !dep.optional {
                conflicts.push(DependencyConflict {
                    skill_id: skill_metadata.skill_id.clone(),
                    conflicting_skill: dep.skill_id.clone(),
                    conflict_type: ConflictType::MissingDependency,
                    details: format!("Required dependency {} not found", dep.skill_id),
                });
            }
        }

        // Check version conflicts
        for dep in &skill_metadata.dependencies {
            if let Ok(existing) = self.get_skill(&dep.skill_id) {
                if !self.version_satisfies(&existing.semantic_version, &dep.version_requirement) {
                    conflicts.push(DependencyConflict {
                        skill_id: skill_metadata.skill_id.clone(),
                        conflicting_skill: dep.skill_id.clone(),
                        conflict_type: ConflictType::VersionIncompatible,
                        details: format!(
                            "Requires {} {} but {} is installed",
                            dep.skill_id, dep.version_requirement, existing.semantic_version
                        ),
                    });
                }
            }
        }

        Ok(conflicts)
    }

    fn would_create_cycle(
        &self,
        skill_id: &str,
        dependencies: &[SkillDependency],
    ) -> Result<bool, RegistryError> {
        // Simple cycle detection - check if any dependency eventually depends on this skill
        for dep in dependencies {
            if self.transitively_depends_on(&dep.skill_id, skill_id)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn transitively_depends_on(&self, skill_id: &str, target: &str) -> Result<bool, RegistryError> {
        if skill_id == target {
            return Ok(true);
        }

        // Get dependencies of this skill. Scope the lock so it is released BEFORE the
        // recursive calls below (std Mutex is not re-entrant → recursion would deadlock).
        let deps: Vec<String> = {
            let db = self.db.lock().unwrap();
            let mut stmt = db
                .prepare("SELECT dependency_skill_id FROM skill_dependencies WHERE skill_id = ?1")
                .map_err(RegistryError::Db)?;

            let collected: Vec<String> = stmt
                .query_map(params![skill_id], |row| Ok(row.get(0)?))
                .map_err(RegistryError::Db)?
                .filter_map(|r| r.ok())
                .collect();
            collected
        };

        // Recursively check dependencies (lock released above)
        for dep in deps {
            if self.transitively_depends_on(&dep, target)? {
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn version_satisfies(&self, installed: &str, requirement: &str) -> bool {
        // Single source of truth: the neutral semver helper in the capability
        // intelligence layer (Wave 6). A bare version string (no operator) is
        // treated as an exact-match requirement for backward compatibility with
        // manifests that pin a plain version.
        use crate::capability::intelligence::version_satisfies;
        let req = requirement.trim();
        let normalized = if req
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
        {
            format!("={req}")
        } else {
            req.to_string()
        };
        match version_satisfies(installed, &normalized) {
            Ok(v) => v,
            // Unparsable input: fall back to exact string match rather than
            // silently accepting (honest, conservative).
            Err(_) => installed.trim() == req,
        }
    }

    /// Get reverse dependencies (skills that depend on this one).
    pub fn get_reverse_dependencies(&self, skill_id: &str) -> Result<Vec<String>, RegistryError> {
        let db = self.db.lock().unwrap();
        let mut stmt = db
            .prepare("SELECT skill_id FROM skill_dependencies WHERE dependency_skill_id = ?1")
            .map_err(RegistryError::Db)?;

        let deps: Vec<String> = stmt
            .query_map(params![skill_id], |row| Ok(row.get(0)?))
            .map_err(RegistryError::Db)?
            .filter_map(|r| r.ok())
            .collect();

        Ok(deps)
    }

    /// Record registry event in persistent log.
    pub fn record_event(&self, event: &RegistryEvent) -> Result<(), RegistryError> {
        let db = self.db.lock().unwrap();
        let now = chrono::Utc::now().to_rfc3339();

        db.execute(
            "INSERT INTO registry_events (event_type, skill_id, event_data, timestamp) VALUES (?1, ?2, ?3, ?4)",
            params![
                match event {
                    RegistryEvent::Installed { .. } => "installed",
                    RegistryEvent::Updated { .. } => "updated", 
                    RegistryEvent::Enabled { .. } => "enabled",
                    RegistryEvent::Disabled { .. } => "disabled",
                    RegistryEvent::Broken { .. } => "broken",
                    RegistryEvent::Recovered { .. } => "recovered",
                    RegistryEvent::Removed { .. } => "removed",
                    RegistryEvent::Deprecated { .. } => "deprecated",
                    RegistryEvent::Verified { .. } => "verified",
                    RegistryEvent::Rejected { .. } => "rejected",
                    RegistryEvent::ExecutionStarted { .. } => "execution_started",
                    RegistryEvent::ExecutionCompleted { .. } => "execution_completed",
                },
                match event {
                    RegistryEvent::Installed { skill_id, .. } => Some(skill_id.as_str()),
                    RegistryEvent::Updated { skill_id, .. } => Some(skill_id.as_str()),
                    RegistryEvent::Enabled { skill_id } => Some(skill_id.as_str()),
                    RegistryEvent::Disabled { skill_id } => Some(skill_id.as_str()),
                    RegistryEvent::Broken { skill_id, .. } => Some(skill_id.as_str()),
                    RegistryEvent::Recovered { skill_id } => Some(skill_id.as_str()),
                    RegistryEvent::Removed { skill_id } => Some(skill_id.as_str()),
                    RegistryEvent::Deprecated { skill_id } => Some(skill_id.as_str()),
                    RegistryEvent::Verified { skill_id } => Some(skill_id.as_str()),
                    RegistryEvent::Rejected { skill_id, .. } => Some(skill_id.as_str()),
                    RegistryEvent::ExecutionStarted { skill_id, .. } => Some(skill_id.as_str()),
                    RegistryEvent::ExecutionCompleted { skill_id, .. } => Some(skill_id.as_str()),
                },
                serde_json::to_string(event).unwrap(),
                now,
            ],
        ).map_err(RegistryError::Db)?;

        Ok(())
    }
}
