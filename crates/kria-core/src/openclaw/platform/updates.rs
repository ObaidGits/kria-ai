//! A8.6 Update Engine — detect and classify available updates.
//!
//! Detects new versions, security updates, breaking (major) versions, deprecated
//! skills and publisher revocations. Reuses `bundle::version` semver logic. The
//! actual apply path delegates to the single installer (A8.5) — no duplicate installer.

use super::publisher::{PublisherRegistry, VerificationStatus};
use super::repository::{RepositoryEntry, RepositoryManager};
use crate::openclaw::bundle::version;
use semver::Version;
use serde::{Deserialize, Serialize};

/// Classification of an available update for an installed skill (A8.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateKind {
    /// Minor/patch upgrade available.
    Upgrade,
    /// Major (breaking) version available — needs reinstall/approval.
    Breaking,
    /// Publisher revoked — skill should be disabled/removed.
    PublisherRevoked,
    /// Catalogue no longer lists the skill — deprecated/removed upstream.
    Deprecated,
}

/// A detected update for one installed skill.
#[derive(Debug, Clone)]
pub struct AvailableUpdate {
    pub slug: String,
    pub current_version: String,
    pub new_version: Option<String>,
    pub kind: UpdateKind,
    pub publisher_id: String,
}

/// Auto-update policy (A8.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutoUpdatePolicy {
    /// Never auto-update.
    Manual,
    /// Auto-apply non-breaking upgrades only.
    NonBreaking,
    /// Auto-apply everything including breaking (not recommended).
    All,
}

impl Default for AutoUpdatePolicy {
    fn default() -> Self {
        Self::NonBreaking
    }
}

/// The single update engine (A8.6). Detection is deterministic; applying delegates
/// to the installer.
#[derive(Clone)]
pub struct UpdateEngine {
    repos: RepositoryManager,
    publishers: PublisherRegistry,
    policy: AutoUpdatePolicy,
}

impl UpdateEngine {
    pub fn new(
        repos: RepositoryManager,
        publishers: PublisherRegistry,
        policy: AutoUpdatePolicy,
    ) -> Self {
        Self {
            repos,
            publishers,
            policy,
        }
    }

    pub fn policy(&self) -> AutoUpdatePolicy {
        self.policy
    }

    /// Detect updates for the given installed skills (slug, version).
    pub fn detect(&self, installed: &[(String, String)]) -> Vec<AvailableUpdate> {
        let mut updates = Vec::new();

        for (slug, cur_ver) in installed {
            // Publisher revocation check first (highest severity).
            let catalogue_entry = self.repos.find(slug);

            match &catalogue_entry {
                Some(entry) => {
                    // Revoked publisher?
                    if let Some(p) = self.publishers.get(&entry.publisher_id) {
                        if p.verification == VerificationStatus::Revoked {
                            updates.push(AvailableUpdate {
                                slug: slug.clone(),
                                current_version: cur_ver.clone(),
                                new_version: None,
                                kind: UpdateKind::PublisherRevoked,
                                publisher_id: entry.publisher_id.clone(),
                            });
                            continue;
                        }
                    }

                    // Version comparison.
                    if let (Ok(cur), Ok(new)) =
                        (Version::parse(cur_ver), Version::parse(&entry.version))
                    {
                        match version::relation(&new, Some(&cur)) {
                            version::VersionRelation::Upgrade => {
                                let kind = if version::is_breaking_change(&cur, &new) {
                                    UpdateKind::Breaking
                                } else {
                                    UpdateKind::Upgrade
                                };
                                updates.push(AvailableUpdate {
                                    slug: slug.clone(),
                                    current_version: cur_ver.clone(),
                                    new_version: Some(entry.version.clone()),
                                    kind,
                                    publisher_id: entry.publisher_id.clone(),
                                });
                            }
                            _ => {}
                        }
                    }
                }
                None => {
                    // No longer in catalogue → deprecated/removed upstream.
                    updates.push(AvailableUpdate {
                        slug: slug.clone(),
                        current_version: cur_ver.clone(),
                        new_version: None,
                        kind: UpdateKind::Deprecated,
                        publisher_id: String::new(),
                    });
                }
            }
        }

        updates
    }

    /// Filter detected updates to those eligible for auto-apply under the policy.
    pub fn auto_applicable<'a>(&self, updates: &'a [AvailableUpdate]) -> Vec<&'a AvailableUpdate> {
        updates
            .iter()
            .filter(|u| match self.policy {
                AutoUpdatePolicy::Manual => false,
                AutoUpdatePolicy::NonBreaking => matches!(u.kind, UpdateKind::Upgrade),
                AutoUpdatePolicy::All => {
                    matches!(u.kind, UpdateKind::Upgrade | UpdateKind::Breaking)
                }
            })
            .collect()
    }

    /// Convenience: entry to download for an update, if the catalogue has it.
    pub fn update_entry(&self, slug: &str) -> Option<RepositoryEntry> {
        self.repos.find(slug)
    }
}
