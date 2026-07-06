//! `CapabilityProfile` extractor + persistence (task 2.3, design §7.1/§7.4).
//!
//! [`extract_profile`] derives a [`CapabilityProfile`] **generically** from a
//! [`SkillMetadata`] — a rebuildable VIEW, never an authoritative store. The
//! authoritative source of truth remains [`ProductionSkillRegistry`]; the
//! [`ProfileStore`] here persists derived profiles into the `capability_profiles`
//! table that task 2.1 appended to the frozen registry `MIGRATIONS` (migration 3),
//! inside the SAME `skills.db` — there is **no second database**.
//!
//! # No-hardcoding primitive (design §7.1 anti-hardcoding proof)
//!
//! Extraction treats every category/capability/type string **uniformly**. There
//! is deliberately NO `if skill_id == …` and NO `match category { "image" => … }`
//! branch anywhere: a category is copied verbatim into a [`CapabilityTag::id`], a
//! dependency's `skill_id` becomes a consumed tag, and JSON-schema `type`/`format`
//! scalars become I/O type strings — all open vocabulary. A never-before-seen
//! domain (`"quantum.entangle.route"`) flows through with zero code change.
//!
//! # Determinism (task 2.6 invariant)
//!
//! For a fixed [`SkillMetadata`], extraction is deterministic: all derived
//! collections are gathered into a [`BTreeSet`] (dedup + stable lexicographic
//! order) before being turned into tags/strings, and permissions come from the
//! frozen, order-preserving [`capability`] helpers. No randomness, no hashing of
//! addresses, no time. Same input → byte-identical output.
//!
//! [`ProductionSkillRegistry`]: crate::openclaw::registry::ProductionSkillRegistry
//! [`SkillMetadata`]: crate::openclaw::registry::SkillMetadata
//! [`capability`]: crate::openclaw::capability

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection};

use super::profile::{CapabilityProfile, CapabilityProfileRow, CapabilityTag, ProfileColumns};
use super::CilError;
use crate::openclaw::capability::{self, Capability};
use crate::openclaw::registry::SkillMetadata;

/// Derive a [`CapabilityProfile`] from a skill's [`SkillMetadata`] (design §7.1).
///
/// This is a **pure, deterministic** projection — a derived view over the
/// registry, never an authoritative store. It performs no I/O and no branching
/// on any specific category/capability string (see the module docs for the
/// no-hardcoding contract):
///
/// - **`provides`** — the skill's declared `category` (singular) plus every
///   `categories` entry, treated uniformly as open-vocabulary [`CapabilityTag`]
///   ids (deduped, sorted).
/// - **`consumes`** — every declared dependency's `skill_id`, as a composition
///   edge tag (deduped, sorted; empty when the skill has no dependencies).
/// - **`inputs`** — the `type`/`format` scalar hints found anywhere in the MCP
///   `input_schema` JSON, parsed generically (deduped, sorted).
/// - **`outputs`** — empty: `SkillMetadata` carries no output contract today.
///   Left honestly empty (not faked) rather than guessed; the field stays a
///   generic open-string list so a future output schema fills it with zero code.
/// - **`permissions`** — the frozen `Vec<Capability>`, preferring the
///   authoritative `granted_capabilities` (via [`capability::capabilities_of`])
///   and falling back to the legacy `SkillCapabilities` flags (via
///   [`capability::from_legacy`]). Risk/permission logic is NOT reinvented here.
pub fn extract_profile(meta: &SkillMetadata) -> CapabilityProfile {
    CapabilityProfile {
        skill_id: meta.skill_id.clone(),
        provides: tags_from_strings(provides_strings(meta)),
        consumes: tags_from_strings(consumes_strings(meta)),
        permissions: extract_permissions(meta),
        inputs: schema_type_hints(meta.input_schema.as_ref()),
        outputs: Vec::new(),
    }
}

/// Open-vocabulary strings a skill PROVIDES: its `category` + `categories`.
/// Every string is treated identically — no per-category branch.
fn provides_strings(meta: &SkillMetadata) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    insert_nonempty(&mut out, &meta.category);
    for c in &meta.categories {
        insert_nonempty(&mut out, c);
    }
    out
}

/// Open-vocabulary strings a skill CONSUMES: each declared dependency `skill_id`.
/// Empty when metadata declares no dependencies.
fn consumes_strings(meta: &SkillMetadata) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for dep in &meta.dependencies {
        insert_nonempty(&mut out, &dep.skill_id);
    }
    out
}

/// Map the frozen capability signal into `Vec<Capability>`, reusing frozen
/// helpers only. Prefer the authoritative `granted_capabilities` (the SOLE set
/// the runtime materializes); fall back to the lossy legacy flag projection when
/// no grant is present. Never reinvents risk/permission logic.
fn extract_permissions(meta: &SkillMetadata) -> Vec<Capability> {
    if meta.granted_capabilities.is_empty() {
        capability::from_legacy(&meta.capabilities)
    } else {
        capability::capabilities_of(&meta.granted_capabilities)
    }
}

/// Turn a deduped, sorted set of open strings into bare [`CapabilityTag`]s.
/// `BTreeSet` iteration order is lexicographic ⇒ deterministic output.
fn tags_from_strings(strings: BTreeSet<String>) -> Vec<CapabilityTag> {
    strings.into_iter().map(CapabilityTag::new).collect()
}

/// Insert a trimmed string if non-empty (skips whitespace-only noise uniformly).
fn insert_nonempty(set: &mut BTreeSet<String>, s: &str) {
    let t = s.trim();
    if !t.is_empty() {
        set.insert(t.to_string());
    }
}

/// Extract I/O type hints from an MCP `inputSchema` JSON value, generically.
///
/// Walks the whole JSON tree and collects the scalar value of every `"type"` and
/// `"format"` key (a `"type"` may also be an array of strings, e.g.
/// `["string","null"]`). This is uniform structural parsing — no key names or
/// schema shapes are special-cased. Returns a deduped, sorted `Vec<String>`.
fn schema_type_hints(schema: Option<&serde_json::Value>) -> Vec<String> {
    let mut hints = BTreeSet::new();
    if let Some(v) = schema {
        collect_type_hints(v, &mut hints);
    }
    hints.into_iter().collect()
}

/// Recursive generic walk collecting `type`/`format` scalars (see [`schema_type_hints`]).
fn collect_type_hints(value: &serde_json::Value, out: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map {
                if key == "type" || key == "format" {
                    match val {
                        serde_json::Value::String(s) => insert_nonempty(out, s),
                        serde_json::Value::Array(items) => {
                            for it in items {
                                if let serde_json::Value::String(s) = it {
                                    insert_nonempty(out, s);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                // Recurse into every value uniformly (properties, items, $defs,
                // allOf/anyOf/oneOf, nested objects — all handled the same way).
                collect_type_hints(val, out);
            }
        }
        serde_json::Value::Array(items) => {
            for it in items {
                collect_type_hints(it, out);
            }
        }
        _ => {}
    }
}

/// Persistence for the `capability_profiles` derived view (design §7.4).
///
/// Holds an `Arc<Mutex<Connection>>` to `skills.db` — the SAME database as
/// [`ProductionSkillRegistry`] and [`GrantStore`], following the registry's
/// connection pattern (no second database). The `capability_profiles` table is
/// created only by the frozen registry migrations (migration 3); this store
/// issues no DDL and never drops or rewrites schema.
///
/// Every write is an `INSERT OR REPLACE` keyed by `skill_id`: the table is a
/// **rebuildable view**, never authoritative. Re-deriving and re-persisting a
/// profile is idempotent.
///
/// [`ProductionSkillRegistry`]: crate::openclaw::registry::ProductionSkillRegistry
/// [`GrantStore`]: crate::openclaw::perm::grant_store::GrantStore
pub struct ProfileStore {
    db: Arc<Mutex<Connection>>,
}

impl ProfileStore {
    /// Open an additional connection to `skills.db`.
    ///
    /// Opens the SAME database file the registry uses (never a second database)
    /// and enables WAL for concurrent reads, matching `GrantStore::open` /
    /// `ProductionSkillRegistry::new`. The `capability_profiles` table is created
    /// by the frozen registry migrations (migration 3); construct the registry
    /// first (or otherwise run migrations) so the table exists.
    pub fn open(db_path: &Path) -> Result<Self, CilError> {
        let conn = Connection::open(db_path)
            .map_err(|e| CilError::Io(format!("open skills.db for profile store: {e}")))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| CilError::Io(format!("enable WAL for profile store: {e}")))?;
        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    /// Build a `ProfileStore` over an already-open, shared `skills.db` connection.
    ///
    /// Preferred when the registry's connection is available: it keeps every
    /// writer on one connection and one source of truth.
    pub fn from_shared_connection(db: Arc<Mutex<Connection>>) -> Self {
        Self { db }
    }

    /// Persist a derived profile (insert or replace by `skill_id`).
    ///
    /// Reuses [`CapabilityProfileRow::to_columns`] for the column encoding.
    /// `embedding` is the optional profile-level dense vector (computed by the
    /// embedder in a later phase — `None` at extraction time); `profile_epoch`
    /// versions the derived view. Idempotent: re-persisting the same `skill_id`
    /// overwrites the prior derived row.
    pub fn upsert_profile(
        &self,
        profile: &CapabilityProfile,
        embedding: Option<&[f32]>,
        profile_epoch: i64,
    ) -> Result<(), CilError> {
        let row = CapabilityProfileRow {
            profile: profile.clone(),
            embedding: embedding.map(|e| e.to_vec()),
            profile_epoch,
        };
        let cols = row.to_columns()?;
        let db = self
            .db
            .lock()
            .map_err(|_| CilError::Io("profile store connection poisoned".into()))?;
        db.execute(
            "INSERT OR REPLACE INTO capability_profiles (
                skill_id, provides_json, consumes_json, inputs_json,
                outputs_json, embedding, profile_epoch
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                cols.skill_id,
                cols.provides_json,
                cols.consumes_json,
                cols.inputs_json,
                cols.outputs_json,
                cols.embedding,
                cols.profile_epoch,
            ],
        )
        .map_err(|e| CilError::Io(format!("persist profile {}: {e}", cols.skill_id)))?;
        Ok(())
    }

    /// Derive a profile from metadata and persist it in one step (convenience for
    /// the backfill/reindex path, task 2.4). Equivalent to
    /// `upsert_profile(&extract_profile(meta), embedding, profile_epoch)`.
    pub fn derive_and_persist(
        &self,
        meta: &SkillMetadata,
        embedding: Option<&[f32]>,
        profile_epoch: i64,
    ) -> Result<(), CilError> {
        let profile = extract_profile(meta);
        self.upsert_profile(&profile, embedding, profile_epoch)
    }

    /// Delete the derived profile row for `skill_id` (idempotent).
    ///
    /// Used by the backfill registry subscriber (task 2.4) when a skill is
    /// removed/uninstalled: the derived view is a rebuildable projection of the
    /// registry, so dropping its row simply keeps the view current. A `DELETE`
    /// for an absent `skill_id` is a no-op (0 rows affected), never an error.
    pub fn delete_profile(&self, skill_id: &str) -> Result<(), CilError> {
        let db = self
            .db
            .lock()
            .map_err(|_| CilError::Io("profile store connection poisoned".into()))?;
        db.execute(
            "DELETE FROM capability_profiles WHERE skill_id = ?1",
            params![skill_id],
        )
        .map_err(|e| CilError::Io(format!("delete profile {skill_id}: {e}")))?;
        Ok(())
    }

    /// Fetch a persisted profile row by `skill_id`, or `None` if absent.
    ///
    /// Note: `permissions` are NOT a stored column (they are re-derived by
    /// [`extract_profile`]), so the returned [`CapabilityProfileRow::profile`]
    /// has an empty `permissions` vec — see [`CapabilityProfileRow::from_columns`].
    pub fn get_profile(&self, skill_id: &str) -> Result<Option<CapabilityProfileRow>, CilError> {
        let db = self
            .db
            .lock()
            .map_err(|_| CilError::Io("profile store connection poisoned".into()))?;
        let mut stmt = db
            .prepare(
                "SELECT skill_id, provides_json, consumes_json, inputs_json,
                        outputs_json, embedding, profile_epoch
                 FROM capability_profiles WHERE skill_id = ?1",
            )
            .map_err(|e| CilError::Io(format!("prepare profile get: {e}")))?;
        let mut rows = stmt
            .query(params![skill_id])
            .map_err(|e| CilError::Io(format!("query profile get: {e}")))?;
        match rows
            .next()
            .map_err(|e| CilError::Io(format!("read profile row: {e}")))?
        {
            Some(row) => {
                let cols = ProfileColumns {
                    skill_id: row
                        .get(0)
                        .map_err(|e| CilError::Io(format!("read skill_id: {e}")))?,
                    provides_json: row
                        .get(1)
                        .map_err(|e| CilError::Io(format!("read provides_json: {e}")))?,
                    consumes_json: row
                        .get(2)
                        .map_err(|e| CilError::Io(format!("read consumes_json: {e}")))?,
                    inputs_json: row
                        .get(3)
                        .map_err(|e| CilError::Io(format!("read inputs_json: {e}")))?,
                    outputs_json: row
                        .get(4)
                        .map_err(|e| CilError::Io(format!("read outputs_json: {e}")))?,
                    embedding: row
                        .get(5)
                        .map_err(|e| CilError::Io(format!("read embedding: {e}")))?,
                    profile_epoch: row
                        .get(6)
                        .map_err(|e| CilError::Io(format!("read profile_epoch: {e}")))?,
                };
                Ok(Some(CapabilityProfileRow::from_columns(&cols)?))
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openclaw::capability::{CapabilityKind, CapabilityMode};
    use crate::openclaw::registry::{
        DiscoverySource, ProductionSkillRegistry, SkillDependency, SkillMetadata, SkillState,
    };
    use crate::openclaw::types::{ResourceClass, SkillCapabilities, TrustTier};
    use crate::safety::RiskLevel;

    /// Minimal `SkillMetadata` builder for extractor smoke tests.
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
            categories: vec!["media.image".to_string(), "doc.pdf".to_string()],
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

    #[test]
    fn provides_from_category_and_categories_deduped_sorted() {
        let mut meta = sample_meta("acme.tool");
        // A duplicate of the singular category appears in categories → deduped.
        meta.categories = vec!["media".into(), "media.image".into(), "doc.pdf".into()];
        let profile = extract_profile(&meta);
        let ids: Vec<&str> = profile.provides.iter().map(|t| t.id.as_str()).collect();
        // Lexicographically sorted, deduped, includes the singular `category`.
        assert_eq!(ids, vec!["doc.pdf", "media", "media.image"]);
    }

    #[test]
    fn consumes_from_dependencies_open_strings() {
        let mut meta = sample_meta("acme.tool");
        meta.dependencies = vec![
            SkillDependency {
                skill_id: "z.dep".into(),
                version_requirement: "*".into(),
                optional: false,
            },
            SkillDependency {
                skill_id: "a.dep".into(),
                version_requirement: "*".into(),
                optional: true,
            },
        ];
        let profile = extract_profile(&meta);
        let ids: Vec<&str> = profile.consumes.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["a.dep", "z.dep"]);
        // No dependencies → empty consumes.
        meta.dependencies.clear();
        assert!(extract_profile(&meta).consumes.is_empty());
    }

    #[test]
    fn inputs_parsed_generically_from_input_schema() {
        let mut meta = sample_meta("acme.tool");
        meta.input_schema = Some(serde_json::json!({
            "type": "object",
            "properties": {
                "file": { "type": "string", "format": "binary" },
                "count": { "type": "integer" },
                "opt": { "type": ["string", "null"] }
            }
        }));
        let profile = extract_profile(&meta);
        // All `type`/`format` scalars, deduped + sorted; no schema special-casing.
        assert_eq!(
            profile.inputs,
            vec![
                "binary".to_string(),
                "integer".to_string(),
                "null".to_string(),
                "object".to_string(),
                "string".to_string(),
            ]
        );
        // No schema → no inputs.
        meta.input_schema = None;
        assert!(extract_profile(&meta).inputs.is_empty());
    }

    #[test]
    fn permissions_prefer_granted_then_fall_back_to_legacy_flags() {
        // Legacy fallback: no granted caps, network flag set → frozen from_legacy.
        let mut meta = sample_meta("acme.net");
        meta.capabilities.network = true;
        meta.capabilities.network_domains = vec!["api.example.com".into()];
        let legacy = extract_profile(&meta);
        assert!(legacy
            .permissions
            .iter()
            .any(|c| c.kind == CapabilityKind::Network));

        // Authoritative path: granted_capabilities present → used verbatim.
        let granted = capability::from_legacy(&{
            let mut caps = SkillCapabilities::default();
            caps.filesystem_write = true;
            caps
        });
        meta.granted_capabilities =
            capability::grant_all(&granted, capability::GrantSource::Manifest, true);
        let derived = extract_profile(&meta);
        assert!(derived
            .permissions
            .iter()
            .any(|c| c.kind == CapabilityKind::Filesystem && c.mode == CapabilityMode::ReadWrite));
    }

    #[test]
    fn extraction_is_deterministic_for_fixed_metadata() {
        let mut meta = sample_meta("acme.tool");
        meta.dependencies = vec![SkillDependency {
            skill_id: "a.dep".into(),
            version_requirement: "*".into(),
            optional: false,
        }];
        meta.input_schema = Some(serde_json::json!({ "type": "object" }));
        let a = extract_profile(&meta);
        let b = extract_profile(&meta);
        // Serialized form is byte-identical (stable ordering, no randomness).
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
    }

    /// A never-before-seen domain flows through as an open string — no code change.
    #[test]
    fn novel_capability_domain_needs_no_code_change() {
        let mut meta = sample_meta("acme.novel");
        meta.category = "quantum.entangle.route".into();
        meta.categories = vec![];
        let profile = extract_profile(&meta);
        assert_eq!(profile.provides.len(), 1);
        assert_eq!(profile.provides[0].id, "quantum.entangle.route");
    }

    #[test]
    fn upsert_then_get_roundtrips_over_skills_db() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("skills.db");
        // Frozen registry migrations create the capability_profiles table (migration 3).
        let registry = ProductionSkillRegistry::new(&db_path).expect("registry init");
        let store = ProfileStore::open(&db_path).expect("profile store open");

        let mut meta = sample_meta("acme.tool");
        meta.dependencies = vec![SkillDependency {
            skill_id: "io.file".into(),
            version_requirement: "*".into(),
            optional: false,
        }];
        // Profiles are a derived view over registered skills (FK to skills.skill_id).
        registry.install_skill(&meta).expect("register skill");
        store
            .derive_and_persist(&meta, None, 0)
            .expect("derive + persist");

        let row = store
            .get_profile("acme.tool")
            .expect("get")
            .expect("present");
        assert_eq!(row.profile.skill_id, "acme.tool");
        let expected = extract_profile(&meta);
        assert_eq!(row.profile.provides, expected.provides);
        assert_eq!(row.profile.consumes, expected.consumes);
        assert_eq!(row.profile.inputs, expected.inputs);
        assert_eq!(row.profile.outputs, expected.outputs);
        assert_eq!(row.profile_epoch, 0);
        assert!(row.embedding.is_none());
    }

    #[test]
    fn upsert_is_idempotent_replace_by_skill_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("skills.db");
        let registry = ProductionSkillRegistry::new(&db_path).expect("registry init");
        let store = ProfileStore::open(&db_path).expect("profile store open");

        let meta = sample_meta("acme.tool");
        registry.install_skill(&meta).expect("register skill");
        store.derive_and_persist(&meta, None, 1).expect("first");
        // Re-derive + re-persist: INSERT OR REPLACE keeps a single row, new epoch.
        store.derive_and_persist(&meta, None, 2).expect("second");
        let row = store
            .get_profile("acme.tool")
            .expect("get")
            .expect("present");
        assert_eq!(row.profile_epoch, 2);
    }

    // -----------------------------------------------------------------------
    // Task 2.6 — extractor determinism (extends the single-shape smoke test
    // `extraction_is_deterministic_for_fixed_metadata` above).
    //
    // `extract_profile` is a pure, deterministic projection (design §7.1 /
    // module docs). These tests assert byte-identical output across N repeated
    // runs for SEVERAL fixed `SkillMetadata` shapes (including dependencies,
    // multiple categories, and a nested `input_schema`), and that ordering of
    // `provides`/`consumes`/`inputs` is stable regardless of input insertion
    // order (R1.4: derived views are deterministic and rebuildable).
    // -----------------------------------------------------------------------

    /// A richer fixed skill exercising every derived field at once: a singular
    /// `category`, several `categories`, multiple `dependencies`, and a nested
    /// `input_schema` with arrays/objects.
    fn rich_meta(skill_id: &str) -> SkillMetadata {
        let mut meta = sample_meta(skill_id);
        meta.category = "media".into();
        meta.categories = vec![
            "media.image.ocr".into(),
            "doc.pdf.render".into(),
            "media".into(), // duplicate of singular category → deduped
        ];
        meta.dependencies = vec![
            SkillDependency {
                skill_id: "io.file.read".into(),
                version_requirement: "*".into(),
                optional: false,
            },
            SkillDependency {
                skill_id: "net.http.fetch".into(),
                version_requirement: "^1".into(),
                optional: true,
            },
        ];
        meta.input_schema = Some(serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "format": "uri" },
                "pages": { "type": "array", "items": { "type": "integer" } },
                "flags": { "type": ["boolean", "null"] }
            }
        }));
        meta
    }

    /// `extract_profile` produces byte-identical serialized output across many
    /// repeated runs, for several distinct fixed shapes.
    #[test]
    fn extract_profile_is_byte_identical_across_repeated_runs() {
        // A skill with no deps / no schema / single category.
        let mut minimal = sample_meta("acme.min");
        minimal.categories = vec![];
        minimal.dependencies = vec![];
        minimal.input_schema = None;

        // A skill with granted capabilities set (permissions path).
        let mut granted = rich_meta("acme.granted");
        let granted_caps = capability::from_legacy(&{
            let mut caps = SkillCapabilities::default();
            caps.filesystem_read = true;
            caps.network = true;
            caps
        });
        granted.granted_capabilities =
            capability::grant_all(&granted_caps, capability::GrantSource::Manifest, true);

        let shapes = [
            sample_meta("acme.smoke"),
            rich_meta("acme.rich"),
            minimal,
            granted,
        ];

        for meta in &shapes {
            let baseline = serde_json::to_string(&extract_profile(meta)).unwrap();
            // N repeated extractions must all equal the first, byte-for-byte.
            for _ in 0..8 {
                let again = serde_json::to_string(&extract_profile(meta)).unwrap();
                assert_eq!(
                    again, baseline,
                    "extract_profile must be deterministic for a fixed SkillMetadata ({})",
                    meta.skill_id
                );
            }
        }
    }

    /// Ordering of `provides`/`consumes`/`inputs` is stable and independent of
    /// the insertion order of the underlying metadata (sorted, deduped output).
    #[test]
    fn extract_profile_ordering_is_stable_regardless_of_input_order() {
        let a = rich_meta("acme.order");
        // Same logical content as `a` but every collection supplied in a
        // different (reversed) insertion order.
        let mut b = a.clone();
        b.categories = a.categories.iter().rev().cloned().collect();
        b.dependencies = a.dependencies.iter().rev().cloned().collect();
        b.input_schema = Some(serde_json::json!({
            "type": "object",
            "properties": {
                "flags": { "type": ["null", "boolean"] },
                "pages": { "type": "array", "items": { "type": "integer" } },
                "path": { "format": "uri", "type": "string" }
            }
        }));

        let pa = extract_profile(&a);
        let pb = extract_profile(&b);

        // Derived vectors are identically ordered despite reordered inputs.
        assert_eq!(pa.provides, pb.provides);
        assert_eq!(pa.consumes, pb.consumes);
        assert_eq!(pa.inputs, pb.inputs);

        // And each derived list is itself sorted (stable lexicographic order).
        let provides_ids: Vec<&str> = pa.provides.iter().map(|t| t.id.as_str()).collect();
        let mut sorted = provides_ids.clone();
        sorted.sort_unstable();
        assert_eq!(
            provides_ids, sorted,
            "provides must be lexicographically sorted"
        );

        let consumes_ids: Vec<&str> = pa.consumes.iter().map(|t| t.id.as_str()).collect();
        let mut sorted_c = consumes_ids.clone();
        sorted_c.sort_unstable();
        assert_eq!(
            consumes_ids, sorted_c,
            "consumes must be lexicographically sorted"
        );

        let mut sorted_i = pa.inputs.clone();
        sorted_i.sort();
        assert_eq!(
            pa.inputs, sorted_i,
            "inputs must be lexicographically sorted"
        );

        // `a` itself remains deterministic vs a fresh clone (no hidden state).
        assert_eq!(
            serde_json::to_string(&pa).unwrap(),
            serde_json::to_string(&extract_profile(&a.clone())).unwrap()
        );
    }
}
