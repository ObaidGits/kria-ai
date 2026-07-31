//! Canonical legacy → v2 model mapping and cutover ledger (task F2.1.6).
//!
//! # One canonical model (MGR-002, MGR-034)
//!
//! The `model` module is the **single canonical typed model** for the memory
//! graph. Everything downstream — the graph projection (F2.2+), five-strategy
//! retrieval (F3), the API surface (F3.9), and the UI contracts (F4) — builds
//! on the `model` types, never on a second parallel model. There is exactly one
//! conceptual model; where a *legacy row representation* still lingers it is a
//! transitional persistence detail, not a competing model.
//!
//! # Why the legacy structs still exist
//!
//! Fully deleting the duplicate legacy fact/memory/entity/goal structs today
//! would break the build **and** the live memory path, because the governed v2
//! write path (F1.5) and retrieval-on-v2 (F3) do not exist yet: the v2 record
//! tables (migration 0017) have no data because nothing writes them through the
//! authority path, so every live read still comes from the legacy tables. This
//! module is therefore the authoritative record of:
//!
//!   1. each duplicate legacy struct that overlaps the canonical v2 model,
//!   2. its canonical v2 replacement,
//!   3. its current live consumers, and
//!   4. the **gate** whose completion finally removes it.
//!
//! Nothing here is a runtime type — it introduces no third representation. It is
//! documentation with compiler-checked intra-doc links, so if a mapped type is
//! renamed or removed the doc build breaks and this ledger cannot silently rot.
//!
//! # Cutover gates
//!
//! * **F1.5 — governed writer cutover.** Routes all durable creation through
//!   `AuthorityTx` into the v2 `records`/`entities_v2`/`goals_v2`/`sources`
//!   tables. Retires the legacy write paths
//!   ([`crate::memory::runtime_backend::KriaMemoryRuntime`] fact writes, the
//!   legacy [`crate::memory::write_policy`] `Memory` path, the direct
//!   [`crate::memory::goals::GoalStore`] table writes).
//! * **F2.2 — relation registry + Memory Links canonical.** Replaces the
//!   free-text [`crate::memory::types::Relationship`] edge and the ad-hoc graph
//!   entity read model with the registry-governed relationship + typed
//!   endpoints over [`crate::memory::model::Entity`].
//! * **F3 — retrieval-on-v2.** Repoints retrieval/search/graph reads (and the
//!   API DTOs they feed) at the v2 [`crate::memory::model::Record`] /
//!   [`crate::memory::model::Entity`] / [`crate::memory::model::Goal`]
//!   projections instead of the legacy tables/structs.
//! * **F4 — UI contracts on v2.** Repoints the desktop/server presentation DTOs
//!   (e.g. the goal command JSON, analytics fact entries) at v2 projections.
//!   Tauri command/event names are contract-frozen and do not change.
//!
//! # Legacy → v2 → consumers → removal gate
//!
//! ## Fact / memory records
//!
//! | Legacy | Canonical v2 | Live consumers | Gate |
//! |---|---|---|---|
//! | [`crate::memory::types::Memory`] | [`crate::memory::model::Record`] (`RecordKind::Memory`) + [`crate::memory::model::Provenance`] | legacy write policy, [`crate::memory::stores`] `sqlite_memory`, retrieval, `api` | F1.5 write + F3 read |
//! | [`crate::memory::runtime_types::MemoryFact`] | [`crate::memory::model::Record`] (`RecordKind::Memory`); decay/access become retrieval-time scoring, not stored columns | [`crate::memory::runtime_backend::KriaMemoryRuntime`], [`crate::memory::manager`] `MemoryManager`/`MemoryReader`, `tools::knowledge`, desktop history helpers | F1.5 write + F3 read |
//! | [`crate::memory::types::Event`] | authority `events` + [`crate::memory::model::EventId`] / [`crate::memory::model::Provenance`] creation event | append-only event log, write path | F1.5 |
//! | [`crate::memory::types::WriteCandidate`] / `WriteDecision` | `AuthorityTx` command inputs/outputs ([`crate::memory::authority`]) | legacy write policy, tools | F1.5 |
//!
//! ## Provenance / source
//!
//! | Legacy | Canonical v2 | Live consumers | Gate |
//! |---|---|---|---|
//! | [`crate::memory::types::Source`] (provenance-tag enum) | structured [`crate::memory::model::Provenance`] ([`crate::memory::model::Actor`] / [`crate::memory::model::Method`] / [`crate::memory::model::ModelIdentity`] / [`crate::memory::model::Locator`] / [`crate::memory::model::ParentRef`]) + [`crate::memory::model::SourceRef`] | `types::Event`/`Memory`/`WriteCandidate`, write policy, extraction | F1.5 |
//! | *the consented-source concept* | [`crate::memory::model::SourceRecord`] (`sources` row) with closed [`crate::memory::authority::command::SourceKind`] | source trust/consent (F1) | F1.5 |
//!
//! ## Graph entities / relationships
//!
//! | Legacy | Canonical v2 | Live consumers | Gate |
//! |---|---|---|---|
//! | [`crate::memory::types::Entity`] | [`crate::memory::model::Entity`] (`entities_v2`) + [`crate::memory::model::Alias`] / [`crate::memory::model::Mention`] | [`crate::memory::entity_resolution`], `api` graph methods, [`crate::memory::stores`] `GraphStore`/`sqlite_graph` | F2.2 + F1.5 |
//! | [`crate::memory::types::Relationship`] | F2.2 registry-governed relationship + [`crate::memory::model::Evidence`] (no v2 struct yet — added in F2.2) | [`crate::memory::extraction`], `api`, `GraphStore`/`sqlite_graph` | **Deleted F2.2.7** |
//! | [`crate::memory::types::GraphHit`] | F2.2/F3 traversal result over v2 [`crate::memory::model::Entity`] | `api` graph neighbourhood | **Deleted F2.2.7** |
//!
//! ## Goals
//!
//! | Legacy | Canonical v2 | Live consumers | Gate |
//! |---|---|---|---|
//! | [`crate::memory::goals::Goal`] | [`crate::memory::model::Goal`] (`goals_v2`) + [`crate::memory::model::GoalProgress`] | `api` goal accessor, desktop goal commands, active-learning, dreaming | F1.5 + F3 |
//! | [`crate::memory::goals::GoalStatus`] | [`crate::memory::model::GoalStatus`] — **closed** set. Legacy `failed`/`abandoned` are **not** in the v2 set (which adds `conflicted`/`stale`/`superseded`/`deleted`); the cutover remaps `failed`/`abandoned` at F1.5 (see below). | as above | F1.5 |
//! | [`crate::memory::goals::GoalStore`] / `NewGoal` / `GoalAnalytics` | governed goal commands over `goals_v2` (F1.5) | `api`, desktop commands | F1.5 |
//!
//! ## Legacy `GoalStatus` → v2 [`crate::memory::model::GoalStatus`] remap (applied at F1.5)
//!
//! The two status sets are deliberately different closed sets. The F1.5 cutover
//! remaps the legacy terminal values that the v2 set does not carry:
//!
//! | Legacy | v2 |
//! |---|---|
//! | `candidate` | `candidate` |
//! | `active` | `active` |
//! | `paused` | `paused` |
//! | `completed` | `completed` |
//! | `failed` | `deleted` (governed-terminated, not re-openable) |
//! | `abandoned` | `deleted` |
//!
//! (`conflicted`/`stale`/`superseded` are new v2 dispositions with no legacy
//! source; they arise only from governed v2 transitions.)
//!
//! # Removal ledger status (as of F2.7.5)
//!
//! ## Deleted in F2.7.5 (this task)
//!
//! * **`crate::memory::authority::relationship_migration`** — the
//!   `LegacyRelationshipMigrator` module that reconciled the legacy free-text
//!   `relationships` table into `relationships_v2`.  The legacy table was already
//!   dropped by migration 0024 (F2.2.7); no caller outside the module
//!   referenced it; deleting it removes the only remaining artefact of the
//!   pre-v2 free-text graph path.  The `pub mod` and `pub use` declarations in
//!   `authority/mod.rs` were removed at the same time.
//!
//! # Removal ledger status (as of F2.2.7)
//!
//! ## Deleted in F2.2.7 (this task)
//!
//! * **`crate::memory::types::Relationship`** — legacy free-text edge struct
//!   (free-text `rel_type`, `f32` strength).  All writers and readers have been
//!   redirected to `relationships_v2` or removed.  The struct no longer exists.
//! * **`crate::memory::types::GraphHit`** — legacy BFS traversal result wrapping
//!   the legacy `Entity` + hop path.  Removed; the graph API now returns
//!   `(Uuid, u8)` pairs.  F3.3 introduces the canonical traversal result over
//!   v2 entities with policy, hidden-intermediary omission, and edge metadata.
//! * **Legacy `relationships` table** (+ `ix_rel_source`, `ix_rel_target`,
//!   `graph_2hop_cache`) — dropped by migration 0024.  All write paths
//!   (`sqlite_graph::add_relationship`, `graph_intel::complete_transitive`,
//!   `extraction::add_comention_edges`, `api::create_relationship`) have been
//!   removed or redirected to `relationships_v2`.
//! * **`GraphStore::add_relationship`** and **`GraphStore::relationships_for`** —
//!   removed from the port trait.  The canonical write path is
//!   `RelationshipCommandBus` (F2.2.5).
//!
//! ## Still retained (pending later gates)
//!
//! All fact/memory/entity/goal legacy structs still have ≥1 live consumer in the
//! legacy write/read path; they are removed at F1.5 (writes) + F3 (reads).
//!
//! # Removal ledger status (as of F2.1.6) — superseded by F2.2.7 above
//!
//! * **Deleted now:** none — every duplicate above has ≥1 live consumer, so
//!   deleting any of them would break the build or the live memory path.
//! * **Adapted now:** none — no live consumer can be repointed at v2 before the
//!   v2 write path (F1.5) populates the v2 tables and retrieval-on-v2 (F3)
//!   reads them. Adapting a consumer earlier would read empty v2 tables and
//!   silently drop the user's live memory.
//! * **Retained pending cutover:** all rows above, each annotated at its
//!   definition site with a `superseded by … (task F2.1.6); removed at <gate>`
//!   note pointing back here.
//!
//! Out of scope (not duplicates of this model): the agent operational
//! world-model facts (`agent::world_model::WorldFact`,
//! `agent::uncertainty::belief_graph::BeliefFact`, PSDG facts, grounder facts)
//! are a different domain, and presentation-only adapter DTOs (the desktop goal
//! command JSON, `commands::analytics` fact entries) are UI contracts retired
//! at F4, not core model types.

// Intentionally no items: this module is the canonical mapping/ledger only. The
// doc links above are compiler-checked so the ledger cannot silently drift from
// the code.

// ─── Task 1.9.3 audit note (F1 clean-up inventory re-run) ─────────────────────
//
// Re-run date: task 1.9.3 (post-1.6–1.9.2 work).
//
// 1. WRITE INVENTORY (new direct writes since 1.5.6)
//    Searched: `.execute("INSERT INTO memories`, `.execute("UPDATE memories`,
//    `.execute("INSERT INTO (entities|relationships|goals|records|sources)`.
//    Result: ZERO new direct writes found outside of the authority transaction
//    boundary or documented legacy stores. No new bypass paths were introduced
//    in tasks 1.6–1.9.2.
//
// 2. ANN AUTHORITY ASSUMPTIONS
//    Searched: LanceDB/HNSW/Qdrant as authority, ANN authority.
//    Result: ALL references were derived-index paths (not authority claims).
//    RESOLVED (task 3.1.6): `AnnVectorStore` and `stores/ann_vectors.rs` have
//    been deleted. `SqliteVectorStore` (exact cosine over `mem_vectors_v2`) is
//    the sole vector backend. No ANN/LanceDB/Qdrant/HNSW code remains in the
//    release closure. `stores/ports.rs` `VectorStore` trait comment updated.
//    FIXED: `stores/ports.rs` `VectorStore` trait comment formerly said
//    "LanceDB v1; Qdrant escape hatch" — corrected to document SqliteVectorStore
//    as the durable authority (exact cosine, F3.1 canonical store).
//
// 3. SIMULATED-SUCCESS TESTS
//    Searched: `fn mock_write_policy`, `fn simulated_success`, `fn fake_write`,
//    `mock.*always.*Stored`, `fn mock_authority`, `always_returns_stored`,
//    `fn stub_write`, `fn fake_policy`, `MockWritePolicy`, `always_ok_write`.
//    Result: ZERO simulated-success test helpers found. All existing tests use
//    real in-memory SQLite via `Database::open_in_memory()`.
//
// 4. PERMISSIVE CORS AUDIT
//    Two `CorsLayer::permissive()` usages exist:
//    a) `kria-server/src/origin.rs` `build_cors_layer()` — correctly gated:
//       only returned when `!remote_enabled` (loopback/default path).
//       Remote mode uses a restricted allow-list. CORRECT BY DESIGN.
//    b) `kria-desktop/src/commands/local_api.rs` — Tauri desktop-internal bridge,
//       bound to `cfg.server.host` (defaults to 127.0.0.1, guarded by
//       `auth_middleware` token on every request). Not part of the kria-server
//       remote-mode path. Added an explanatory comment clarifying the scope and
//       that this bridge does not support remote exposure.
//    No new permissive routes were introduced in tasks 1.6–1.9.2.
//
// 5. WHAT WAS NOT CLEANED UP (and why)
//    The legacy write path (WritePolicy → sqlite_memory / GoalStore /
//    KriaMemoryRuntime / FeedbackService / runtime_backend) was NOT deleted.
//    Reason: this is the live persistence path until F2 lands (see gate column
//    in the tables above). Every legacy struct listed in this ledger has ≥1 live
//    consumer; deleting any of them would break the build or silently drop the
//    user's live memory. Cutover gates are F1.5 (writes) and F3 (reads).
