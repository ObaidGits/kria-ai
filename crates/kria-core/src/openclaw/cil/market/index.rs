//! `MarketIndex` — offline-embedded, federated marketplace catalog cache
//! (task 6.2, design §7.4 / §8.2, R9.2 / R9.4).
//!
//! `MarketIndex` is the read-side of the federated marketplace. It syncs every
//! configured [`MarketplaceProvider`], **embeds each catalog entry offline at
//! sync time**, and persists the vector plus ranking metadata into the
//! `market_catalog` derived table (frozen registry migration 4) in the SAME
//! `skills.db` — there is **no second database**. Discovery
//! ([`MarketIndex::search`]) is then a pure DB read + cosine rank over the
//! pre-embedded `market_catalog.embedding` BLOBs: it **never performs a live
//! per-query marketplace fetch** (R9.2). If the catalog is empty (never synced /
//! offline), search honestly returns nothing rather than reaching out.
//!
//! # Single source of truth (R5.1)
//!
//! `market_catalog` is a **rebuildable derived view**: every row is re-derivable
//! by re-syncing the providers, and each write is an `INSERT OR REPLACE` keyed by
//! the table PK `(provider_id, slug)`, so a re-sync is idempotent. The store
//! issues **no DDL** — the table is owned by the frozen registry migrations.
//!
//! # What is embedded, and what `manifest_json` holds
//!
//! The embedding text is the entry's `name`, `description`, `category`, and
//! `capabilities_summary` joined — the same open-vocabulary surface installed
//! discovery embeds, so a market candidate and an installed candidate are
//! comparable in the same vector space (reusing the **frozen** [`Embedder`] and
//! the [`encode_embedding`]/[`decode_embedding`] BLOB layout shared with
//! `capability_profiles`).
//!
//! `market_catalog.manifest_json` is documented in the design as the skill
//! manifest. To honor R9.2 (**no live per-query fetch**) and keep sync cheap
//! (§8.2 — a provider only reads catalogs; manifests are fetched lazily), this
//! task stores a **minimal JSON projection of the known [`MarketEntry`] fields**
//! (slug, name, description, category, version, manifest_url,
//! capabilities_summary, declared_trust) rather than fetching the full manifest
//! per entry at sync time. This is an honest, documented trade-off: the full
//! manifest is fetched at install time via
//! [`MarketplaceProvider::fetch_manifest`] (the frozen path), and `manifest_url`
//! is preserved here so that fetch needs no re-sync. A richer provider or a
//! later phase can replace the projection with the fetched manifest without
//! changing this table's schema or any reader.
//!
//! # Metadata recorded per entry (R9.4)
//!
//! Every row records `version`, `deprecated`, `trust_hint` (via
//! [`MarketplaceProvider::trust_hint`], stored as the lower-case
//! [`TrustTier`] string), `quality`, and `popularity` — the full set of ranking
//! signals §7.4 requires (`quality`/`popularity` are `NULL` when the provider
//! supplies none, honestly absent rather than fabricated), plus `fetched_at`
//! (RFC3339) for staleness.
//!
//! # Scope boundaries
//!
//! Incremental sync (`fetched_at` + content change-detection; provider-level
//! ETag is a documented future short-circuit), **bounded-concurrency** provider
//! sync, and **offline fallback** (stale cache + [`MarketCandidate::offline`]
//! flagging) are task 6.3 and now live here. Facade wiring (mapping
//! [`MarketCandidate`] into the ranked candidate set) is task 6.4; the
//! idempotent-reindex property test is task 6.5.
//!
//! [`encode_embedding`]: crate::openclaw::cil::profile::encode_embedding
//! [`decode_embedding`]: crate::openclaw::cil::profile::decode_embedding

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use futures::stream::{self, StreamExt};
use rusqlite::{params, Connection};

use super::super::embed::Embedder;
use super::super::profile::{decode_embedding, encode_embedding};
use super::super::CilError;
use super::provider::{MarketEntry, MarketplaceProvider};
use crate::openclaw::types::TrustTier;

/// A marketplace candidate discovered from the pre-embedded `market_catalog`
/// (design §7.3 — the `Marketplace` candidate source).
///
/// This is a **market-local** result type: the facade (task 6.4) maps it into
/// the shared `CapabilityCandidate` / `CandidateSource::Marketplace` shape when
/// merging market results with installed-skill results. Keeping it local avoids
/// editing `cil::index::CandidateSource` concurrently with task 3.3/6.4.
///
/// It carries exactly the `market_catalog` columns a ranker needs: the identity
/// `(provider_id, slug)`, the `version`/`deprecated` version-awareness signals,
/// the effective `trust_hint`, the optional `quality`/`popularity` ranking
/// signals, and the `score` (cosine similarity of the goal against the entry's
/// pre-embedded vector, clamped to `0.0..=1.0`).
#[derive(Debug, Clone, PartialEq)]
pub struct MarketCandidate {
    /// Which marketplace this candidate came from (`market_catalog.provider_id`).
    pub provider_id: String,
    /// Stable skill identifier (`market_catalog.slug`).
    pub slug: String,
    /// Semver string (`market_catalog.version`) — drives version-awareness.
    pub version: String,
    /// Effective trust tier recorded at sync time (`market_catalog.trust_hint`).
    /// `None` only if the column was somehow NULL (older/manual rows).
    pub trust_hint: Option<TrustTier>,
    /// Quality signal, if the provider supplied one (`market_catalog.quality`).
    pub quality: Option<f64>,
    /// Popularity signal, if the provider supplied one (`market_catalog.popularity`).
    pub popularity: Option<f64>,
    /// Whether the entry is deprecated (`market_catalog.deprecated`).
    pub deprecated: bool,
    /// Cosine similarity of the goal embedding against the entry's pre-embedded
    /// vector, clamped to `0.0..=1.0`.
    pub score: f32,
    /// `true` when this candidate's `provider_id` was **unreachable during the
    /// most recent [`MarketIndex::sync`]** and its rows are therefore served
    /// stale from cache (R13.3). The candidate is still returned — a stale hit
    /// beats no hit — but the caller MUST surface it as "offline" and never
    /// present it as fresh (honesty invariant). `false` for a provider that
    /// synced successfully (or was never marked offline).
    pub offline: bool,
}

/// The outcome of a [`MarketIndex::sync`] cycle (task 6.3, R9.5 / R13.3).
///
/// Reports what changed so callers know the catalog's freshness without a
/// second query: how many rows were (re-)embedded and written (`upserted`), how
/// many were skipped as unchanged by incremental change-detection (`skipped`),
/// and which providers were **unreachable this cycle** (`offline_providers`).
/// A provider listed in `offline_providers` had its previously-cached rows left
/// **intact and stale** — never dropped — and its candidates are flagged
/// [`MarketCandidate::offline`] on the next [`MarketIndex::search`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncReport {
    /// Rows re-embedded and written this cycle (new or content-changed entries).
    pub upserted: usize,
    /// Rows skipped as unchanged (same version + same manifest projection),
    /// avoiding needless re-embedding and re-writing (R9.5 incremental sync).
    pub skipped: usize,
    /// Providers that were unreachable this cycle; their cached rows are served
    /// stale and their candidates flagged offline (R13.3). Sorted for
    /// determinism.
    pub offline_providers: Vec<String>,
}

/// The per-provider result of one sync pass, aggregated into a [`SyncReport`].
struct ProviderSyncOutcome {
    provider_id: String,
    upserted: usize,
    skipped: usize,
    /// `true` when the provider was unreachable (its stale cache is retained).
    offline: bool,
}

/// Offline-embedded, federated marketplace catalog over `market_catalog`
/// (design §8.2 / §7.4).
///
/// Holds an `Arc<Mutex<Connection>>` to `skills.db` (the SAME database as
/// [`ProductionSkillRegistry`] / `ProfileStore` / `GrantStore` — no second
/// database), an `Arc<dyn Embedder>` (the frozen embedding backend, reused for
/// offline embedding), and the set of federated
/// [`MarketplaceProvider`]s. Issues no DDL: the `market_catalog` table is created
/// by the frozen registry migrations (migration 4).
///
/// [`ProductionSkillRegistry`]: crate::openclaw::registry::ProductionSkillRegistry
pub struct MarketIndex {
    db: Arc<Mutex<Connection>>,
    embedder: Arc<dyn Embedder>,
    providers: Vec<Arc<dyn MarketplaceProvider>>,
    /// Providers that were unreachable during the most recent [`sync`]. Read by
    /// [`search`] to flag affected candidates offline (R13.3). Empty until the
    /// first sync marks a provider offline; refreshed wholesale each sync so a
    /// provider that recovers is no longer flagged.
    ///
    /// [`sync`]: MarketIndex::sync
    /// [`search`]: MarketIndex::search
    offline_providers: Arc<Mutex<HashSet<String>>>,
}

/// Bounded concurrency limit for federated provider sync (R9.5 — "bounded work
/// queue"). Providers are polled concurrently via [`buffer_unordered`] with at
/// most this many in flight, so federation never spawns an unbounded number of
/// tasks (KRIA: no uncontrolled loops / task explosion). A small fixed cap is
/// plenty — sync is I/O-bound and runs off the hot path.
///
/// [`buffer_unordered`]: futures::stream::StreamExt::buffer_unordered
const SYNC_CONCURRENCY: usize = 4;

impl MarketIndex {
    /// Open an additional connection to `skills.db` for the market index.
    ///
    /// Opens the SAME database file the registry uses (never a second database)
    /// and enables WAL for concurrent reads, matching `ProfileStore::open` /
    /// `GrantStore::open` / `CapabilityGraph::open`. The `market_catalog` table is
    /// created by the frozen registry migrations (migration 4); construct the
    /// registry first (or otherwise run migrations) so the table exists. This
    /// store issues no DDL.
    pub fn open(
        db_path: &Path,
        embedder: Arc<dyn Embedder>,
        providers: Vec<Arc<dyn MarketplaceProvider>>,
    ) -> Result<Self, CilError> {
        let conn = Connection::open(db_path)
            .map_err(|e| CilError::Io(format!("open skills.db for market index: {e}")))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| CilError::Io(format!("enable WAL for market index: {e}")))?;
        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
            embedder,
            providers,
            offline_providers: Arc::new(Mutex::new(HashSet::new())),
        })
    }

    /// Build a `MarketIndex` over an already-open, shared `skills.db` connection.
    ///
    /// Preferred when the registry's connection is available: it keeps every
    /// writer on one connection and one source of truth.
    pub fn from_shared_connection(
        db: Arc<Mutex<Connection>>,
        embedder: Arc<dyn Embedder>,
        providers: Vec<Arc<dyn MarketplaceProvider>>,
    ) -> Self {
        Self {
            db,
            embedder,
            providers,
            offline_providers: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// The number of configured federated providers.
    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }

    /// The open-vocabulary text embedded offline for an entry: `name`,
    /// `description`, `category`, and `capabilities_summary` joined.
    ///
    /// This mirrors the installed-discovery lexical/semantic surface so a market
    /// candidate and an installed candidate live in the same vector space.
    fn embedding_text(entry: &MarketEntry) -> String {
        let mut parts: Vec<&str> = vec![&entry.name, &entry.description, &entry.category];
        for c in &entry.capabilities_summary {
            parts.push(c);
        }
        parts.join(" ")
    }

    /// A minimal JSON projection of the known [`MarketEntry`] fields, stored as
    /// `market_catalog.manifest_json` (see module docs for why this is not the
    /// full manifest — R9.2 avoids a fetch-per-entry at sync time).
    fn manifest_projection(entry: &MarketEntry) -> Result<String, CilError> {
        let value = serde_json::json!({
            "provider_id": entry.provider_id,
            "slug": entry.slug,
            "name": entry.name,
            "description": entry.description,
            "category": entry.category,
            "version": entry.version,
            "manifest_url": entry.manifest_url,
            "declared_trust": entry.declared_trust,
            "capabilities_summary": entry.capabilities_summary,
        });
        serde_json::to_string(&value)
            .map_err(|e| CilError::Io(format!("serialize manifest_json projection: {e}")))
    }

    /// Incrementally sync every configured provider under a **bounded work
    /// queue**, embed only new/changed entries offline, UPSERT them into
    /// `market_catalog`, and fall back to the stale cache for any unreachable
    /// provider (design §8.2 / §7.4, R9.2 / R9.4 / **R9.5** / **R13.3**).
    ///
    /// # Bounded concurrency (R9.5)
    ///
    /// Providers are processed **concurrently** — not strictly sequentially —
    /// via [`buffer_unordered`] with at most [`SYNC_CONCURRENCY`] in flight, so
    /// federation is a bounded work queue with **no unbounded task spawn** (no
    /// `tokio::spawn` per provider; the futures share this task). Results are
    /// aggregated into a [`SyncReport`]; a single provider failing does **not**
    /// abort the others (see offline fallback).
    ///
    /// # Incremental sync (R9.5)
    ///
    /// For each entry the stored row's fingerprint `(version, manifest_json)` is
    /// compared against the incoming entry. **Unchanged** entries (same version
    /// **and** same manifest projection) are **skipped** — no re-embedding, no
    /// re-write — and counted in [`SyncReport::skipped`]. Only new/changed
    /// entries are embedded via the frozen [`Embedder`] and written
    /// `INSERT OR REPLACE` keyed by `(provider_id, slug)`, recording
    /// `manifest_json`, `version`, the offline `embedding` (little-endian `f32`
    /// BLOB via [`encode_embedding`]), `trust_hint`, `quality`, `popularity`,
    /// `deprecated`, and a fresh `fetched_at`. The `manifest_json` projection is
    /// the content hash here (it already folds in `version`, `name`,
    /// `description`, `category`, `capabilities_summary`, `manifest_url`, and
    /// `declared_trust` — everything the offline embedding text derives from),
    /// so a change to any embedding-relevant field re-embeds while a pure
    /// no-op re-sync writes nothing. A provider-level **ETag** — once a provider
    /// exposes one — can later short-circuit the *whole* provider before this
    /// per-entry check even runs; [`MarketEntry`]/[`MarketplaceProvider`] carry
    /// no ETag today, so `fetched_at` + this content comparison is the honest,
    /// bounded stand-in.
    ///
    /// # Offline fallback (R13.3)
    ///
    /// If a provider's [`sync_index`](MarketplaceProvider::sync_index) errors
    /// (unreachable), its previously-cached `market_catalog` rows are left
    /// **intact and stale** (never dropped) and the provider id is recorded in
    /// [`SyncReport::offline_providers`] and in this index's tracked offline set,
    /// so [`search`](MarketIndex::search) flags affected candidates
    /// [`offline`](MarketCandidate::offline). The offline set is refreshed
    /// wholesale each sync, so a provider that recovers stops being flagged.
    ///
    /// A hard [`Embedder`] failure (not a provider being unreachable) is still
    /// surfaced as `Err` — that is degraded mode, reported honestly, not a
    /// masked success.
    ///
    /// [`buffer_unordered`]: futures::stream::StreamExt::buffer_unordered
    pub async fn sync(&self) -> Result<SyncReport, CilError> {
        let fetched_at = chrono::Utc::now().to_rfc3339();

        // Bounded work queue: poll providers concurrently, at most
        // SYNC_CONCURRENCY in flight, no per-provider task spawn (R9.5).
        let outcomes: Vec<Result<ProviderSyncOutcome, CilError>> =
            stream::iter(self.providers.iter().cloned())
                .map(|provider| {
                    let fetched_at = fetched_at.clone();
                    async move { self.sync_one_provider(provider, &fetched_at).await }
                })
                .buffer_unordered(SYNC_CONCURRENCY)
                .collect()
                .await;

        let mut report = SyncReport::default();
        for outcome in outcomes {
            // Propagate a hard failure (e.g. embedder unavailable) honestly.
            let outcome = outcome?;
            report.upserted += outcome.upserted;
            report.skipped += outcome.skipped;
            if outcome.offline {
                report.offline_providers.push(outcome.provider_id);
            }
        }
        // Deterministic ordering for callers/tests.
        report.offline_providers.sort();

        // Publish the offline set for search() to flag stale candidates (R13.3),
        // refreshed wholesale so recovered providers are no longer flagged.
        {
            let mut guard = self
                .offline_providers
                .lock()
                .map_err(|_| CilError::Io("market index offline set poisoned".into()))?;
            *guard = report.offline_providers.iter().cloned().collect();
        }

        Ok(report)
    }

    /// Sync a single provider: fetch its index, incrementally upsert changed
    /// entries, or fall back to its stale cache when unreachable.
    ///
    /// Returns `Ok(ProviderSyncOutcome { offline: true, .. })` when the provider
    /// is unreachable (its cached rows are retained — R13.3), and `Err` only for
    /// a hard failure such as the embedder being unavailable (honest degraded
    /// mode, not a masked success).
    async fn sync_one_provider(
        &self,
        provider: Arc<dyn MarketplaceProvider>,
        fetched_at: &str,
    ) -> Result<ProviderSyncOutcome, CilError> {
        let provider_id = provider.provider_id().to_string();

        // A future provider-level ETag check would short-circuit here before any
        // per-entry work; no ETag is available on the trait today.
        let entries = match provider.sync_index().await {
            Ok(entries) => entries,
            Err(_unreachable) => {
                // Provider unreachable → keep its stale rows, flag offline. Do
                // NOT propagate: other providers must still sync (R13.3).
                return Ok(ProviderSyncOutcome {
                    provider_id,
                    upserted: 0,
                    skipped: 0,
                    offline: true,
                });
            }
        };

        let mut upserted = 0usize;
        let mut skipped = 0usize;
        for entry in &entries {
            let manifest_json = Self::manifest_projection(entry)?;

            // Incremental change detection: skip unchanged entries (same version
            // + same manifest projection) — no re-embed, no re-write (R9.5).
            if let Some((stored_version, stored_manifest)) =
                self.stored_fingerprint(&entry.provider_id, &entry.slug)?
            {
                if stored_version == entry.version && stored_manifest == manifest_json {
                    skipped += 1;
                    continue;
                }
            }

            // New or changed → embed offline at sync time (never at query time —
            // R9.2) and upsert.
            let text = Self::embedding_text(entry);
            let vector = self.embedder.embed(&text).await?;
            self.upsert_entry(provider.as_ref(), entry, &vector, fetched_at)?;
            upserted += 1;
        }

        Ok(ProviderSyncOutcome {
            provider_id,
            upserted,
            skipped,
            offline: false,
        })
    }

    /// Read the stored change-detection fingerprint `(version, manifest_json)`
    /// for a `(provider_id, slug)` row, or `None` if the entry is not yet cached.
    ///
    /// This is the incremental-sync probe (R9.5): comparing the incoming entry's
    /// `(version, manifest projection)` against this fingerprint decides whether
    /// re-embedding is needed, without adding any schema column (migrations are
    /// frozen/additive-only — no change here).
    fn stored_fingerprint(
        &self,
        provider_id: &str,
        slug: &str,
    ) -> Result<Option<(String, String)>, CilError> {
        let db = self
            .db
            .lock()
            .map_err(|_| CilError::Io("market index connection poisoned".into()))?;
        db.query_row(
            "SELECT version, manifest_json FROM market_catalog
             WHERE provider_id = ?1 AND slug = ?2",
            params![provider_id, slug],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(CilError::Io(format!(
                "read market_catalog fingerprint ({provider_id}, {slug}): {other}"
            ))),
        })
    }

    /// Persist one embedded entry into `market_catalog` (insert or replace by
    /// `(provider_id, slug)`). Records the full R9.4 metadata set.
    fn upsert_entry(
        &self,
        provider: &dyn MarketplaceProvider,
        entry: &MarketEntry,
        embedding: &[f32],
        fetched_at: &str,
    ) -> Result<(), CilError> {
        let manifest_json = Self::manifest_projection(entry)?;
        let embedding_blob = encode_embedding(embedding);
        let trust_hint = provider.trust_hint(entry).as_str();
        let deprecated: i64 = if entry.deprecated { 1 } else { 0 };

        let db = self
            .db
            .lock()
            .map_err(|_| CilError::Io("market index connection poisoned".into()))?;
        db.execute(
            "INSERT OR REPLACE INTO market_catalog (
                provider_id, slug, manifest_json, version, embedding,
                trust_hint, quality, popularity, deprecated, fetched_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                entry.provider_id,
                entry.slug,
                manifest_json,
                entry.version,
                embedding_blob,
                trust_hint,
                entry.quality,
                entry.popularity,
                deprecated,
                fetched_at,
            ],
        )
        .map_err(|e| {
            CilError::Io(format!(
                "persist market_catalog row ({}, {}): {e}",
                entry.provider_id, entry.slug
            ))
        })?;
        Ok(())
    }

    /// Offline cosine search over the pre-embedded `market_catalog` (R9.2).
    ///
    /// Reads every row's pre-embedded `embedding` BLOB
    /// ([`decode_embedding`]), cosine-ranks it against `goal_embedding`, and
    /// returns the top `k` [`MarketCandidate`]s. This is a **pure DB read** — it
    /// **never** performs a live marketplace fetch during discovery. An empty
    /// catalog (never synced / offline) yields an empty result honestly.
    ///
    /// Ordering is deterministic: descending `score` with a stable tie-break by
    /// `(provider_id, slug)`.
    ///
    /// # Offline flagging (R13.3)
    ///
    /// Any candidate whose `provider_id` was unreachable during the most recent
    /// [`sync`](MarketIndex::sync) is returned with
    /// [`offline = true`](MarketCandidate::offline): its row is served stale from
    /// cache (a stale hit beats no hit), but callers MUST surface it as offline
    /// and never present it as fresh (honesty invariant). Providers that synced
    /// successfully yield `offline = false`.
    pub fn search(
        &self,
        goal_embedding: &[f32],
        k: usize,
    ) -> Result<Vec<MarketCandidate>, CilError> {
        if k == 0 {
            return Ok(Vec::new());
        }
        // Snapshot the offline-provider set from the most recent sync (R13.3).
        let offline_providers: HashSet<String> = self
            .offline_providers
            .lock()
            .map_err(|_| CilError::Io("market index offline set poisoned".into()))?
            .clone();
        let db = self
            .db
            .lock()
            .map_err(|_| CilError::Io("market index connection poisoned".into()))?;
        let mut stmt = db
            .prepare(
                "SELECT provider_id, slug, version, embedding,
                        trust_hint, quality, popularity, deprecated
                 FROM market_catalog",
            )
            .map_err(|e| CilError::Io(format!("prepare market_catalog search: {e}")))?;

        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,          // provider_id
                    row.get::<_, String>(1)?,          // slug
                    row.get::<_, String>(2)?,          // version
                    row.get::<_, Option<Vec<u8>>>(3)?, // embedding
                    row.get::<_, Option<String>>(4)?,  // trust_hint
                    row.get::<_, Option<f64>>(5)?,     // quality
                    row.get::<_, Option<f64>>(6)?,     // popularity
                    row.get::<_, i64>(7)?,             // deprecated
                ))
            })
            .map_err(|e| CilError::Io(format!("query market_catalog: {e}")))?;

        let mut candidates: Vec<MarketCandidate> = Vec::new();
        for r in rows {
            let (
                provider_id,
                slug,
                version,
                embedding,
                trust_hint,
                quality,
                popularity,
                deprecated,
            ) = r.map_err(|e| CilError::Io(format!("read market_catalog row: {e}")))?;

            // Rows without an embedding cannot be semantically ranked; skip them
            // honestly (they remain in the catalog for metadata reads).
            let Some(bytes) = embedding else { continue };
            let vector = decode_embedding(&bytes)?;
            let score = cosine_similarity(goal_embedding, &vector).clamp(0.0, 1.0);
            let offline = offline_providers.contains(&provider_id);

            candidates.push(MarketCandidate {
                provider_id,
                slug,
                version,
                trust_hint: trust_hint.as_deref().map(parse_trust_tier),
                quality,
                popularity,
                deprecated: deprecated != 0,
                score,
                offline,
            });
        }

        // Deterministic ordering: descending score, stable tie-break by
        // (provider_id, slug).
        candidates.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.provider_id.cmp(&b.provider_id))
                .then_with(|| a.slug.cmp(&b.slug))
        });
        candidates.truncate(k);
        Ok(candidates)
    }
}

/// Cosine similarity between two equal-length vectors; `0.0` for mismatched or
/// empty inputs (honest, never a panic — mirrors `resolver`/`dense`).
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a.sqrt() * norm_b.sqrt())
}

/// Parse a stored `trust_hint` string (lower-case [`TrustTier::as_str`]) back
/// into a [`TrustTier`]. Unknown/empty values fall back to
/// [`TrustTier::Untrusted`] (deny-by-default).
fn parse_trust_tier(s: &str) -> TrustTier {
    match s.trim().to_ascii_lowercase().as_str() {
        "verified" => TrustTier::Verified,
        "community" => TrustTier::Community,
        "local" => TrustTier::Local,
        _ => TrustTier::Untrusted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openclaw::registry::ProductionSkillRegistry;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tempfile::tempdir;

    /// Deterministic mock embedder: maps text length → a small fixed-dim vector
    /// so tests are reproducible without model downloads or network.
    struct MockEmbedder {
        dim: usize,
    }

    #[async_trait]
    impl Embedder for MockEmbedder {
        async fn embed(&self, text: &str) -> Result<Vec<f32>, CilError> {
            // Deterministic bag-of-tokens hashing: each lowercase token is hashed
            // into a fixed bucket and its count incremented. Shared vocabulary
            // between two texts drives up cosine similarity, so this is a faithful
            // (if crude) stand-in for a real embedder — no model download/network.
            let mut v = vec![0.0f32; self.dim];
            for tok in text
                .to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|t| !t.is_empty())
            {
                let mut h: u64 = 1469598103934665603; // FNV-1a offset basis
                for b in tok.bytes() {
                    h ^= b as u64;
                    h = h.wrapping_mul(1099511628211);
                }
                v[(h as usize) % self.dim] += 1.0;
            }
            Ok(v)
        }
        async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, CilError> {
            let mut out = Vec::with_capacity(texts.len());
            for t in texts {
                out.push(self.embed(t).await?);
            }
            Ok(out)
        }
        fn dim(&self) -> usize {
            self.dim
        }
        fn model_id(&self) -> &str {
            "mock-embedder-v1"
        }
    }

    /// A mock provider with **interior mutability** so tests can (a) swap its
    /// catalog between syncs (incremental change-detection) and (b) toggle it
    /// "unreachable" to exercise the offline fallback — all with no network.
    struct MockProvider {
        id: String,
        entries: Mutex<Vec<MarketEntry>>,
        unreachable: AtomicBool,
    }

    impl MockProvider {
        fn new(id: &str, entries: Vec<MarketEntry>) -> Self {
            Self {
                id: id.into(),
                entries: Mutex::new(entries),
                unreachable: AtomicBool::new(false),
            }
        }
        /// Replace the catalog returned by the next `sync_index`.
        fn set_entries(&self, entries: Vec<MarketEntry>) {
            *self.entries.lock().unwrap() = entries;
        }
        /// Toggle whether the next `sync_index` errors as if the marketplace were
        /// unreachable (R13.3 offline fallback).
        fn set_unreachable(&self, v: bool) {
            self.unreachable.store(v, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl MarketplaceProvider for MockProvider {
        fn provider_id(&self) -> &str {
            &self.id
        }
        async fn sync_index(&self) -> Result<Vec<MarketEntry>, CilError> {
            if self.unreachable.load(Ordering::SeqCst) {
                return Err(CilError::Market(format!(
                    "provider '{}' unreachable (mock)",
                    self.id
                )));
            }
            Ok(self.entries.lock().unwrap().clone())
        }
        async fn fetch_manifest(&self, slug: &str) -> Result<String, CilError> {
            Err(CilError::Market(format!(
                "mock has no manifest for '{slug}'"
            )))
        }
        fn trust_hint(&self, entry: &MarketEntry) -> TrustTier {
            // Clamp declared "verified" down to Community (never elevate remote).
            parse_trust_tier(&entry.declared_trust).max(TrustTier::Community)
        }
    }

    fn entry(provider: &str, slug: &str, desc: &str, deprecated: bool) -> MarketEntry {
        MarketEntry {
            provider_id: provider.into(),
            slug: slug.into(),
            name: slug.into(),
            description: desc.into(),
            category: "developer".into(),
            version: "1.0.0".into(),
            manifest_url: format!("https://example.com/{slug}/SKILL.md"),
            declared_trust: "community".into(),
            capabilities_summary: vec!["subprocess".into()],
            quality: Some(0.7),
            popularity: Some(0.5),
            deprecated,
        }
    }

    /// Open a real skills.db with the frozen migrations applied (so
    /// `market_catalog` exists, migration 4), then open the market index over the
    /// same file (WAL allows the concurrent connection), mirroring
    /// `ProfileStore`/`CapabilityGraph` test setup.
    fn test_index(entries: Vec<MarketEntry>) -> (MarketIndex, tempfile::TempDir) {
        let (index, _providers, dir) = test_index_with(vec![("mock", entries)]);
        (index, dir)
    }

    /// Build a `MarketIndex` over a real migrated `skills.db` with one or more
    /// [`MockProvider`]s, returning the concrete provider handles so tests can
    /// mutate their catalogs / toggle reachability between syncs.
    fn test_index_with(
        providers: Vec<(&str, Vec<MarketEntry>)>,
    ) -> (MarketIndex, Vec<Arc<MockProvider>>, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("skills.db");
        // Frozen registry migrations create the market_catalog table (migration 4).
        let _registry = ProductionSkillRegistry::new(&db_path).expect("registry migrations");
        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder { dim: 16 });
        let handles: Vec<Arc<MockProvider>> = providers
            .into_iter()
            .map(|(id, entries)| Arc::new(MockProvider::new(id, entries)))
            .collect();
        let dyn_providers: Vec<Arc<dyn MarketplaceProvider>> = handles
            .iter()
            .map(|p| p.clone() as Arc<dyn MarketplaceProvider>)
            .collect();
        let index =
            MarketIndex::open(&db_path, embedder, dyn_providers).expect("market index open");
        (index, handles, dir)
    }

    #[tokio::test]
    async fn sync_persists_rows_with_embedding_and_metadata() {
        let entries = vec![
            entry("mock", "oc_alpha", "compress and archive files", false),
            entry("mock", "oc_beta", "send email over smtp", true),
        ];
        let (index, _dir) = test_index(entries);

        let report = index.sync().await.expect("sync ok");
        assert_eq!(report.upserted, 2, "both entries upserted");
        assert_eq!(report.skipped, 0, "nothing skipped on first sync");
        assert!(report.offline_providers.is_empty(), "provider reachable");

        // Verify rows persisted with embedding + all R9.4 metadata.
        let db = index.db.lock().unwrap();
        let (version, has_embedding, trust_hint, quality, popularity, deprecated): (
            String,
            bool,
            String,
            f64,
            f64,
            i64,
        ) = db
            .query_row(
                "SELECT version, embedding IS NOT NULL, trust_hint, quality, popularity, deprecated
                 FROM market_catalog WHERE provider_id = 'mock' AND slug = 'oc_beta'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .expect("row present");
        assert_eq!(version, "1.0.0");
        assert!(
            has_embedding,
            "embedding BLOB persisted offline at sync time"
        );
        assert_eq!(trust_hint, "community", "trust_hint recorded (R9.4)");
        assert!((quality - 0.7).abs() < 1e-9, "quality recorded (R9.4)");
        assert!(
            (popularity - 0.5).abs() < 1e-9,
            "popularity recorded (R9.4)"
        );
        assert_eq!(deprecated, 1, "deprecation recorded (R9.4)");
    }

    #[tokio::test]
    async fn resync_is_idempotent_by_provider_slug() {
        let entries = vec![entry("mock", "oc_alpha", "compress files", false)];
        let (index, _dir) = test_index(entries);

        index.sync().await.expect("first sync");
        index.sync().await.expect("second sync");

        let db = index.db.lock().unwrap();
        let n: i64 = db
            .query_row("SELECT COUNT(*) FROM market_catalog", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            n, 1,
            "re-sync replaces by (provider_id, slug), no duplicate"
        );
    }

    #[tokio::test]
    async fn search_ranks_from_cache_without_network() {
        let entries = vec![
            entry(
                "mock",
                "oc_archive",
                "compress and archive zip files",
                false,
            ),
            entry("mock", "oc_email", "send an email message", false),
        ];
        let (index, _dir) = test_index(entries);
        index.sync().await.expect("sync ok");

        // Build a goal embedding by embedding a query with the SAME mock embedder
        // (offline). Search must be a pure cache read.
        let embedder = MockEmbedder { dim: 16 };
        let goal = embedder
            .embed("compress and archive zip files")
            .await
            .unwrap();

        let results = index.search(&goal, 5).expect("search ok");
        assert_eq!(results.len(), 2, "both cached entries ranked");
        // The archive entry (identical text) should rank first.
        assert_eq!(results[0].slug, "oc_archive");
        assert!(
            results[0].score >= results[1].score,
            "descending score order"
        );
        // Metadata surfaced from the cache.
        assert_eq!(results[0].trust_hint, Some(TrustTier::Community));
        assert_eq!(results[0].quality, Some(0.7));
        assert_eq!(results[0].popularity, Some(0.5));
        assert!(!results[0].deprecated);
    }

    #[tokio::test]
    async fn search_on_empty_catalog_returns_empty() {
        let (index, _dir) = test_index(vec![]);
        // No sync performed → empty catalog. Search must not fetch; returns [].
        let goal = vec![0.1f32; 16];
        let results = index.search(&goal, 5).expect("search ok");
        assert!(
            results.is_empty(),
            "empty catalog yields empty result, no live fetch"
        );
    }

    #[tokio::test]
    async fn search_k_zero_returns_empty() {
        let entries = vec![entry("mock", "oc_alpha", "compress files", false)];
        let (index, _dir) = test_index(entries);
        index.sync().await.unwrap();
        let goal = vec![0.1f32; 16];
        assert!(index.search(&goal, 0).unwrap().is_empty());
    }

    // ---- Task 6.3: incremental / bounded-concurrent sync + offline fallback ----

    /// R9.5 incremental: re-syncing an unchanged entry SKIPS it — no re-embed,
    /// no re-write. Proven by the skip count AND by `fetched_at` staying byte-for
    /// -byte identical across the two syncs (a rewrite would refresh it).
    #[tokio::test]
    async fn incremental_sync_skips_unchanged_entry() {
        let entries = vec![entry(
            "mock",
            "oc_alpha",
            "compress and archive files",
            false,
        )];
        let (index, _providers, _dir) = test_index_with(vec![("mock", entries)]);

        let first = index.sync().await.expect("first sync");
        assert_eq!(first.upserted, 1, "entry embedded + written on first sync");
        assert_eq!(first.skipped, 0);

        let fetched_after_first: String = {
            let db = index.db.lock().unwrap();
            db.query_row(
                "SELECT fetched_at FROM market_catalog WHERE slug = 'oc_alpha'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };

        // Second sync with the SAME catalog: unchanged → skipped, not rewritten.
        let second = index.sync().await.expect("second sync");
        assert_eq!(second.skipped, 1, "unchanged entry skipped (no re-embed)");
        assert_eq!(second.upserted, 0, "nothing rewritten");

        let fetched_after_second: String = {
            let db = index.db.lock().unwrap();
            db.query_row(
                "SELECT fetched_at FROM market_catalog WHERE slug = 'oc_alpha'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(
            fetched_after_first, fetched_after_second,
            "skipped row is not rewritten (fetched_at unchanged)"
        );
    }

    /// R9.5 incremental: a CHANGED entry (bumped version) IS re-embedded and
    /// upserted on the next sync.
    #[tokio::test]
    async fn incremental_sync_reupserts_changed_entry() {
        let (index, providers, _dir) = test_index_with(vec![(
            "mock",
            vec![entry("mock", "oc_alpha", "compress files", false)],
        )]);
        let provider = providers[0].clone();

        index.sync().await.expect("first sync");

        // Bump the version → content fingerprint changes → must re-upsert.
        let mut changed = entry("mock", "oc_alpha", "compress files", false);
        changed.version = "2.0.0".into();
        provider.set_entries(vec![changed]);

        let report = index.sync().await.expect("second sync");
        assert_eq!(report.upserted, 1, "changed entry re-embedded + written");
        assert_eq!(report.skipped, 0);

        let stored_version: String = {
            let db = index.db.lock().unwrap();
            db.query_row(
                "SELECT version FROM market_catalog WHERE slug = 'oc_alpha'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(stored_version, "2.0.0", "new version persisted");
    }

    /// R13.3 offline fallback: an unreachable provider keeps its prior cached
    /// rows (stale, never dropped), is reported offline, and `search` flags its
    /// candidates `offline = true` while a reachable provider's stay `false`.
    /// A recovered provider stops being flagged on the next sync.
    #[tokio::test]
    async fn unreachable_provider_serves_stale_cache_and_flags_offline() {
        let (index, providers, _dir) = test_index_with(vec![
            (
                "p_off",
                vec![entry(
                    "p_off",
                    "oc_stale",
                    "compress and archive files",
                    false,
                )],
            ),
            (
                "p_on",
                vec![entry("p_on", "oc_fresh", "send an email message", false)],
            ),
        ]);
        let p_off = providers[0].clone();

        // First sync: both reachable → both rows cached, none offline.
        let first = index.sync().await.expect("first sync");
        assert_eq!(first.upserted, 2);
        assert!(first.offline_providers.is_empty());

        // p_off goes unreachable; p_on stays reachable.
        p_off.set_unreachable(true);
        let second = index.sync().await.expect("second sync must not abort");
        assert_eq!(
            second.offline_providers,
            vec!["p_off".to_string()],
            "unreachable provider reported offline"
        );

        // Stale rows retained: both entries still in the catalog.
        {
            let db = index.db.lock().unwrap();
            let n: i64 = db
                .query_row("SELECT COUNT(*) FROM market_catalog", [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 2, "unreachable provider's stale rows are NOT dropped");
        }

        // Search flags the stale provider's candidate offline; the reachable
        // provider's candidate is fresh.
        let embedder = MockEmbedder { dim: 16 };
        let goal = embedder.embed("compress and archive files").await.unwrap();
        let results = index.search(&goal, 10).expect("search ok");
        let stale = results
            .iter()
            .find(|c| c.slug == "oc_stale")
            .expect("stale present");
        let fresh = results
            .iter()
            .find(|c| c.slug == "oc_fresh")
            .expect("fresh present");
        assert!(stale.offline, "stale candidate flagged offline (R13.3)");
        assert!(!fresh.offline, "reachable provider's candidate is fresh");

        // Recovery: p_off reachable again → no longer flagged offline.
        p_off.set_unreachable(false);
        let third = index.sync().await.expect("third sync");
        assert!(
            third.offline_providers.is_empty(),
            "recovered provider not offline"
        );
        let results = index.search(&goal, 10).expect("search ok");
        assert!(
            results.iter().all(|c| !c.offline),
            "no candidate flagged offline after recovery"
        );
    }

    /// R9.5 bounded concurrency: multiple providers are synced under the bounded
    /// work queue in a single `sync`, and the aggregated catalog contains entries
    /// from every provider.
    #[tokio::test]
    async fn concurrent_sync_covers_all_providers() {
        let (index, _providers, _dir) = test_index_with(vec![
            (
                "p_a",
                vec![entry("p_a", "oc_a", "compress and archive files", false)],
            ),
            (
                "p_b",
                vec![entry("p_b", "oc_b", "send an email message", false)],
            ),
        ]);

        let report = index.sync().await.expect("sync ok");
        assert_eq!(report.upserted, 2, "entries from both providers upserted");
        assert!(report.offline_providers.is_empty());

        // Both providers' rows are present and searchable.
        let providers_in_db: Vec<String> = {
            let db = index.db.lock().unwrap();
            let mut stmt = db
                .prepare("SELECT DISTINCT provider_id FROM market_catalog ORDER BY provider_id")
                .unwrap();
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .map(|r| r.unwrap())
                .collect();
            rows
        };
        assert_eq!(providers_in_db, vec!["p_a".to_string(), "p_b".to_string()]);

        let embedder = MockEmbedder { dim: 16 };
        let goal = embedder.embed("compress and archive files").await.unwrap();
        let results = index.search(&goal, 10).expect("search ok");
        let seen: HashSet<String> = results.iter().map(|c| c.provider_id.clone()).collect();
        assert!(
            seen.contains("p_a") && seen.contains("p_b"),
            "both providers ranked"
        );
    }

    /// R13.3: when MULTIPLE providers are unreachable in one sync,
    /// `SyncReport.offline_providers` lists **exactly** those providers (sorted,
    /// deterministic) while a still-reachable provider is absent — and search
    /// flags each offline provider's candidate `offline = true` and the reachable
    /// one `offline = false`.
    #[tokio::test]
    async fn multiple_unreachable_providers_all_reported_and_flagged() {
        let (index, providers, _dir) = test_index_with(vec![
            (
                "p_x",
                vec![entry("p_x", "oc_x", "compress and archive files", false)],
            ),
            (
                "p_y",
                vec![entry("p_y", "oc_y", "send an email message", false)],
            ),
            (
                "p_z",
                vec![entry("p_z", "oc_z", "translate a document", false)],
            ),
        ]);

        // First sync: all reachable → all cached, none offline.
        let first = index.sync().await.expect("first sync");
        assert_eq!(first.upserted, 3);
        assert!(first.offline_providers.is_empty());

        // p_x and p_z go unreachable; p_y stays reachable.
        providers[0].set_unreachable(true); // p_x
        providers[2].set_unreachable(true); // p_z

        let second = index.sync().await.expect("second sync must not abort");
        assert_eq!(
            second.offline_providers,
            vec!["p_x".to_string(), "p_z".to_string()],
            "exactly the unreachable providers reported, sorted; reachable p_y absent"
        );

        // All three stale/fresh rows are retained (nothing dropped).
        {
            let db = index.db.lock().unwrap();
            let n: i64 = db
                .query_row("SELECT COUNT(*) FROM market_catalog", [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 3, "unreachable providers' stale rows are NOT dropped");
        }

        // Search flags both offline providers' candidates; p_y stays fresh.
        let embedder = MockEmbedder { dim: 16 };
        let goal = embedder.embed("compress and archive files").await.unwrap();
        let results = index.search(&goal, 10).expect("search ok");
        let offline_slugs: HashSet<String> = results
            .iter()
            .filter(|c| c.offline)
            .map(|c| c.slug.clone())
            .collect();
        assert_eq!(
            offline_slugs,
            HashSet::from(["oc_x".to_string(), "oc_z".to_string()]),
            "candidates from both unreachable providers flagged offline (R13.3)"
        );
        let fresh = results
            .iter()
            .find(|c| c.slug == "oc_y")
            .expect("oc_y present");
        assert!(!fresh.offline, "reachable provider's candidate stays fresh");
    }

    /// R13.3 honesty: a stale (offline) candidate is STILL returned — a stale hit
    /// beats no hit — and it **retains its full metadata** (version, trust_hint,
    /// quality, popularity, deprecated) from the last successful sync, so the
    /// caller can rank it and surface it truthfully as offline (never fabricated,
    /// never presented as fresh).
    #[tokio::test]
    async fn stale_offline_candidate_retains_full_metadata() {
        let mut cached = entry("p_off", "oc_stale", "compress and archive files", true);
        cached.version = "3.2.1".into();
        cached.declared_trust = "verified".into(); // clamped to Community by trust_hint
        cached.quality = Some(0.9);
        cached.popularity = Some(0.42);
        let (index, providers, _dir) = test_index_with(vec![("p_off", vec![cached])]);

        // First sync caches the row with all metadata.
        index.sync().await.expect("first sync");

        // Provider goes unreachable → its row is served stale.
        providers[0].set_unreachable(true);
        let report = index.sync().await.expect("second sync must not abort");
        assert_eq!(report.offline_providers, vec!["p_off".to_string()]);

        let embedder = MockEmbedder { dim: 16 };
        let goal = embedder.embed("compress and archive files").await.unwrap();
        let results = index.search(&goal, 10).expect("search ok");
        let c = results
            .iter()
            .find(|c| c.slug == "oc_stale")
            .expect("stale hit returned");

        // Stale hit is still returned AND flagged offline.
        assert!(c.offline, "stale candidate flagged offline (R13.3)");
        // Full metadata retained from the last successful sync.
        assert_eq!(c.version, "3.2.1", "version retained on stale row");
        assert_eq!(
            c.trust_hint,
            Some(TrustTier::Community),
            "trust_hint retained (declared 'verified' clamped to Community)"
        );
        assert_eq!(c.quality, Some(0.9), "quality retained on stale row");
        assert_eq!(c.popularity, Some(0.42), "popularity retained on stale row");
        assert!(c.deprecated, "deprecation flag retained on stale row");
    }

    #[test]
    fn cosine_similarity_edges() {
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]), 1.0);
        assert_eq!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]), 0.0);
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 2.0]), 0.0);
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }
}
