//! `CapabilityGraph` — the derived capability-edge view (task 12.1, design §7.4 /
//! §13, ICP §7.4 migration 6).
//!
//! The capability graph is a **derived, rebuildable materialized view** over the
//! `capability_edges` table that task 2.1 appended to the frozen registry
//! `MIGRATIONS` pipeline (migration 6). It lives inside the SAME `skills.db` as
//! [`ProductionSkillRegistry`], [`ProfileStore`], and [`GrantStore`] — there is
//! **no second database**. `ProductionSkillRegistry` remains the single source of
//! truth; every edge here is re-derivable from `SkillMetadata` +
//! [`CapabilityProfile`]s, so [`CapabilityGraph::rebuild`] is idempotent from a
//! fixed registry state (R5.1).
//!
//! # Edge kinds (design §7.4 — `edge_kind` column vocabulary)
//!
//! | kind          | meaning                                                        |
//! |---------------|----------------------------------------------------------------|
//! | `depends`     | `from` declares `to` as a `SkillMetadata` dependency.          |
//! | `provides_for`| `from`'s `provides` tags intersect `to`'s `consumes` tags.     |
//! | `alternative` | `from` and `to` share ≥1 `provides` tag (interchangeable).    |
//! | `supersedes`  | `from` shares identity (`name`) with `to` and has a higher     |
//! |               | `semantic_version` (a newer skill replaces an older one).      |
//!
//! # No-hardcoding (design §7.1 anti-hardcoding proof)
//!
//! Edge derivation is **purely generic** — set operations over open-vocabulary
//! [`CapabilityTag::id`] strings plus `SkillMetadata` fields (`dependencies`,
//! `name`, `semantic_version`). There is NO per-category branch and NO
//! skill-name special-casing anywhere: a never-before-seen capability domain
//! flows through with zero code change.
//!
//! # Determinism (R5.1 idempotent reindex)
//!
//! Edges are gathered into a [`BTreeSet`] keyed by
//! `(from_skill, edge_kind, to_skill)` before persistence, giving stable
//! lexicographic ordering with no duplicates. Rebuilding from identical input
//! yields byte-identical edges, and [`rebuild`](CapabilityGraph::rebuild) clears
//! the table before re-inserting so a rebuild never accumulates stale rows.
//!
//! [`ProductionSkillRegistry`]: crate::openclaw::registry::ProductionSkillRegistry
//! [`ProfileStore`]: crate::openclaw::cil::extract::ProfileStore
//! [`GrantStore`]: crate::openclaw::perm::grant_store::GrantStore
//! [`CapabilityTag::id`]: crate::openclaw::cil::profile::CapabilityTag::id

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection};

use super::profile::CapabilityProfile;
use super::CilError;
use crate::openclaw::registry::SkillMetadata;

/// The kind of a derived capability edge (design §7.4 `edge_kind` column).
///
/// Follows the [`ScopeKind`](crate::openclaw::perm::grant_store::ScopeKind)
/// pattern: a stable lower-case string on disk with an explicit
/// [`from_str`](EdgeKind::from_str) that surfaces an unknown value as a
/// [`CilError`] rather than silently defaulting (honesty invariant).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum EdgeKind {
    /// `from` declares `to` as a `SkillMetadata` dependency.
    Depends,
    /// `from`'s `provides` tags intersect `to`'s `consumes` tags (composition).
    ProvidesFor,
    /// `from` and `to` provide an overlapping capability set (interchangeable).
    Alternative,
    /// `from` shares identity with `to` and has a higher `semantic_version`.
    Supersedes,
}

impl EdgeKind {
    /// Stable lower-case string used in the `edge_kind` column.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Depends => "depends",
            Self::ProvidesFor => "provides_for",
            Self::Alternative => "alternative",
            Self::Supersedes => "supersedes",
        }
    }

    /// Parse an `edge_kind` column value. Unknown values are surfaced as a
    /// persistence error rather than silently defaulted (honesty invariant).
    pub fn from_str(s: &str) -> Result<Self, CilError> {
        match s {
            "depends" => Ok(Self::Depends),
            "provides_for" => Ok(Self::ProvidesFor),
            "alternative" => Ok(Self::Alternative),
            "supersedes" => Ok(Self::Supersedes),
            other => Err(CilError::Io(format!("unknown edge_kind {other:?}"))),
        }
    }
}

/// One row of the `capability_edges` derived view.
///
/// `weight` is the edge weight (defaults to `1.0`); today every derived edge is
/// unit-weighted, but the column and field exist so a later ranking phase can
/// weight edges without a schema change.
#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityEdge {
    /// The source skill id.
    pub from_skill: String,
    /// The destination skill id.
    pub to_skill: String,
    /// The kind of relationship this edge represents.
    pub edge_kind: EdgeKind,
    /// Edge weight (defaults to `1.0`).
    pub weight: f64,
}

impl CapabilityEdge {
    /// Construct a unit-weighted edge.
    fn unit(from_skill: String, to_skill: String, edge_kind: EdgeKind) -> Self {
        Self {
            from_skill,
            to_skill,
            edge_kind,
            weight: 1.0,
        }
    }
}

/// Version-/deprecation-awareness verdict for a single installed skill
/// (task 12.2, design §13 "Version + deprecation awareness", R8.4 / R9.4).
///
/// This is a **derived, honest read** that fuses two independent sources of
/// truth — the capability graph's `supersedes` edges and the `market_catalog`
/// `version`/`deprecated` columns — into one shape the Recommender (task 7) and
/// the Desktop surface (task 13.2) can consume without re-implementing the join:
///
/// - `newer_version`: the highest `market_catalog.version` for the skill's slug
///   that is a strictly-greater semver than the installed version, or `None`
///   when the catalog has no such row (offline / empty catalog / already latest).
///   Because it is `None` when no catalog row exists, an empty catalog reports
///   "no update" truthfully rather than fabricating one.
/// - `deprecated`: `true` when the installed skill is deprecated — either the
///   `market_catalog` row matching the installed version carries `deprecated = 1`,
///   OR a newer skill supersedes it (a superseded skill is, by definition,
///   deprecated). `false` otherwise.
/// - `superseded_by`: skill ids of newer skills that supersede this one
///   (`from --supersedes--> skill_id` edges), sorted. Empty when none.
/// - `alternatives`: interchangeable skill ids (`alternative` edges), sorted.
///   Empty when none.
///
/// All four fields are driven purely from edges + catalog columns + semver
/// metadata — there is no skill-name or per-category branching (no-hardcoding),
/// and the result is deterministic for a fixed `capability_edges` +
/// `market_catalog` state.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct UpdateAvailability {
    /// Highest catalog semver strictly newer than the installed version, if any.
    pub newer_version: Option<String>,
    /// Whether the installed skill is deprecated (catalog flag OR superseded).
    pub deprecated: bool,
    /// Newer skills that supersede this one (`supersedes` edges into `skill_id`).
    pub superseded_by: Vec<String>,
    /// Interchangeable alternatives (`alternative` edges from `skill_id`).
    pub alternatives: Vec<String>,
}

impl UpdateAvailability {
    /// Whether any form of update/replacement is available — a newer catalog
    /// version, a superseding skill, or a deprecation flag. Convenience for the
    /// UI badge; `false` means "you are on the latest, non-deprecated skill".
    pub fn has_update(&self) -> bool {
        self.newer_version.is_some() || self.deprecated || !self.superseded_by.is_empty()
    }
}

/// Derive the full set of capability edges from a registry snapshot — a **pure,
/// deterministic** function (no I/O, no persistence).
///
/// This is the generic derivation core shared by [`CapabilityGraph::rebuild`] and
/// the idempotency property test (task 12.4). Edges are keyed by
/// `(from, edge_kind, to)` in a [`BTreeSet`], so the returned `Vec` is sorted and
/// duplicate-free; identical input yields byte-identical output.
///
/// Derivation rules (all generic — see module docs):
///
/// - **`depends`**: one edge `skill → dep.skill_id` per `SkillMetadata.dependencies`.
/// - **`provides_for`**: `A → B` when `A.provides ∩ B.consumes ≠ ∅` (tag-id
///   overlap), `A ≠ B`.
/// - **`alternative`**: `A → B` (and `B → A`) when `A.provides ∩ B.provides ≠ ∅`
///   (non-empty overlap of provided tag ids), `A ≠ B`. The relation is symmetric,
///   so both directions are emitted.
/// - **`supersedes`**: `A → B` when `A.name == B.name`, both `semantic_version`s
///   parse as semver, and `A`'s version is strictly greater than `B`'s (the newer
///   skill supersedes the older). Skills whose `semantic_version` fails to parse
///   are skipped for this edge kind; task 12.2 augments `supersedes` from
///   `market_catalog` version/deprecation data.
pub fn derive_edges(
    skills: &[SkillMetadata],
    profiles: &[CapabilityProfile],
) -> Vec<CapabilityEdge> {
    // Key set for dedup + stable (from, kind, to) ordering. EdgeKind is Ord and
    // sorts by declaration order (Depends < ProvidesFor < Alternative < Supersedes).
    let mut keys: BTreeSet<(String, EdgeKind, String)> = BTreeSet::new();

    // provides/consumes tag-id sets keyed by skill_id (open-vocabulary strings).
    let provides: BTreeMap<&str, BTreeSet<&str>> = profiles
        .iter()
        .map(|p| {
            (
                p.skill_id.as_str(),
                p.provides.iter().map(|t| t.id.as_str()).collect(),
            )
        })
        .collect();
    let consumes: BTreeMap<&str, BTreeSet<&str>> = profiles
        .iter()
        .map(|p| {
            (
                p.skill_id.as_str(),
                p.consumes.iter().map(|t| t.id.as_str()).collect(),
            )
        })
        .collect();

    // --- depends: from SkillMetadata.dependencies (generic, no name/category branch).
    for skill in skills {
        for dep in &skill.dependencies {
            let to = dep.skill_id.trim();
            if to.is_empty() || to == skill.skill_id {
                continue;
            }
            keys.insert((skill.skill_id.clone(), EdgeKind::Depends, to.to_string()));
        }
    }

    // --- provides_for + alternative: pairwise set overlap over profiles.
    for pa in profiles {
        let a_prov = provides.get(pa.skill_id.as_str());
        for pb in profiles {
            if pa.skill_id == pb.skill_id {
                continue;
            }
            // provides_for: A.provides ∩ B.consumes ≠ ∅.
            if let (Some(a_prov), Some(b_cons)) = (a_prov, consumes.get(pb.skill_id.as_str())) {
                if a_prov.intersection(b_cons).next().is_some() {
                    keys.insert((
                        pa.skill_id.clone(),
                        EdgeKind::ProvidesFor,
                        pb.skill_id.clone(),
                    ));
                }
            }
            // alternative: A.provides ∩ B.provides ≠ ∅ (symmetric; both dirs added
            // — the reverse direction is emitted when the loop reaches (pb, pa)).
            if let (Some(a_prov), Some(b_prov)) = (a_prov, provides.get(pb.skill_id.as_str())) {
                if a_prov.intersection(b_prov).next().is_some() {
                    keys.insert((
                        pa.skill_id.clone(),
                        EdgeKind::Alternative,
                        pb.skill_id.clone(),
                    ));
                }
            }
        }
    }

    // --- supersedes: same identity (name), higher semver → newer supersedes older.
    for a in skills {
        for b in skills {
            if a.skill_id == b.skill_id || a.name != b.name {
                continue;
            }
            if let (Ok(va), Ok(vb)) = (
                semver::Version::parse(a.semantic_version.trim()),
                semver::Version::parse(b.semantic_version.trim()),
            ) {
                if va > vb {
                    keys.insert((a.skill_id.clone(), EdgeKind::Supersedes, b.skill_id.clone()));
                }
            }
        }
    }

    keys.into_iter()
        .map(|(from, kind, to)| CapabilityEdge::unit(from, to, kind))
        .collect()
}

/// The `CapabilityGraph` store over the `capability_edges` derived view (design §7.4).
///
/// Holds an `Arc<Mutex<Connection>>` to `skills.db` — the SAME database as
/// [`ProductionSkillRegistry`] / [`ProfileStore`] / [`GrantStore`] (no second
/// database). The `capability_edges` table is created only by the frozen registry
/// migrations (migration 6); this store issues **no DDL** and never drops or
/// rewrites schema. It is a rebuildable materialized view: [`rebuild`] clears and
/// re-inserts atomically so the persisted edges always reflect the current
/// registry state.
///
/// [`ProductionSkillRegistry`]: crate::openclaw::registry::ProductionSkillRegistry
/// [`ProfileStore`]: crate::openclaw::cil::extract::ProfileStore
/// [`GrantStore`]: crate::openclaw::perm::grant_store::GrantStore
/// [`rebuild`]: CapabilityGraph::rebuild
pub struct CapabilityGraph {
    db: Arc<Mutex<Connection>>,
}

impl CapabilityGraph {
    /// Open an additional connection to `skills.db` (WAL), mirroring
    /// [`ProfileStore::open`](crate::openclaw::cil::extract::ProfileStore::open).
    ///
    /// The `capability_edges` table is created by the frozen registry migrations
    /// (migration 6); construct the registry first (or otherwise run migrations)
    /// so the table exists. This store never issues DDL.
    pub fn open(db_path: &Path) -> Result<Self, CilError> {
        let conn = Connection::open(db_path)
            .map_err(|e| CilError::Io(format!("open skills.db for capability graph: {e}")))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| CilError::Io(format!("enable WAL for capability graph: {e}")))?;
        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    /// Build a `CapabilityGraph` over an already-open, shared `skills.db`
    /// connection (preferred when the registry connection is available — keeps
    /// every writer on one connection and one source of truth).
    pub fn from_shared_connection(db: Arc<Mutex<Connection>>) -> Self {
        Self { db }
    }

    /// Re-derive all edges from the registry snapshot and persist them,
    /// atomically clearing the table first (R5.1 idempotent reindex).
    ///
    /// Edges are derived by [`derive_edges`] (pure/generic) then written inside a
    /// single transaction: `DELETE FROM capability_edges` followed by an
    /// `INSERT OR REPLACE` per edge. Because derivation is deterministic and the
    /// table is cleared first, rebuilding from identical `skills`/`profiles`
    /// yields identical persisted edges and never accumulates stale rows.
    pub fn rebuild(
        &self,
        skills: &[SkillMetadata],
        profiles: &[CapabilityProfile],
    ) -> Result<(), CilError> {
        let edges = derive_edges(skills, profiles);
        let mut db = self
            .db
            .lock()
            .map_err(|_| CilError::Io("capability graph connection poisoned".into()))?;
        let tx = db
            .transaction()
            .map_err(|e| CilError::Io(format!("begin capability_edges rebuild tx: {e}")))?;
        tx.execute("DELETE FROM capability_edges", [])
            .map_err(|e| CilError::Io(format!("clear capability_edges: {e}")))?;
        for edge in &edges {
            tx.execute(
                "INSERT OR REPLACE INTO capability_edges (from_skill, to_skill, edge_kind, weight)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    edge.from_skill,
                    edge.to_skill,
                    edge.edge_kind.as_str(),
                    edge.weight,
                ],
            )
            .map_err(|e| {
                CilError::Io(format!(
                    "insert edge {} -{}-> {}: {e}",
                    edge.from_skill,
                    edge.edge_kind.as_str(),
                    edge.to_skill
                ))
            })?;
        }
        tx.commit()
            .map_err(|e| CilError::Io(format!("commit capability_edges rebuild: {e}")))?;
        Ok(())
    }

    /// All edges originating at `skill_id`, ordered by `(edge_kind, to_skill)`.
    pub fn edges_from(&self, skill_id: &str) -> Result<Vec<CapabilityEdge>, CilError> {
        self.query_edges(
            "SELECT from_skill, to_skill, edge_kind, weight
             FROM capability_edges WHERE from_skill = ?1
             ORDER BY edge_kind, to_skill",
            params![skill_id],
        )
    }

    /// All edges of a given [`EdgeKind`], ordered by `(from_skill, to_skill)`.
    pub fn edges_of_kind(&self, kind: EdgeKind) -> Result<Vec<CapabilityEdge>, CilError> {
        self.query_edges(
            "SELECT from_skill, to_skill, edge_kind, weight
             FROM capability_edges WHERE edge_kind = ?1
             ORDER BY from_skill, to_skill",
            params![kind.as_str()],
        )
    }

    /// The `to_skill`s reachable from `skill_id` along edges of `kind`, sorted.
    pub fn neighbors(&self, skill_id: &str, kind: EdgeKind) -> Result<Vec<String>, CilError> {
        let db = self
            .db
            .lock()
            .map_err(|_| CilError::Io("capability graph connection poisoned".into()))?;
        let mut stmt = db
            .prepare(
                "SELECT to_skill FROM capability_edges
                 WHERE from_skill = ?1 AND edge_kind = ?2
                 ORDER BY to_skill",
            )
            .map_err(|e| CilError::Io(format!("prepare neighbors: {e}")))?;
        let rows = stmt
            .query_map(params![skill_id, kind.as_str()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| CilError::Io(format!("query neighbors: {e}")))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| CilError::Io(format!("read neighbor: {e}")))?);
        }
        Ok(out)
    }

    /// Convenience read: skills that are alternatives to `skill_id`.
    pub fn alternatives(&self, skill_id: &str) -> Result<Vec<String>, CilError> {
        self.neighbors(skill_id, EdgeKind::Alternative)
    }

    /// Convenience read: skills that `skill_id` depends on.
    pub fn dependencies_of(&self, skill_id: &str) -> Result<Vec<String>, CilError> {
        self.neighbors(skill_id, EdgeKind::Depends)
    }

    /// Older skills that `skill_id` supersedes (forward `supersedes` edges),
    /// sorted. In the derivation convention (see [`derive_edges`]) a
    /// `supersedes` edge points from the newer skill to the older one, so this
    /// returns the skills `skill_id` replaces.
    pub fn supersessions(&self, skill_id: &str) -> Result<Vec<String>, CilError> {
        self.neighbors(skill_id, EdgeKind::Supersedes)
    }

    /// Newer skills that supersede `skill_id` (reverse `supersedes` edges),
    /// sorted. This is the "a newer skill replaces this" direction: any skill
    /// `Y` with an edge `Y --supersedes--> skill_id`.
    pub fn superseded_by(&self, skill_id: &str) -> Result<Vec<String>, CilError> {
        let db = self
            .db
            .lock()
            .map_err(|_| CilError::Io("capability graph connection poisoned".into()))?;
        let mut stmt = db
            .prepare(
                "SELECT from_skill FROM capability_edges
                 WHERE to_skill = ?1 AND edge_kind = ?2
                 ORDER BY from_skill",
            )
            .map_err(|e| CilError::Io(format!("prepare superseded_by: {e}")))?;
        let rows = stmt
            .query_map(params![skill_id, EdgeKind::Supersedes.as_str()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| CilError::Io(format!("query superseded_by: {e}")))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| CilError::Io(format!("read superseded_by: {e}")))?);
        }
        Ok(out)
    }

    /// The marketplace's view of `slug`: the highest catalog `version` that is a
    /// strictly-greater semver than `installed_version`, together with whether
    /// the marketplace flags the slug `deprecated`.
    ///
    /// Reads the `market_catalog` table that lives in the SAME `skills.db`
    /// (migration 4) — no second database. The table's PK is
    /// `(provider_id, slug)`, so a slug has at most one row **per provider**,
    /// each holding that provider's currently-offered version and deprecation
    /// flag. This method federates across providers:
    ///
    /// - `newer_version`: the highest offered `version` (across all providers)
    ///   that parses as semver and is strictly greater than `installed_version`.
    ///   `None` when no provider offers a newer version. Rows whose `version`
    ///   fails to parse are skipped (honest: an unparseable version cannot be
    ///   proven newer).
    /// - deprecated flag: `true` when ANY provider's row for the slug is flagged
    ///   `deprecated` — the marketplace has declared the skill deprecated,
    ///   independent of version.
    ///
    /// Returns `(None, false)` when the catalog has no row for `slug`, so an
    /// empty/offline catalog reports "no update" truthfully rather than
    /// fabricating one.
    fn market_version_info(
        &self,
        slug: &str,
        installed_version: &str,
    ) -> Result<(Option<String>, bool), CilError> {
        let installed = semver::Version::parse(installed_version.trim());
        let db = self
            .db
            .lock()
            .map_err(|_| CilError::Io("capability graph connection poisoned".into()))?;
        let mut stmt = db
            .prepare("SELECT version, deprecated FROM market_catalog WHERE slug = ?1")
            .map_err(|e| CilError::Io(format!("prepare market_catalog lookup: {e}")))?;
        let rows = stmt
            .query_map(params![slug], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| CilError::Io(format!("query market_catalog: {e}")))?;

        let mut best_newer: Option<semver::Version> = None;
        let mut catalog_deprecated = false;
        for r in rows {
            let (version_str, deprecated) =
                r.map_err(|e| CilError::Io(format!("read market_catalog row: {e}")))?;
            // Marketplace declares the skill deprecated (any provider), any version.
            if deprecated != 0 {
                catalog_deprecated = true;
            }
            // Newer-version comparison only when both versions parse as semver.
            if let (Ok(installed), Ok(candidate)) = (
                installed.as_ref(),
                semver::Version::parse(version_str.trim()).as_ref(),
            ) {
                if candidate > installed && best_newer.as_ref().map_or(true, |b| candidate > b) {
                    best_newer = Some(candidate.clone());
                }
            }
        }
        Ok((best_newer.map(|v| v.to_string()), catalog_deprecated))
    }

    /// Fuse the capability graph's `supersedes`/`alternative` edges with the
    /// `market_catalog` `version`/`deprecated` columns into a single
    /// [`UpdateAvailability`] verdict for an installed skill (R8.4, R9.4).
    ///
    /// - `skill_id`: the installed skill's registry id (used for edge lookups).
    /// - `slug`: the `market_catalog` slug for this skill (equals `skill_id` in
    ///   the current registry convention; kept separate so a future federated
    ///   slug scheme needs no signature change).
    /// - `installed_version`: the installed semver, used to decide "newer".
    ///
    /// The verdict is honest: with no catalog row and no `supersedes` edge it
    /// reports `newer_version = None`, `deprecated = false`, and empty
    /// `superseded_by` — never a fabricated update. `deprecated` is `true` when
    /// the installed catalog row is flagged OR the skill is superseded.
    pub fn update_availability(
        &self,
        skill_id: &str,
        slug: &str,
        installed_version: &str,
    ) -> Result<UpdateAvailability, CilError> {
        let superseded_by = self.superseded_by(skill_id)?;
        let alternatives = self.alternatives(skill_id)?;
        let (newer_version, catalog_deprecated) =
            self.market_version_info(slug, installed_version)?;
        Ok(UpdateAvailability {
            newer_version,
            deprecated: catalog_deprecated || !superseded_by.is_empty(),
            superseded_by,
            alternatives,
        })
    }

    /// Shared row-mapping helper for the edge queries above.
    fn query_edges(
        &self,
        sql: &str,
        params: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<CapabilityEdge>, CilError> {
        let db = self
            .db
            .lock()
            .map_err(|_| CilError::Io("capability graph connection poisoned".into()))?;
        let mut stmt = db
            .prepare(sql)
            .map_err(|e| CilError::Io(format!("prepare edge query: {e}")))?;
        let rows = stmt
            .query_map(params, |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, f64>(3)?,
                ))
            })
            .map_err(|e| CilError::Io(format!("query edges: {e}")))?;
        let mut out = Vec::new();
        for r in rows {
            let (from_skill, to_skill, kind_str, weight) =
                r.map_err(|e| CilError::Io(format!("read edge row: {e}")))?;
            out.push(CapabilityEdge {
                from_skill,
                to_skill,
                edge_kind: EdgeKind::from_str(&kind_str)?,
                weight,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openclaw::cil::profile::{CapabilityProfile, CapabilityTag};
    use crate::openclaw::registry::{
        DiscoverySource, ProductionSkillRegistry, SkillDependency, SkillMetadata, SkillState,
    };
    use crate::openclaw::types::{ResourceClass, SkillCapabilities, TrustTier};
    use crate::safety::RiskLevel;

    fn sample_meta(skill_id: &str) -> SkillMetadata {
        SkillMetadata {
            skill_id: skill_id.to_string(),
            name: format!("Skill {skill_id}"),
            description: "smoke-test skill".to_string(),
            publisher: "test".to_string(),
            version: "1.0.0".to_string(),
            category: "media".to_string(),
            discovery_source: DiscoverySource::Bundled {
                path: "test".to_string(),
            },
            discovered_at: chrono::Utc::now(),
            capabilities: SkillCapabilities::default(),
            runtime_requirements: "docker".to_string(),
            risk_level: RiskLevel::Green,
            resource_class: ResourceClass::Light,
            tags: vec!["test".to_string()],
            categories: vec![],
            semantic_version: "1.0.0".to_string(),
            dependencies: vec![],
            compatibility_requirements: vec![],
            trust_tier: TrustTier::Local,
            content_hash: format!("hash_{skill_id}"),
            signature: None,
            granted_capabilities: Vec::new(),
            bundle_path: None,
            manifest_toml: None,
            input_schema: None,
            state: SkillState::Discovered,
            state_changed_at: chrono::Utc::now(),
        }
    }

    fn profile(skill_id: &str, provides: &[&str], consumes: &[&str]) -> CapabilityProfile {
        CapabilityProfile {
            skill_id: skill_id.to_string(),
            provides: provides.iter().map(|s| CapabilityTag::new(*s)).collect(),
            consumes: consumes.iter().map(|s| CapabilityTag::new(*s)).collect(),
            permissions: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
        }
    }

    #[test]
    fn edge_kind_str_roundtrip_and_unknown_errors() {
        for kind in [
            EdgeKind::Depends,
            EdgeKind::ProvidesFor,
            EdgeKind::Alternative,
            EdgeKind::Supersedes,
        ] {
            assert_eq!(EdgeKind::from_str(kind.as_str()).unwrap(), kind);
        }
        assert!(EdgeKind::from_str("bogus").is_err());
    }

    #[test]
    fn derive_edges_covers_all_four_kinds_generically() {
        // A depends on B; A provides "media" consumed by C; A and D both provide
        // "media" (alternatives); A2 is a newer version of A1 by name+semver.
        let mut a = sample_meta("acme.a");
        a.dependencies = vec![SkillDependency {
            skill_id: "acme.b".into(),
            version_requirement: "*".into(),
            optional: false,
        }];
        let b = sample_meta("acme.b");
        let c = sample_meta("acme.c");
        let d = sample_meta("acme.d");

        // supersedes: same name, higher semver.
        let mut old = sample_meta("acme.tool.v1");
        old.name = "Shared Tool".into();
        old.semantic_version = "1.0.0".into();
        let mut new = sample_meta("acme.tool.v2");
        new.name = "Shared Tool".into();
        new.semantic_version = "2.0.0".into();

        let skills = vec![a, b, c, d, old, new];
        let profiles = vec![
            profile("acme.a", &["media"], &[]),
            profile("acme.b", &[], &[]),
            profile("acme.c", &[], &["media"]),
            profile("acme.d", &["media"], &[]),
        ];

        let edges = derive_edges(&skills, &profiles);
        let has = |from: &str, kind: EdgeKind, to: &str| {
            edges
                .iter()
                .any(|e| e.from_skill == from && e.edge_kind == kind && e.to_skill == to)
        };

        assert!(has("acme.a", EdgeKind::Depends, "acme.b"), "depends edge");
        assert!(
            has("acme.a", EdgeKind::ProvidesFor, "acme.c"),
            "provides_for edge (media → consumer)"
        );
        // alternative is symmetric.
        assert!(has("acme.a", EdgeKind::Alternative, "acme.d"));
        assert!(has("acme.d", EdgeKind::Alternative, "acme.a"));
        // newer supersedes older, never the reverse.
        assert!(has("acme.tool.v2", EdgeKind::Supersedes, "acme.tool.v1"));
        assert!(!has("acme.tool.v1", EdgeKind::Supersedes, "acme.tool.v2"));
    }

    #[test]
    fn derive_edges_is_deterministic_and_sorted() {
        let mut a = sample_meta("acme.a");
        a.dependencies = vec![SkillDependency {
            skill_id: "acme.b".into(),
            version_requirement: "*".into(),
            optional: false,
        }];
        let b = sample_meta("acme.b");
        let skills = vec![a, b];
        let profiles = vec![
            profile("acme.a", &["x"], &["y"]),
            profile("acme.b", &["y"], &["x"]),
        ];

        let e1 = derive_edges(&skills, &profiles);
        let e2 = derive_edges(&skills, &profiles);
        assert_eq!(e1, e2, "identical input → identical edges");

        // sorted by (from, kind, to).
        let keys: Vec<(String, EdgeKind, String)> = e1
            .iter()
            .map(|e| (e.from_skill.clone(), e.edge_kind, e.to_skill.clone()))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "edges must be stably ordered");
    }

    #[test]
    fn rebuild_persists_and_is_idempotent_over_skills_db() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("skills.db");
        // Frozen migrations create capability_edges (migration 6).
        let _registry = ProductionSkillRegistry::new(&db_path).expect("registry init");
        let graph = CapabilityGraph::open(&db_path).expect("graph open");

        let mut a = sample_meta("acme.a");
        a.dependencies = vec![SkillDependency {
            skill_id: "acme.b".into(),
            version_requirement: "*".into(),
            optional: false,
        }];
        let b = sample_meta("acme.b");
        let skills = vec![a, b];
        let profiles = vec![
            profile("acme.a", &["media"], &[]),
            profile("acme.b", &[], &["media"]),
        ];

        graph.rebuild(&skills, &profiles).expect("first rebuild");
        let deps = graph.dependencies_of("acme.a").expect("deps");
        assert_eq!(deps, vec!["acme.b".to_string()]);
        let pf = graph
            .edges_of_kind(EdgeKind::ProvidesFor)
            .expect("provides_for");
        assert_eq!(pf.len(), 1);
        assert_eq!(pf[0].from_skill, "acme.a");
        assert_eq!(pf[0].to_skill, "acme.b");

        // Rebuild from identical input → identical persisted edges (idempotent).
        let before = graph.edges_from("acme.a").expect("before");
        graph.rebuild(&skills, &profiles).expect("second rebuild");
        let after = graph.edges_from("acme.a").expect("after");
        assert_eq!(before, after, "rebuild is idempotent from fixed input");
    }

    /// Insert a market_catalog row directly (task 6.2 owns real population; this
    /// helper lets 12.2 smoke-test the version/deprecation read in isolation).
    fn insert_market_row(
        db: &Arc<Mutex<Connection>>,
        provider: &str,
        slug: &str,
        version: &str,
        deprecated: bool,
    ) {
        let conn = db.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO market_catalog
             (provider_id, slug, manifest_json, version, deprecated, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                provider,
                slug,
                "{}",
                version,
                if deprecated { 1_i64 } else { 0_i64 },
                "2024-01-01T00:00:00Z"
            ],
        )
        .expect("insert market_catalog row");
    }

    #[test]
    fn superseded_skill_reports_newer_and_is_deprecated() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("skills.db");
        let _registry = ProductionSkillRegistry::new(&db_path).expect("registry init");
        let graph = CapabilityGraph::open(&db_path).expect("graph open");

        // Two versions of the same-named skill → supersedes edge (v2 → v1).
        let mut old = sample_meta("acme.tool.v1");
        old.name = "Shared Tool".into();
        old.semantic_version = "1.0.0".into();
        let mut new = sample_meta("acme.tool.v2");
        new.name = "Shared Tool".into();
        new.semantic_version = "2.0.0".into();
        let skills = vec![old, new];
        let profiles = vec![
            profile("acme.tool.v1", &[], &[]),
            profile("acme.tool.v2", &[], &[]),
        ];
        graph.rebuild(&skills, &profiles).expect("rebuild");

        // Directions.
        assert_eq!(
            graph.superseded_by("acme.tool.v1").expect("superseded_by"),
            vec!["acme.tool.v2".to_string()]
        );
        assert_eq!(
            graph.supersessions("acme.tool.v2").expect("supersessions"),
            vec!["acme.tool.v1".to_string()]
        );

        // The old skill is superseded → deprecated=true even with no catalog row.
        let ua = graph
            .update_availability("acme.tool.v1", "acme.tool.v1", "1.0.0")
            .expect("update_availability");
        assert!(ua.deprecated, "superseded skill is deprecated");
        assert_eq!(ua.superseded_by, vec!["acme.tool.v2".to_string()]);
        assert!(ua.has_update());
    }

    #[test]
    fn market_catalog_drives_newer_version_and_deprecation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("skills.db");
        let _registry = ProductionSkillRegistry::new(&db_path).expect("registry init");
        let graph = CapabilityGraph::open(&db_path).expect("graph open");

        let skills = vec![sample_meta("acme.solo")];
        let profiles = vec![profile("acme.solo", &[], &[])];
        graph.rebuild(&skills, &profiles).expect("rebuild");

        // Two federated providers offer the slug (PK is (provider_id, slug), so
        // one row per provider): clawhub offers a newer 2.0.0 and flags it
        // deprecated; a mirror still offers the installed 1.0.0.
        insert_market_row(&graph.db, "mirror", "acme.solo", "1.0.0", false);
        insert_market_row(&graph.db, "clawhub", "acme.solo", "2.0.0", true);

        let ua = graph
            .update_availability("acme.solo", "acme.solo", "1.0.0")
            .expect("update_availability");
        assert_eq!(
            ua.newer_version,
            Some("2.0.0".to_string()),
            "newest version across providers wins"
        );
        assert!(ua.deprecated, "a provider flags the slug deprecated");
        assert!(ua.superseded_by.is_empty(), "no supersedes edge");
        assert!(ua.has_update());
    }

    #[test]
    fn no_edge_no_catalog_reports_no_update_honestly() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("skills.db");
        let _registry = ProductionSkillRegistry::new(&db_path).expect("registry init");
        let graph = CapabilityGraph::open(&db_path).expect("graph open");

        let skills = vec![sample_meta("acme.solo")];
        let profiles = vec![profile("acme.solo", &[], &[])];
        graph.rebuild(&skills, &profiles).expect("rebuild");

        // Empty market_catalog + no supersedes edge → honest "no update".
        let ua = graph
            .update_availability("acme.solo", "acme.solo", "1.0.0")
            .expect("update_availability");
        assert_eq!(ua, UpdateAvailability::default());
        assert!(!ua.has_update());
    }

    #[test]
    fn alternatives_query_reads_symmetric_edges() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("skills.db");
        let _registry = ProductionSkillRegistry::new(&db_path).expect("registry init");
        let graph = CapabilityGraph::open(&db_path).expect("graph open");

        let skills = vec![sample_meta("acme.a"), sample_meta("acme.d")];
        let profiles = vec![
            profile("acme.a", &["media"], &[]),
            profile("acme.d", &["media"], &[]),
        ];
        graph.rebuild(&skills, &profiles).expect("rebuild");

        assert_eq!(
            graph.alternatives("acme.a").expect("alts"),
            vec!["acme.d".to_string()]
        );
        assert_eq!(
            graph.alternatives("acme.d").expect("alts"),
            vec!["acme.a".to_string()]
        );
    }

    /// R8.4 (alternatives): a skill in a set of ≥3 that share a provided tag
    /// surfaces ALL its alternatives, sorted; the relation is symmetric across
    /// the whole set; and a skill sharing no provided tag reports empty. This
    /// extends `alternatives_query_reads_symmetric_edges` (2-skill) to the
    /// multi-alternative + no-alternative cases.
    #[test]
    fn alternatives_three_skill_set_surfaces_all_sorted_and_empty_when_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("skills.db");
        let _registry = ProductionSkillRegistry::new(&db_path).expect("registry init");
        let graph = CapabilityGraph::open(&db_path).expect("graph open");

        // Three skills all provide "media" → each is an alternative of the other
        // two. A fourth provides an unrelated tag → it has no alternatives.
        let skills = vec![
            sample_meta("acme.a"),
            sample_meta("acme.b"),
            sample_meta("acme.c"),
            sample_meta("acme.lonely"),
        ];
        let profiles = vec![
            profile("acme.a", &["media"], &[]),
            profile("acme.b", &["media"], &[]),
            profile("acme.c", &["media"], &[]),
            profile("acme.lonely", &["solo"], &[]),
        ];
        graph.rebuild(&skills, &profiles).expect("rebuild");

        // ALL alternatives surfaced for each member, sorted lexicographically.
        assert_eq!(
            graph.alternatives("acme.a").expect("alts a"),
            vec!["acme.b".to_string(), "acme.c".to_string()]
        );
        assert_eq!(
            graph.alternatives("acme.b").expect("alts b"),
            vec!["acme.a".to_string(), "acme.c".to_string()]
        );
        assert_eq!(
            graph.alternatives("acme.c").expect("alts c"),
            vec!["acme.a".to_string(), "acme.b".to_string()]
        );

        // Symmetric across the whole 3-skill set (both directions present).
        for (x, y) in [
            ("acme.a", "acme.b"),
            ("acme.a", "acme.c"),
            ("acme.b", "acme.c"),
        ] {
            assert!(
                graph.alternatives(x).unwrap().contains(&y.to_string()),
                "{x} lists {y}"
            );
            assert!(
                graph.alternatives(y).unwrap().contains(&x.to_string()),
                "{y} lists {x}"
            );
        }

        // No shared provided tag → no alternatives (honest empty, not fabricated).
        assert!(
            graph
                .alternatives("acme.lonely")
                .expect("alts lonely")
                .is_empty(),
            "a skill sharing no provided tag has no alternatives"
        );
    }

    /// R8.4 (successors): a 3-version chain (v1 < v2 < v3, same name) surfaces
    /// the full transitive-by-semver picture — `superseded_by(v1)` lists BOTH
    /// newer versions, `supersessions(v3)` lists BOTH older versions, and the
    /// oldest's `update_availability` reports it superseded + deprecated. This
    /// extends `superseded_skill_reports_newer_and_is_deprecated` (2 versions)
    /// to the multi-successor case.
    #[test]
    fn supersedes_three_version_chain_reports_all_newer_and_older() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("skills.db");
        let _registry = ProductionSkillRegistry::new(&db_path).expect("registry init");
        let graph = CapabilityGraph::open(&db_path).expect("graph open");

        let mut v1 = sample_meta("acme.chain.v1");
        v1.name = "Chain Tool".into();
        v1.semantic_version = "1.0.0".into();
        let mut v2 = sample_meta("acme.chain.v2");
        v2.name = "Chain Tool".into();
        v2.semantic_version = "2.0.0".into();
        let mut v3 = sample_meta("acme.chain.v3");
        v3.name = "Chain Tool".into();
        v3.semantic_version = "3.0.0".into();
        let skills = vec![v1, v2, v3];
        let profiles = vec![
            profile("acme.chain.v1", &[], &[]),
            profile("acme.chain.v2", &[], &[]),
            profile("acme.chain.v3", &[], &[]),
        ];
        graph.rebuild(&skills, &profiles).expect("rebuild");

        // v1 is superseded by BOTH newer versions, sorted.
        assert_eq!(
            graph
                .superseded_by("acme.chain.v1")
                .expect("superseded_by v1"),
            vec!["acme.chain.v2".to_string(), "acme.chain.v3".to_string()]
        );
        // v3 supersedes BOTH older versions, sorted.
        assert_eq!(
            graph
                .supersessions("acme.chain.v3")
                .expect("supersessions v3"),
            vec!["acme.chain.v1".to_string(), "acme.chain.v2".to_string()]
        );
        // The newest is superseded by nothing.
        assert!(
            graph
                .superseded_by("acme.chain.v3")
                .expect("superseded_by v3")
                .is_empty(),
            "the latest version is not superseded"
        );

        // R9.4: the oldest version reports superseded + deprecated, no catalog row.
        let ua = graph
            .update_availability("acme.chain.v1", "acme.chain.v1", "1.0.0")
            .expect("update_availability v1");
        assert!(ua.deprecated, "a superseded skill is deprecated");
        assert_eq!(
            ua.superseded_by,
            vec!["acme.chain.v2".to_string(), "acme.chain.v3".to_string()]
        );
        assert!(ua.has_update());
    }

    /// R9.4 (version awareness): when the marketplace only offers the installed
    /// version (or older) and flags nothing deprecated, `update_availability`
    /// reports NO update even though catalog rows exist — the presence of rows
    /// must not fabricate an update. Complements `no_edge_no_catalog_...` (which
    /// covers the empty-catalog case).
    #[test]
    fn latest_installed_with_catalog_reports_no_update() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("skills.db");
        let _registry = ProductionSkillRegistry::new(&db_path).expect("registry init");
        let graph = CapabilityGraph::open(&db_path).expect("graph open");

        let skills = vec![sample_meta("acme.solo")];
        let profiles = vec![profile("acme.solo", &[], &[])];
        graph.rebuild(&skills, &profiles).expect("rebuild");

        // Two providers offer only the installed version or older, none deprecated.
        insert_market_row(&graph.db, "clawhub", "acme.solo", "1.0.0", false);
        insert_market_row(&graph.db, "mirror", "acme.solo", "0.9.0", false);

        let ua = graph
            .update_availability("acme.solo", "acme.solo", "1.0.0")
            .expect("update_availability");
        assert_eq!(
            ua,
            UpdateAvailability::default(),
            "on the latest, non-deprecated version → no update despite catalog rows"
        );
        assert!(
            ua.newer_version.is_none(),
            "no strictly-newer version offered"
        );
        assert!(!ua.has_update());
    }

    /// R9.4 (deprecation respected): a `market_catalog` row flagged
    /// `deprecated = 1` makes `update_availability.deprecated` true even when
    /// there is NO newer version and NO supersedes edge — deprecation is an
    /// independent signal. Isolates the deprecation flag from the version bump
    /// tested in `market_catalog_drives_newer_version_and_deprecation`.
    #[test]
    fn deprecation_flag_respected_without_newer_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("skills.db");
        let _registry = ProductionSkillRegistry::new(&db_path).expect("registry init");
        let graph = CapabilityGraph::open(&db_path).expect("graph open");

        let skills = vec![sample_meta("acme.solo")];
        let profiles = vec![profile("acme.solo", &[], &[])];
        graph.rebuild(&skills, &profiles).expect("rebuild");

        // Only the installed version is offered, but the provider flags it deprecated.
        insert_market_row(&graph.db, "clawhub", "acme.solo", "1.0.0", true);

        let ua = graph
            .update_availability("acme.solo", "acme.solo", "1.0.0")
            .expect("update_availability");
        assert!(ua.deprecated, "catalog deprecation flag is respected");
        assert!(
            ua.newer_version.is_none(),
            "deprecated but no strictly-newer version offered"
        );
        assert!(ua.superseded_by.is_empty(), "no supersedes edge");
        assert!(
            ua.has_update(),
            "deprecation alone signals the user should act"
        );
    }

    // -------------------------------------------------------------------------
    // Property 1: Single source of truth — idempotent edge rebuild (task 12.4)
    //
    // **Validates: Requirements 5.1**
    //
    // Requirement 5.1: *"rebuilding all derived views from the registry yields
    // identical query results (idempotent reindex)."* The `capability_edges`
    // view is a rebuildable projection of `SkillMetadata` + `CapabilityProfile`s
    // (the registry being the sole source of truth). This property generates
    // arbitrary synthetic metadata/profile sets — with shared names + varying
    // semvers so `supersedes` edges arise, cross-referencing dependencies, and a
    // deliberately novel open-vocabulary `CapabilityTag` domain (no hardcoding)
    // — then builds the graph twice and asserts every graph query (edges by
    // kind, edges-from, alternatives, dependencies, supersedes both directions)
    // is byte-identical across rebuilds. It further asserts the persisted edges
    // equal the pure `derive_edges` derivation and survive a fresh reopen of the
    // store (restart durability), so the view is a well-defined function of
    // registry state alone.
    // -------------------------------------------------------------------------
    use proptest::prelude::*;

    /// Open-vocabulary capability id strings: common namespaced ids, a
    /// deliberately **novel** domain, and freely-generated reverse-DNS strings.
    /// Keeps the input space open (no closed enum) per Requirement 1.
    fn pbt_cap_string() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("media.image".to_string()),
            Just("doc.pdf".to_string()),
            Just("io.file.read".to_string()),
            Just("net.email.send".to_string()),
            // Never-before-seen domain: flows through as an open string, zero code.
            Just("quantum.entangle.route".to_string()),
            "[a-z]{1,8}(\\.[a-z]{1,8}){0,2}",
        ]
    }

    /// A small semver pool so same-named skills produce `supersedes` edges.
    fn pbt_version() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("1.0.0".to_string()),
            Just("1.2.0".to_string()),
            Just("2.0.0".to_string()),
            Just("3.1.4".to_string()),
        ]
    }

    /// A generated skill "shape". `skill_id` is assigned by index at build time
    /// (guaranteeing uniqueness) while every other field varies freely. A small
    /// `name_pool` forces name collisions so `supersedes` edges arise; `dep_mods`
    /// reference other generated skills by modular index to produce `depends`
    /// edges within the set.
    #[derive(Debug, Clone)]
    struct GraphSkillSpec {
        id_stub: String,
        name_pool: u8,
        version: String,
        provides: Vec<String>,
        consumes: Vec<String>,
        dep_mods: Vec<usize>,
    }

    fn graph_skill_spec() -> impl Strategy<Value = GraphSkillSpec> {
        (
            "[a-z]{1,6}",
            0u8..4u8,
            pbt_version(),
            prop::collection::vec(pbt_cap_string(), 0..4),
            prop::collection::vec(pbt_cap_string(), 0..4),
            prop::collection::vec(0usize..16usize, 0..3),
        )
            .prop_map(
                |(id_stub, name_pool, version, provides, consumes, dep_mods)| GraphSkillSpec {
                    id_stub,
                    name_pool,
                    version,
                    provides,
                    consumes,
                    dep_mods,
                },
            )
    }

    /// A deterministic, comparable snapshot of ALL graph query results, keyed and
    /// ordered by `skill_id`. This is the "query result" `R` the invariant
    /// compares (`R1 == R2 == R3`). It exercises every public read: edges by kind
    /// (all four), edges-from, alternatives, dependencies, and supersedes in both
    /// directions.
    type GraphSnapshot = (
        Vec<CapabilityEdge>,
        Vec<(
            String,
            Vec<CapabilityEdge>,
            Vec<String>,
            Vec<String>,
            Vec<String>,
            Vec<String>,
        )>,
    );

    fn graph_snapshot(graph: &CapabilityGraph, ids: &[String]) -> GraphSnapshot {
        let mut edges = Vec::new();
        for kind in [
            EdgeKind::Depends,
            EdgeKind::ProvidesFor,
            EdgeKind::Alternative,
            EdgeKind::Supersedes,
        ] {
            edges.extend(graph.edges_of_kind(kind).expect("edges_of_kind"));
        }

        let mut sorted = ids.to_vec();
        sorted.sort();
        sorted.dedup();
        let per_skill = sorted
            .into_iter()
            .map(|id| {
                let ef = graph.edges_from(&id).expect("edges_from");
                let alt = graph.alternatives(&id).expect("alternatives");
                let dep = graph.dependencies_of(&id).expect("dependencies_of");
                let sb = graph.superseded_by(&id).expect("superseded_by");
                let ss = graph.supersessions(&id).expect("supersessions");
                (id, ef, alt, dep, sb, ss)
            })
            .collect();

        (edges, per_skill)
    }

    proptest! {
        // Bounded case count keeps this DB-backed test fast and deterministic.
        #![proptest_config(ProptestConfig::with_cases(32))]

        /// Property 1: Single source of truth (Validates: Requirements 5.1).
        ///
        /// For an arbitrary synthetic metadata/profile population, rebuilding the
        /// `capability_edges` view is idempotent (`R1 == R2`), the persisted edges
        /// equal the pure `derive_edges` derivation, and the view survives a fresh
        /// reopen of the store (`R1 == R3`).
        #[test]
        fn idempotent_edge_rebuild_is_single_source_of_truth(
            specs in prop::collection::vec(graph_skill_spec(), 0..12)
        ) {
            let total = specs.len();
            // Unique skill_ids first, so dependencies can cross-reference by index.
            let ids: Vec<String> = specs
                .iter()
                .enumerate()
                .map(|(i, s)| format!("skill.{}.{i}", s.id_stub))
                .collect();

            // Build metadata (shared names + semvers → supersedes; deps → depends).
            let skills: Vec<SkillMetadata> = specs
                .iter()
                .enumerate()
                .map(|(i, spec)| {
                    let mut meta = sample_meta(&ids[i]);
                    meta.name = format!("shared.{}", spec.name_pool);
                    meta.version = spec.version.clone();
                    meta.semantic_version = spec.version.clone();
                    if total > 0 {
                        let mut seen = std::collections::BTreeSet::new();
                        for m in &spec.dep_mods {
                            let target = m % total;
                            if target == i || !seen.insert(target) {
                                continue;
                            }
                            meta.dependencies.push(SkillDependency {
                                skill_id: ids[target].clone(),
                                version_requirement: "*".into(),
                                optional: false,
                            });
                        }
                    }
                    meta
                })
                .collect();

            let profiles: Vec<CapabilityProfile> = specs
                .iter()
                .enumerate()
                .map(|(i, spec)| {
                    let prov: Vec<&str> = spec.provides.iter().map(String::as_str).collect();
                    let cons: Vec<&str> = spec.consumes.iter().map(String::as_str).collect();
                    profile(&ids[i], &prov, &cons)
                })
                .collect();

            let dir = tempfile::tempdir().expect("tempdir");
            let db_path = dir.path().join("skills.db");
            // Frozen migration 6 creates `capability_edges`; the registry is the
            // sole source of truth (we pass its projected view straight to rebuild).
            let _registry = ProductionSkillRegistry::new(&db_path).expect("registry init");
            let graph = CapabilityGraph::open(&db_path).expect("graph open");

            // Pass 1: build the view → R1.
            graph.rebuild(&skills, &profiles).expect("first rebuild");
            let r1 = graph_snapshot(&graph, &ids);

            // The persisted edge set must equal the pure derivation (single source
            // of truth: persistence introduces nothing the derivation did not).
            let mut persisted = r1.0.clone();
            persisted.sort_by(|a, b| {
                (a.from_skill.as_str(), a.edge_kind, a.to_skill.as_str())
                    .cmp(&(b.from_skill.as_str(), b.edge_kind, b.to_skill.as_str()))
            });
            let expected = derive_edges(&skills, &profiles);
            prop_assert_eq!(&persisted, &expected, "persisted edges differ from derive_edges");

            // Pass 2: rebuild from identical input (idempotent) → R2 == R1.
            graph.rebuild(&skills, &profiles).expect("second rebuild");
            let r2 = graph_snapshot(&graph, &ids);
            prop_assert_eq!(&r1, &r2, "idempotent rebuild changed query results (R1 != R2)");

            // Recovery/durability: a fresh store handle over the same skills.db
            // (simulating a restart) sees identical query results → R3 == R1.
            let graph2 = CapabilityGraph::open(&db_path).expect("graph reopen");
            let r3 = graph_snapshot(&graph2, &ids);
            prop_assert_eq!(&r1, &r3, "reopened store yielded different query results (R1 != R3)");
        }
    }
}
