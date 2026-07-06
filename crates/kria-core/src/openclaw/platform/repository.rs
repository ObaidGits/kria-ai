//! A8.1 ClawHub Repository — ONE repository layer.
//!
//! Local / remote / cache / mirror repositories behind a single `Repository` trait.
//! A `RepositoryManager` orders them by priority, fails over, refreshes, indexes and
//! merges into one catalogue. Offline-first: cache + local repos need no network (A8.10).

use super::metrics::PlatformMetrics;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// A single skill available from a repository (index entry).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepositoryEntry {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub version: String,
    pub publisher_id: String,
    /// Content hash of the bundle (for verification + dedup).
    pub content_hash: String,
    /// Location to fetch the bundle: URL (remote) or path (local/cache).
    pub location: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub signed: bool,
}

/// Kind of repository (A8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepositoryKind {
    Local,
    Remote,
    Cache,
    Mirror,
}

/// Repository metadata (A8.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryMeta {
    pub id: String,
    pub kind: RepositoryKind,
    /// Lower number = higher priority (checked first).
    pub priority: u32,
    /// Whether this repo is reachable/healthy.
    pub enabled: bool,
}

/// Health snapshot of a repository.
#[derive(Debug, Clone, Default)]
pub struct RepositoryHealth {
    pub reachable: bool,
    pub entry_count: usize,
    pub detail: String,
}

#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    #[error("io error: {0}")]
    Io(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("all repositories failed for '{0}'")]
    AllFailed(String),
}

/// The single repository interface (A8.1). Every backend implements this.
#[async_trait]
pub trait Repository: Send + Sync {
    fn meta(&self) -> RepositoryMeta;

    /// Fetch the full index of available skills.
    async fn fetch_index(&self) -> Result<Vec<RepositoryEntry>, RepositoryError>;

    /// Look up a single entry by slug (latest version).
    async fn get_entry(&self, slug: &str) -> Result<RepositoryEntry, RepositoryError> {
        let index = self.fetch_index().await?;
        index
            .into_iter()
            .filter(|e| e.slug == slug)
            .max_by(|a, b| a.version.cmp(&b.version))
            .ok_or_else(|| RepositoryError::NotFound(slug.to_string()))
    }

    /// Download a bundle's raw bytes to a destination path. Returns the written path.
    async fn download_bundle(
        &self,
        entry: &RepositoryEntry,
        dest_dir: &Path,
    ) -> Result<PathBuf, RepositoryError>;

    /// Health check.
    async fn health(&self) -> RepositoryHealth {
        match self.fetch_index().await {
            Ok(idx) => RepositoryHealth {
                reachable: true,
                entry_count: idx.len(),
                detail: "ok".into(),
            },
            Err(e) => RepositoryHealth {
                reachable: false,
                entry_count: 0,
                detail: e.to_string(),
            },
        }
    }
}

/// A filesystem-backed local (or cache/mirror) repository (A8.1 + A8.10).
///
/// Layout: `<root>/index.json` (Vec<RepositoryEntry>) + bundle files referenced by
/// `entry.location` (relative to root or absolute).
pub struct LocalRepository {
    meta: RepositoryMeta,
    root: PathBuf,
}

impl LocalRepository {
    pub fn new(
        id: impl Into<String>,
        kind: RepositoryKind,
        priority: u32,
        root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            meta: RepositoryMeta {
                id: id.into(),
                kind,
                priority,
                enabled: true,
            },
            root: root.into(),
        }
    }

    fn index_path(&self) -> PathBuf {
        self.root.join("index.json")
    }

    /// Write the index (used by cache persistence + tests / mirrors).
    pub fn write_index(&self, entries: &[RepositoryEntry]) -> Result<(), RepositoryError> {
        std::fs::create_dir_all(&self.root).map_err(|e| RepositoryError::Io(e.to_string()))?;
        let json = serde_json::to_string_pretty(entries)
            .map_err(|e| RepositoryError::Parse(e.to_string()))?;
        std::fs::write(self.index_path(), json).map_err(|e| RepositoryError::Io(e.to_string()))?;
        Ok(())
    }

    /// Synchronous index read (used by the publishing pipeline, which is sync).
    pub fn fetch_index_blocking(&self) -> Result<Vec<RepositoryEntry>, String> {
        let path = self.index_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let json = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        serde_json::from_str(&json).map_err(|e| e.to_string())
    }

    /// Repository root directory.
    pub fn root_dir(&self) -> &Path {
        &self.root
    }

    fn resolve_location(&self, location: &str) -> PathBuf {
        let p = Path::new(location);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.root.join(location)
        }
    }
}

#[async_trait]
impl Repository for LocalRepository {
    fn meta(&self) -> RepositoryMeta {
        self.meta.clone()
    }

    async fn fetch_index(&self) -> Result<Vec<RepositoryEntry>, RepositoryError> {
        let path = self.index_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let json =
            std::fs::read_to_string(&path).map_err(|e| RepositoryError::Io(e.to_string()))?;
        serde_json::from_str(&json).map_err(|e| RepositoryError::Parse(e.to_string()))
    }

    async fn download_bundle(
        &self,
        entry: &RepositoryEntry,
        dest_dir: &Path,
    ) -> Result<PathBuf, RepositoryError> {
        let src = self.resolve_location(&entry.location);
        if !src.exists() {
            return Err(RepositoryError::NotFound(entry.location.clone()));
        }
        std::fs::create_dir_all(dest_dir).map_err(|e| RepositoryError::Io(e.to_string()))?;
        let file_name = src
            .file_name()
            .ok_or_else(|| RepositoryError::Io("bad source path".into()))?;
        let dest = dest_dir.join(file_name);
        std::fs::copy(&src, &dest).map_err(|e| RepositoryError::Io(e.to_string()))?;
        Ok(dest)
    }
}

/// The single repository manager (A8.1). Orders repositories by priority, provides
/// failover, a merged/indexed catalogue, refresh and an offline cache.
#[derive(Clone)]
pub struct RepositoryManager {
    /// Ordered by priority (ascending). Highest priority (lowest number) first.
    repos: Arc<RwLock<Vec<Arc<dyn Repository>>>>,
    /// Optional offline cache repository (persisted merged index + bundles).
    cache: Arc<RwLock<Option<Arc<LocalRepository>>>>,
    /// In-memory merged index (slug → best entry), refreshed on demand.
    index: Arc<RwLock<HashMap<String, RepositoryEntry>>>,
    metrics: PlatformMetrics,
}

impl RepositoryManager {
    pub fn new(metrics: PlatformMetrics) -> Self {
        Self {
            repos: Arc::new(RwLock::new(Vec::new())),
            cache: Arc::new(RwLock::new(None)),
            index: Arc::new(RwLock::new(HashMap::new())),
            metrics,
        }
    }

    /// Register a repository. Repos are kept sorted by priority (ascending).
    pub fn add_repository(&self, repo: Arc<dyn Repository>) {
        let mut repos = self.repos.write().unwrap();
        repos.push(repo);
        repos.sort_by_key(|r| r.meta().priority);
    }

    /// Set the offline cache repository (A8.10).
    pub fn set_cache(&self, cache: Arc<LocalRepository>) {
        *self.cache.write().unwrap() = Some(cache);
    }

    /// Ordered repository metadata (for diagnostics / marketplace).
    pub fn repositories(&self) -> Vec<RepositoryMeta> {
        self.repos
            .read()
            .unwrap()
            .iter()
            .map(|r| r.meta())
            .collect()
    }

    /// Refresh: fetch every enabled repository's index, merge by priority, persist to
    /// the offline cache. Returns the number of unique skills indexed (A8.1 + A8.9).
    pub async fn refresh(&self) -> Result<usize, RepositoryError> {
        let start = std::time::Instant::now();
        let repos: Vec<Arc<dyn Repository>> = {
            let guard = self.repos.read().unwrap();
            guard.iter().filter(|r| r.meta().enabled).cloned().collect()
        };

        // Higher priority (lower number) processed first; earlier entries win ties.
        let mut merged: HashMap<String, RepositoryEntry> = HashMap::new();
        let mut any_ok = false;
        for repo in &repos {
            match repo.fetch_index().await {
                Ok(entries) => {
                    any_ok = true;
                    for e in entries {
                        merged
                            .entry(e.slug.clone())
                            .and_modify(|existing| {
                                // Prefer higher semver if slug already present.
                                if e.version > existing.version {
                                    *existing = e.clone();
                                }
                            })
                            .or_insert(e);
                    }
                }
                Err(_) => {
                    // Failover: skip unreachable repo, continue.
                    continue;
                }
            }
        }

        if !any_ok && !repos.is_empty() {
            // Everything failed → fall back to cache (offline mode).
            if let Some(cache) = self.cache.read().unwrap().clone() {
                if let Ok(entries) = cache.fetch_index().await {
                    for e in entries {
                        merged.insert(e.slug.clone(), e);
                    }
                }
            }
        }

        let count = merged.len();
        *self.index.write().unwrap() = merged.clone();

        // Persist to offline cache.
        if let Some(cache) = self.cache.read().unwrap().clone() {
            let entries: Vec<RepositoryEntry> = merged.into_values().collect();
            let _ = cache.write_index(&entries);
        }

        self.metrics
            .record_repo_latency(start.elapsed().as_millis() as u64);
        Ok(count)
    }

    /// The merged catalogue (in-memory index). Call `refresh` first to populate.
    pub fn catalogue(&self) -> Vec<RepositoryEntry> {
        self.index.read().unwrap().values().cloned().collect()
    }

    /// Look up a skill by slug in the merged index.
    pub fn find(&self, slug: &str) -> Option<RepositoryEntry> {
        self.index.read().unwrap().get(slug).cloned()
    }

    /// Download a skill's bundle with failover across repositories (A8.1).
    ///
    /// Tries each enabled repo in priority order; first success wins. Falls back to
    /// the offline cache if all live repos fail (A8.10). Records metrics.
    pub async fn download(
        &self,
        slug: &str,
        dest_dir: &Path,
    ) -> Result<(PathBuf, RepositoryEntry), RepositoryError> {
        let repos: Vec<Arc<dyn Repository>> = {
            let guard = self.repos.read().unwrap();
            guard.iter().filter(|r| r.meta().enabled).cloned().collect()
        };

        for repo in &repos {
            if let Ok(entry) = repo.get_entry(slug).await {
                match repo.download_bundle(&entry, dest_dir).await {
                    Ok(path) => {
                        self.metrics.inc_download();
                        // Repos other than cache count as a live hit.
                        if repo.meta().kind == RepositoryKind::Cache {
                            self.metrics.inc_cache_hit();
                        } else {
                            self.metrics.inc_cache_miss();
                        }
                        return Ok((path, entry));
                    }
                    Err(_) => continue, // failover
                }
            }
        }

        // Offline cache fallback.
        if let Some(cache) = self.cache.read().unwrap().clone() {
            if let Ok(entry) = cache.get_entry(slug).await {
                if let Ok(path) = cache.download_bundle(&entry, dest_dir).await {
                    self.metrics.inc_cache_hit();
                    return Ok((path, entry));
                }
            }
        }

        self.metrics.inc_failure();
        Err(RepositoryError::AllFailed(slug.to_string()))
    }

    /// Health of every registered repository.
    pub async fn health_all(&self) -> Vec<(String, RepositoryHealth)> {
        let repos: Vec<Arc<dyn Repository>> = self.repos.read().unwrap().clone();
        let mut out = Vec::new();
        for repo in repos {
            let h = repo.health().await;
            out.push((repo.meta().id, h));
        }
        out
    }
}

/// A remote repository backed by the existing `ClawHubClient` (A8.1).
///
/// Adapts the GitHub-based `index.json` remote registry into the unified `Repository`
/// interface. Downloads are validated + size-limited by the client. Remote entries are
/// mapped from `RemoteSkillEntry`; the bundle `location` is the manifest URL.
pub struct RemoteRepository {
    meta: RepositoryMeta,
    client: crate::openclaw::clawhub::ClawHubClient,
}

impl RemoteRepository {
    pub fn new(
        id: impl Into<String>,
        priority: u32,
        index_url: &str,
        allowed_hosts: Vec<String>,
    ) -> Self {
        Self {
            meta: RepositoryMeta {
                id: id.into(),
                kind: RepositoryKind::Remote,
                priority,
                enabled: true,
            },
            client: crate::openclaw::clawhub::ClawHubClient::new(index_url, allowed_hosts),
        }
    }
}

#[async_trait]
impl Repository for RemoteRepository {
    fn meta(&self) -> RepositoryMeta {
        self.meta.clone()
    }

    async fn fetch_index(&self) -> Result<Vec<RepositoryEntry>, RepositoryError> {
        let remote = self
            .client
            .fetch_remote_index()
            .await
            .map_err(|e| RepositoryError::Network(e.to_string()))?;
        Ok(remote
            .into_iter()
            .map(|r| RepositoryEntry {
                slug: r.slug,
                name: r.name,
                description: r.description,
                category: r.category,
                version: r.version,
                // Remote registry entries do not carry a publisher id in the legacy
                // index; attribute to a generic community publisher until signed bundles
                // (with manifest publisher keys) are downloaded.
                publisher_id: "community".to_string(),
                content_hash: String::new(),
                location: r.manifest_url,
                tags: r.capabilities_summary,
                signed: false,
            })
            .collect())
    }

    async fn download_bundle(
        &self,
        entry: &RepositoryEntry,
        dest_dir: &Path,
    ) -> Result<PathBuf, RepositoryError> {
        // Download the raw manifest (legacy SKILL.md) to dest as `<slug>.manifest`.
        let text = self
            .client
            .download_skill_manifest(&entry.location)
            .await
            .map_err(|e| RepositoryError::Network(e.to_string()))?;
        std::fs::create_dir_all(dest_dir).map_err(|e| RepositoryError::Io(e.to_string()))?;
        let dest = dest_dir.join(format!("{}.manifest", entry.slug));
        std::fs::write(&dest, text).map_err(|e| RepositoryError::Io(e.to_string()))?;
        Ok(dest)
    }
}
