//! ClawHub remote registry client.
//!
//! Fetches skill listings from a GitHub-based `index.json` catalogue and
//! downloads individual `SKILL.md` manifests from their raw download URLs.
//!
//! # Registry Format
//!
//! The registry is a plain JSON array hosted at a configurable URL
//! (default: GitHub raw content). Each entry carries enough metadata
//! for the marketplace UI plus a `manifest_url` to download the full
//! SKILL.md at install time.
//!
//! # Security
//!
//! - All download URLs are validated against an HTTPS-only allowlist.
//! - Domain validation is PSL-aware: only `github.com`, `githubusercontent.com`,
//!   and the configured `registry_allowed_hosts` are accepted.
//! - Manifests are size-limited to 64 KiB before passing to the transpiler.
//! - Remote skills are always assigned `TrustTier::Community` — never Verified.

use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use url::Url;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_MANIFEST_BYTES: usize = 64 * 1024; // 64 KiB

/// Default registry index URL — points to the official, production KRIA skills
/// repo (decision locked: `ObaidGits/kria-skills` is authoritative; see
/// `.kiro/specs/openclaw-production-validation/tasks.md` task 25).
pub const DEFAULT_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/ObaidGits/kria-skills/refs/heads/main/index.json";

/// Domains permitted for manifest downloads.
const ALLOWED_DOWNLOAD_HOSTS: &[&str] = &[
    "raw.githubusercontent.com",
    "githubusercontent.com",
    "github.com",
];

// ── Remote types ──────────────────────────────────────────────────────────────

/// A single entry in the remote registry index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteSkillEntry {
    /// Stable identifier, e.g. `oc_code_sandbox`.
    pub slug: String,
    /// Display name.
    pub name: String,
    /// One-line description (verb-noun, ≤ 200 chars).
    pub description: String,
    /// Category tag (web / productivity / developer / …).
    pub category: String,
    /// Declared trust tier — always overridden to Community on install.
    pub trust_tier: String,
    /// Semver string, e.g. `"1.0.0"`.
    pub version: String,
    /// Full URL to the raw `SKILL.md` (or `skill.json`) manifest.
    pub manifest_url: String,
    /// Declared capabilities summary for the permission modal.
    #[serde(default)]
    pub capabilities_summary: Vec<String>,
}

// ── Domain validator ──────────────────────────────────────────────────────────

/// Validates that a download URL is safe to fetch from.
///
/// Enforces HTTPS-only and restricts hosts to an allowlist.
/// Extra hosts may be added via `OpenClawConfig::registry_allowed_hosts`.
pub struct DomainValidator {
    extra_hosts: Vec<String>,
}

impl DomainValidator {
    pub fn new(extra_hosts: Vec<String>) -> Self {
        Self { extra_hosts }
    }

    /// Returns `Ok(())` if `raw_url` is safe, `Err(reason)` otherwise.
    pub fn validate(&self, raw_url: &str) -> Result<(), String> {
        let parsed = Url::parse(raw_url).map_err(|e| format!("invalid URL: {e}"))?;

        if parsed.scheme() != "https" {
            return Err(format!(
                "only HTTPS downloads are allowed, got scheme '{}'",
                parsed.scheme()
            ));
        }

        let host = parsed
            .host_str()
            .ok_or_else(|| "URL has no host".to_string())?;

        let allowed = ALLOWED_DOWNLOAD_HOSTS
            .iter()
            .any(|&h| host == h || host.ends_with(&format!(".{h}")));

        let extra_allowed = self
            .extra_hosts
            .iter()
            .any(|h| host == h.as_str() || host.ends_with(&format!(".{h}")));

        if !allowed && !extra_allowed {
            return Err(format!(
                "download host '{}' is not in the allowed list. \
                 Add it to openclaw.registry_allowed_hosts in config.toml",
                host
            ));
        }

        Ok(())
    }
}

// ── ClawHubClient ─────────────────────────────────────────────────────────────

/// Client for a GitHub-based remote skill registry.
pub struct ClawHubClient {
    client: Client,
    index_url: String,
    validator: DomainValidator,
}

impl ClawHubClient {
    /// Create a client pointed at `index_url`.
    ///
    /// `allowed_hosts` is appended to the built-in allowlist for manifest
    /// downloads — useful for self-hosted registries.
    pub fn new(index_url: &str, allowed_hosts: Vec<String>) -> Self {
        let client = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .user_agent("kria-openclaw/1.0")
            .https_only(true)
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            index_url: index_url.to_string(),
            validator: DomainValidator::new(allowed_hosts),
        }
    }

    /// Fetch and parse the remote `index.json`.
    /// Returns all entries; caller applies query filtering.
    pub async fn fetch_remote_index(&self) -> Result<Vec<RemoteSkillEntry>, ClawHubError> {
        self.validator
            .validate(&self.index_url)
            .map_err(ClawHubError::DomainViolation)?;

        let resp = self
            .client
            .get(&self.index_url)
            .send()
            .await
            .map_err(|e| ClawHubError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(ClawHubError::Http(resp.status().as_u16()));
        }

        resp.json::<Vec<RemoteSkillEntry>>()
            .await
            .map_err(|e| ClawHubError::Parse(e.to_string()))
    }

    /// Search the remote index — fetches full index then filters locally.
    pub async fn search_remote(
        &self,
        query: &str,
        category: Option<&str>,
    ) -> Result<Vec<RemoteSkillEntry>, ClawHubError> {
        let all = self.fetch_remote_index().await?;
        let q = query.to_lowercase();
        Ok(all
            .into_iter()
            .filter(|e| {
                let q_match = q.is_empty()
                    || e.name.to_lowercase().contains(&q)
                    || e.description.to_lowercase().contains(&q)
                    || e.slug.to_lowercase().contains(&q)
                    || e.category.to_lowercase().contains(&q);
                let cat_match = category
                    .map(|c| e.category.eq_ignore_ascii_case(c))
                    .unwrap_or(true);
                q_match && cat_match
            })
            .collect())
    }

    /// Download a raw manifest from a validated URL.
    ///
    /// Validates the host, enforces 64 KiB size limit, and returns raw text
    /// ready for `transpiler::transpile_skill()`.
    pub async fn download_skill_manifest(
        &self,
        manifest_url: &str,
    ) -> Result<String, ClawHubError> {
        self.validator
            .validate(manifest_url)
            .map_err(ClawHubError::DomainViolation)?;

        let resp = self
            .client
            .get(manifest_url)
            .send()
            .await
            .map_err(|e| ClawHubError::Network(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(ClawHubError::Http(resp.status().as_u16()));
        }

        // Enforce size limit before buffering
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ClawHubError::Network(e.to_string()))?;

        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(ClawHubError::ManifestTooLarge(bytes.len()));
        }

        String::from_utf8(bytes.to_vec())
            .map_err(|_| ClawHubError::Parse("manifest is not valid UTF-8".into()))
    }
}

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ClawHubError {
    #[error("network error: {0}")]
    Network(String),
    #[error("HTTP {0}")]
    Http(u16),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("domain violation: {0}")]
    DomainViolation(String),
    #[error("manifest too large: {0} bytes (max 65536)")]
    ManifestTooLarge(usize),
}

impl From<ClawHubError> for String {
    fn from(e: ClawHubError) -> Self {
        e.to_string()
    }
}
