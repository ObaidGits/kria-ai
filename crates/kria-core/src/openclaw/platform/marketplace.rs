//! A8.4 Marketplace — ONE marketplace surface over the repository catalogue.
//!
//! Browse / search / categories / tags / publishers / trending / featured / verified /
//! installed / updates / recommended. Everything is registry/catalogue-driven — no
//! hardcoded categories. Reads the merged `RepositoryManager` catalogue + publisher set.

use super::publisher::{PublisherRegistry, PublisherTrust};
use super::repository::{RepositoryEntry, RepositoryManager};
use std::collections::BTreeSet;

/// A marketplace listing = a catalogue entry enriched with derived flags.
#[derive(Debug, Clone)]
pub struct Listing {
    pub entry: RepositoryEntry,
    pub verified_publisher: bool,
    pub installed: bool,
    pub update_available: bool,
}

/// Query for marketplace browse/search (A8.4).
#[derive(Debug, Clone, Default)]
pub struct MarketQuery {
    pub text: Option<String>,
    pub category: Option<String>,
    pub tag: Option<String>,
    pub publisher_id: Option<String>,
    pub verified_only: bool,
}

/// The single marketplace (A8.4). Pure read-model over catalogue + publishers.
#[derive(Clone)]
pub struct Marketplace {
    repos: RepositoryManager,
    publishers: PublisherRegistry,
}

impl Marketplace {
    pub fn new(repos: RepositoryManager, publishers: PublisherRegistry) -> Self {
        Self { repos, publishers }
    }

    fn is_verified_publisher(&self, publisher_id: &str) -> bool {
        self.publishers
            .get(publisher_id)
            .map(|p| {
                matches!(
                    p.trust,
                    PublisherTrust::Verified | PublisherTrust::FirstParty
                )
            })
            .unwrap_or(false)
    }

    fn to_listing(&self, entry: RepositoryEntry, installed: &[(String, String)]) -> Listing {
        let verified_publisher = self.is_verified_publisher(&entry.publisher_id);
        let installed_ver = installed
            .iter()
            .find(|(slug, _)| slug == &entry.slug)
            .map(|(_, v)| v.clone());
        let installed_flag = installed_ver.is_some();
        let update_available = installed_ver.map(|v| entry.version > v).unwrap_or(false);
        Listing {
            entry,
            verified_publisher,
            installed: installed_flag,
            update_available,
        }
    }

    /// Browse/search the catalogue. `installed` is a list of (slug, version) currently
    /// installed (from the A5 registry) so listings show installed/update flags.
    pub fn search(&self, q: &MarketQuery, installed: &[(String, String)]) -> Vec<Listing> {
        let text = q.text.as_ref().map(|t| t.to_lowercase());
        self.repos
            .catalogue()
            .into_iter()
            .filter(|e| {
                let text_ok = text.as_ref().map_or(true, |t| {
                    e.name.to_lowercase().contains(t)
                        || e.description.to_lowercase().contains(t)
                        || e.slug.to_lowercase().contains(t)
                        || e.tags.iter().any(|tag| tag.to_lowercase().contains(t))
                });
                let cat_ok = q
                    .category
                    .as_ref()
                    .map_or(true, |c| e.category.eq_ignore_ascii_case(c));
                let tag_ok = q
                    .tag
                    .as_ref()
                    .map_or(true, |t| e.tags.iter().any(|x| x.eq_ignore_ascii_case(t)));
                let pub_ok = q
                    .publisher_id
                    .as_ref()
                    .map_or(true, |p| &e.publisher_id == p);
                text_ok && cat_ok && tag_ok && pub_ok
            })
            .map(|e| self.to_listing(e, installed))
            .filter(|l| !q.verified_only || l.verified_publisher)
            .collect()
    }

    /// All categories present in the catalogue (dynamic — never hardcoded).
    pub fn categories(&self) -> Vec<String> {
        let set: BTreeSet<String> = self
            .repos
            .catalogue()
            .into_iter()
            .map(|e| e.category)
            .collect();
        set.into_iter().collect()
    }

    /// All tags present in the catalogue.
    pub fn tags(&self) -> Vec<String> {
        let set: BTreeSet<String> = self
            .repos
            .catalogue()
            .into_iter()
            .flat_map(|e| e.tags)
            .collect();
        set.into_iter().collect()
    }

    /// All publisher ids appearing in the catalogue.
    pub fn publishers(&self) -> Vec<String> {
        let set: BTreeSet<String> = self
            .repos
            .catalogue()
            .into_iter()
            .map(|e| e.publisher_id)
            .collect();
        set.into_iter().collect()
    }

    /// Verified-publisher listings only (A8.4 "verified").
    pub fn verified(&self, installed: &[(String, String)]) -> Vec<Listing> {
        self.search(
            &MarketQuery {
                verified_only: true,
                ..Default::default()
            },
            installed,
        )
    }

    /// Listings that have an update available for installed skills (A8.4 "updates").
    pub fn updates(&self, installed: &[(String, String)]) -> Vec<Listing> {
        self.search(&MarketQuery::default(), installed)
            .into_iter()
            .filter(|l| l.update_available)
            .collect()
    }

    /// Installed listings (A8.4 "installed").
    pub fn installed(&self, installed: &[(String, String)]) -> Vec<Listing> {
        self.search(&MarketQuery::default(), installed)
            .into_iter()
            .filter(|l| l.installed)
            .collect()
    }

    /// Featured = verified publishers, ranked by publisher reputation (A8.4 "featured").
    pub fn featured(&self, installed: &[(String, String)], limit: usize) -> Vec<Listing> {
        let mut listings = self.verified(installed);
        listings.sort_by(|a, b| {
            let ra = self
                .publishers
                .get(&a.entry.publisher_id)
                .map(|p| p.reputation)
                .unwrap_or(0.0);
            let rb = self
                .publishers
                .get(&b.entry.publisher_id)
                .map(|p| p.reputation)
                .unwrap_or(0.0);
            rb.partial_cmp(&ra).unwrap_or(std::cmp::Ordering::Equal)
        });
        listings.truncate(limit);
        listings
    }
}
