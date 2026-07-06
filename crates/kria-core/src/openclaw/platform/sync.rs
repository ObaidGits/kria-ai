//! A8.9 Synchronization — ONE sync engine.
//!
//! Syncs repository indexes, publisher metadata, trust metadata and the offline
//! cache. Supports delta sync (only changed slugs), resumable state and offline
//! mode (a sync with no reachable repo is a no-op that keeps the cache authoritative).

use super::metrics::PlatformMetrics;
use super::repository::{RepositoryEntry, RepositoryManager};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;

/// Result of a synchronization run (A8.9).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncReport {
    pub total_indexed: usize,
    pub added: Vec<String>,
    pub updated: Vec<String>,
    pub removed: Vec<String>,
    pub offline: bool,
    pub latency_ms: u64,
}

/// Persistable sync state for resume (A8.9).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncState {
    /// slug → last-synced content hash (delta detection).
    pub known: HashMap<String, String>,
    /// Last successful sync timestamp (rfc3339).
    pub last_sync: Option<String>,
}

/// The single synchronization engine (A8.9).
#[derive(Clone)]
pub struct SyncEngine {
    repos: RepositoryManager,
    metrics: PlatformMetrics,
}

impl SyncEngine {
    pub fn new(repos: RepositoryManager, metrics: PlatformMetrics) -> Self {
        Self { repos, metrics }
    }

    /// Perform a (delta) sync against the previous state. Refreshes the repository
    /// manager, computes added/updated/removed slugs by content hash, and returns an
    /// updated state + report. Offline-safe: if refresh yields nothing and repos are
    /// unreachable, reports `offline` and preserves prior state.
    pub async fn sync(&self, prev: &SyncState) -> (SyncState, SyncReport) {
        let start = Instant::now();

        // Refresh merged catalogue (handles failover + cache fallback internally).
        let indexed = self.repos.refresh().await.unwrap_or(0);
        let catalogue: Vec<RepositoryEntry> = self.repos.catalogue();

        let offline = indexed == 0 && prev.last_sync.is_some();

        let mut new_state = SyncState {
            known: HashMap::new(),
            last_sync: Some(chrono::Utc::now().to_rfc3339()),
        };
        let mut report = SyncReport {
            total_indexed: catalogue.len(),
            offline,
            ..Default::default()
        };

        let mut current: HashMap<String, String> = HashMap::new();
        for entry in &catalogue {
            current.insert(entry.slug.clone(), entry.content_hash.clone());
            match prev.known.get(&entry.slug) {
                None => report.added.push(entry.slug.clone()),
                Some(prev_hash) if prev_hash != &entry.content_hash => {
                    report.updated.push(entry.slug.clone())
                }
                _ => {}
            }
            new_state
                .known
                .insert(entry.slug.clone(), entry.content_hash.clone());
        }

        // Removed = present before, absent now.
        for slug in prev.known.keys() {
            if !current.contains_key(slug) {
                report.removed.push(slug.clone());
            }
        }

        // On offline (nothing fetched), keep prior known set authoritative.
        if offline {
            new_state.known = prev.known.clone();
            report.added.clear();
            report.updated.clear();
            report.removed.clear();
        }

        report.latency_ms = start.elapsed().as_millis() as u64;
        self.metrics.record_sync(report.latency_ms);
        (new_state, report)
    }
}
